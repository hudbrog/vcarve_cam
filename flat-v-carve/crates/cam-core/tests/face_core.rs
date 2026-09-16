//! C1 face core: rectangle raster, depth layers, extensions, entry logic and
//! coverage checks (plan sections 11 and 19.1).
use cam_core::{
    checks::{CheckStatus, check_plan},
    geometry::Point,
    project::{
        CamJob, FaceArea, FaceEntry, FaceMargins, FacePattern, FaceSettings, HeightRef,
        HeightReference, JobTool, MillingAssignment, Operation, OperationSettings, RectXY,
        SetupSettings, StockSetup, ToolCapabilities, ToolGeometry, WorkZero, WorkZeroXY, WorkZeroZ,
    },
    sequence::{GenerationStatus, OperationPlan, PlanLimits, StageRole},
    toolpath::{Interpolation, MotionEffect, MotionPurpose, PlannedMotion},
};

fn face_tool() -> JobTool {
    JobTool {
        id: "t1".into(),
        name: "10mm endmill".into(),
        geometry: Some(ToolGeometry::Endmill(cam_core::project::EndmillGeometry {
            diameter_mm: 10.,
            cutting_length_mm: 20.,
        })),
        capabilities: ToolCapabilities {
            plunge_capable: Some(true),
            ramp_capable: None,
        },
    }
}

fn assignment() -> MillingAssignment {
    MillingAssignment {
        tool_id: "t1".into(),
        spindle_rpm: Some(12_000.),
        spindle_direction: Some(cam_core::project::SpindleDirection::Clockwise),
        cutting_feed_mm_min: Some(600.),
        plunge_feed_mm_min: Some(200.),
        max_stepdown_mm: Some(2.),
        stepover_mm: Some(6.),
    }
}

fn face_settings(
    rect: Option<RectXY>,
    stepdown: Option<f64>,
    stepover: Option<f64>,
) -> FaceSettings {
    FaceSettings {
        area: match rect {
            Some(rect) => FaceArea::Rectangle { rect },
            None => FaceArea::EntireStock,
        },
        margins: FaceMargins::default(),
        entry: Default::default(),
        entry_overrun_mm: Some(2.),
        exit_overrun_mm: Some(2.),
        top: HeightRef {
            reference: HeightReference::StockTop,
            offset_mm: 0.,
        },
        bottom: HeightRef {
            reference: HeightReference::StockTop,
            offset_mm: -1.,
        },
        stepdown_mm: stepdown,
        stepover_mm: stepover,
        pass_angle_deg: Some(0.),
        pattern: FacePattern::ZigZag,
        assignment: assignment(),
    }
}

fn face_job(
    rect: Option<RectXY>,
    stepdown: Option<f64>,
    stepover: Option<f64>,
    stock: Option<RectXY>,
) -> CamJob {
    CamJob {
        name: "Face only".into(),
        source: None,
        import: Default::default(),
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(8.),
                xy: stock.or(Some(RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: 100.,
                    length_mm: 60.,
                })),
            },
            work_zero: WorkZero {
                xy: WorkZeroXY::SetupOrigin,
                z: WorkZeroZ::StockTop,
            },
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: None,
        },
        tools: vec![face_tool()],
        operations: vec![Operation {
            id: "face-1".into(),
            name: "Face the stock".into(),
            enabled: true,
            settings: OperationSettings::Face(face_settings(rect, stepdown.or(Some(1.)), stepover)),
        }],
        tolerances: cam_core::job::PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
            arc_fit_tolerance_mm: None,
        },
    }
}

/// The face settings of the fixture job, for edits between plans.
fn face_settings_mut(job: &mut CamJob) -> &mut FaceSettings {
    match &mut job.operations[0].settings {
        OperationSettings::Face(settings) => settings,
        _ => panic!("face settings expected"),
    }
}

#[test]
fn face_only_job_without_svg_plans_completely() {
    let job = face_job(None, None, Some(6.), None);
    assert!(job.source.is_none());
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete
    );
    assert_eq!(plan.stages[0].role, StageRole::Face);
    assert_eq!(check_plan(&plan).unwrap().status, CheckStatus::Passed);
    // The face plane is published as a named output with its coverage.
    let output = &plan.operation_results[0].named_outputs[0];
    assert_eq!(output.kind, "face_plane");
    assert_eq!(output.z_mm, Some(-1.));
    assert!(output.covered.is_some());
    // The stock history of the executed plan shows the full faced floor.
    let history = plan.stock_history(None).unwrap();
    for point in [(5., 5.), (50., 30.), (95., 55.)] {
        assert_eq!(
            history
                .material_top_at(Point::new(point.0, point.1))
                .unwrap(),
            -1.,
            "point {:?} must be faced to the bottom",
            point
        );
    }
    assert_eq!(
        history
            .material_top_at(Point::new(150., 30.))
            .unwrap_err()
            .code,
        "STOCK_POINT_OUTSIDE"
    );
}

#[test]
fn awkward_width_and_stepover_still_cover_the_last_strip_and_corners() {
    // 100 mm width with a 10 mm cutter: scan positions 5, 10.6, ... must end
    // with an explicit last row at 95 (integer division cannot drop it).
    let rect = RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 100.,
        length_mm: 10.5,
    };
    let job = face_job(Some(rect), None, Some(5.6), None);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete
    );
    let history = plan.stock_history(None).unwrap();
    for point in [
        (0.05, 0.05),
        (99.95, 0.05),
        (0.05, 10.45),
        (99.95, 10.45),
        (97.5, 5.25),
    ] {
        assert_eq!(
            history
                .material_top_at(Point::new(point.0, point.1))
                .unwrap(),
            -1.,
            "point {:?} must be covered",
            point
        );
    }
}

#[test]
fn interior_face_plunges_and_outside_stock_face_enters_air() {
    // An interior rectangle: the entry footprint lies inside actual stock,
    // so the descent is a plunge with a feed and a milling effect.
    let interior = RectXY {
        min_x_mm: 20.,
        min_y_mm: 20.,
        width_mm: 60.,
        length_mm: 20.,
    };
    let job = face_job(Some(interior), None, Some(6.), None);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    let entry = plan
        .motions
        .iter()
        .find(|m| m.purpose == MotionPurpose::Entry)
        .expect("interior face has an entry plunge");
    assert_eq!(entry.interpolation, Interpolation::LinearFeed);
    assert_eq!(entry.effect, MotionEffect::MillingSweep);
    assert!(entry.feed_mm_min.is_some());

    // A rectangle extending past the physical stock: the first row starts
    // wholly outside actual stock, so its descent is not a plunge.
    let overhang = RectXY {
        min_x_mm: -20.,
        min_y_mm: -15.,
        width_mm: 140.,
        length_mm: 90.,
    };
    let job = face_job(Some(overhang), None, Some(6.), None);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete
    );
    let entry = plan
        .motions
        .iter()
        .find(|m| m.purpose == MotionPurpose::Entry)
        .expect("overhanging face still has an entry");
    assert_eq!(
        entry.interpolation,
        Interpolation::Rapid,
        "air entry descends without plunge feed"
    );
    assert_eq!(
        entry.effect,
        MotionEffect::None,
        "air entry removes nothing"
    );
    assert_eq!(entry.feed_mm_min, None);
    assert!(check_plan(&plan).unwrap().export_ready);
}

#[test]
fn depth_layers_end_exactly_at_the_bottom() {
    let rect = RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 30.,
        length_mm: 20.,
    };
    let mut job = face_job(Some(rect), Some(0.75), Some(6.), None);
    let OperationSettings::Face(settings) = &mut job.operations[0].settings else {
        panic!("face settings expected");
    };
    settings.bottom.offset_mm = -2.;
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    let cut_depths: Vec<f64> = plan
        .motions
        .iter()
        .filter(|m| m.effect == MotionEffect::MillingSweep && m.purpose == MotionPurpose::Rough)
        .map(|m| m.end.z)
        .collect();
    let mut distinct: Vec<f64> = cut_depths.clone();
    distinct.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    distinct.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(
        distinct,
        vec![-2., -1.5, -0.75],
        "layer floors end exactly at the bottom"
    );
    // Every layer repeats the same row count.
    let rows_per_layer = cut_depths
        .iter()
        .filter(|z| (**z - -0.75).abs() < 1e-12)
        .count();
    for layer in [-1.5, -2.] {
        assert_eq!(
            cut_depths
                .iter()
                .filter(|z| (**z - layer).abs() < 1e-12)
                .count(),
            rows_per_layer,
            "layer {layer} repeats the pattern"
        );
    }
    // The published plane sits at the final bottom.
    assert_eq!(plan.operation_results[0].named_outputs[0].z_mm, Some(-2.));
}

#[test]
fn one_way_retracts_between_rows_and_zigzag_links_at_depth() {
    let rect = RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 40.,
        length_mm: 20.,
    };
    let mut job = face_job(Some(rect), None, Some(6.), None);
    let OperationSettings::Face(settings) = &mut job.operations[0].settings else {
        panic!("face settings expected");
    };
    settings.pattern = FacePattern::OneWay;
    let one_way = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    let rows: Vec<&PlannedMotion> = one_way
        .motions
        .iter()
        .filter(|m| m.purpose == MotionPurpose::Rough)
        .collect();
    assert!(
        rows.iter().all(|m| m.end.x >= m.start.x),
        "one-way rows never reverse"
    );
    let retractions = one_way
        .motions
        .iter()
        .filter(|m| m.purpose == MotionPurpose::Clearance && m.end.z > m.start.z)
        .count();
    assert!(retractions > 0, "one-way pattern retracts between rows");

    let mut job = face_job(Some(rect), None, Some(6.), None);
    let OperationSettings::Face(settings) = &mut job.operations[0].settings else {
        panic!("face settings expected");
    };
    settings.pattern = FacePattern::ZigZag;
    let zigzag = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    let rows: Vec<&PlannedMotion> = zigzag
        .motions
        .iter()
        .filter(|m| m.purpose == MotionPurpose::Rough)
        .collect();
    assert!(
        rows.windows(2)
            .any(|pair| (pair[0].end.x > pair[0].start.x) != (pair[1].end.x > pair[1].start.x)),
        "zigzag rows alternate direction"
    );
    // Cross-row links are cutting moves with a feed inside the envelope.
    let links: Vec<&PlannedMotion> = zigzag
        .motions
        .iter()
        .filter(|m| m.purpose == MotionPurpose::Rough && (m.end.x - m.start.x).abs() < 1e-12)
        .collect();
    assert!(!links.is_empty(), "zigzag links rows");
    assert!(
        links
            .iter()
            .all(|m| m.feed_mm_min.is_some() && m.effect == MotionEffect::MillingSweep)
    );
}

#[test]
fn ninety_degree_passes_rotate_the_raster() {
    let rect = RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 20.,
        length_mm: 40.,
    };
    let mut job = face_job(Some(rect), None, Some(6.), None);
    let OperationSettings::Face(settings) = &mut job.operations[0].settings else {
        panic!("face settings expected");
    };
    settings.pass_angle_deg = Some(90.);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete
    );
    let history = plan.stock_history(None).unwrap();
    for point in [(5., 5.), (10., 35.), (19.9, 0.1)] {
        assert_eq!(
            history
                .material_top_at(Point::new(point.0, point.1))
                .unwrap(),
            -1.
        );
    }
}

#[test]
fn unsupported_angles_and_stepovers_report_located_diagnostics() {
    let rect = RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 40.,
        length_mm: 20.,
    };
    let mut angled = face_job(Some(rect), None, Some(6.), None);
    let OperationSettings::Face(settings) = &mut angled.operations[0].settings else {
        panic!("face settings expected");
    };
    settings.pass_angle_deg = Some(30.);
    let plan = OperationPlan::plan_job(&angled, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "FACE_ANGLE_UNSUPPORTED")
    );

    // A stepover wider than the cutter cannot cover; the raster is checked,
    // not trusted.
    let wide = face_job(Some(rect), None, Some(11.), None);
    let plan = OperationPlan::plan_job(&wide, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "FACE_STEPOVER_RANGE" || d.code == "FACE_COVERAGE_INCOMPLETE")
    );

    // Entire-stock facing without physical dimensions stays explicit.
    let mut no_xy = face_job(None, None, Some(6.), None);
    no_xy.setup.stock.xy = None;
    let plan = OperationPlan::plan_job(&no_xy, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "MISSING_MACHINING_SETTING" && d.message.contains("setup.stock.xy"))
    );
}

#[test]
fn margins_and_overruns_extend_coverage_and_travel_separately() {
    let rect = RectXY {
        min_x_mm: 10.,
        min_y_mm: 10.,
        width_mm: 30.,
        length_mm: 20.,
    };
    let mut job = face_job(Some(rect), None, Some(6.), None);
    let OperationSettings::Face(settings) = &mut job.operations[0].settings else {
        panic!("face settings expected");
    };
    settings.margins = FaceMargins {
        min_x_mm: Some(2.),
        max_x_mm: Some(0.),
        min_y_mm: Some(1.),
        max_y_mm: Some(3.),
    };
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    let covered = plan.operation_results[0].named_outputs[0].covered.unwrap();
    assert_eq!(covered.min_x_mm, 8.);
    assert_eq!(covered.width_mm, 32.);
    assert_eq!(covered.min_y_mm, 9.);
    assert_eq!(covered.length_mm, 24.);
    // Row endpoints include radius plus travel overrun beyond the coverage.
    let row = plan
        .motions
        .iter()
        .find(|m| m.purpose == MotionPurpose::Rough)
        .unwrap();
    assert!(
        row.start.x <= 8. - 5. - 2. + 1e-9,
        "rows overrun coverage by radius + travel"
    );
    assert!(row.end.x >= 8. + 32. + 5. + 2. - 1e-9);
}

/// The rows of one face plan, in execution order: motions that run along the
/// pass axis at a constant scan coordinate.
fn raster_rows_of(plan: &cam_core::sequence::OperationPlan) -> Vec<&PlannedMotion> {
    plan.motions
        .iter()
        .filter(|m| {
            m.purpose == MotionPurpose::Rough
                && (m.start.y - m.end.y).abs() < 1e-12
                && (m.start.x - m.end.x).abs() > 1e-12
        })
        .collect()
}

#[test]
fn the_entry_travel_belongs_to_the_pass_that_descends() {
    // Two passes of one zig-zag layer, the second running the other way. Only
    // the pass that descends enters at the chosen end with the entry travel;
    // the pass that continues the layer begins where the previous one ended,
    // which is the far end with the exit travel. Applying the entry overrun to
    // whatever end a pass happens to start from is what used to leave every
    // second pass entering from a side nobody had cleared.
    let rect = RectXY {
        min_x_mm: 10.,
        min_y_mm: 10.,
        width_mm: 30.,
        length_mm: 20.,
    };
    let mut job = face_job(Some(rect), None, Some(6.), None);
    let OperationSettings::Face(settings) = &mut job.operations[0].settings else {
        panic!("face settings expected");
    };
    settings.entry_overrun_mm = Some(30.);
    settings.exit_overrun_mm = Some(0.);
    settings.pattern = FacePattern::ZigZag;
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete
    );
    // Rows run along X at a constant Y; links between them run along Y.
    let rows = raster_rows_of(&plan);
    assert!(rows.len() >= 2, "the fixture has several rows: {rows:?}");
    let radius = 5.;
    let low = rect.min_x_mm - radius;
    let high = rect.min_x_mm + rect.width_mm + radius;
    for (index, row) in rows.iter().enumerate() {
        let (start, end) = if index == 0 {
            (low - 30., high)
        } else if index % 2 == 1 {
            (high, low)
        } else {
            (low, high)
        };
        assert!(
            (row.start.x - start).abs() < 1e-9,
            "row {index} starts at {} instead of {start}",
            row.start.x
        );
        assert!(
            (row.end.x - end).abs() < 1e-9,
            "row {index} ends at {} instead of {end}",
            row.end.x
        );
    }
    assert!(check_plan(&plan).unwrap().export_ready);
}

#[test]
fn every_layer_enters_at_the_chosen_end() {
    // Two depth layers, one entry: both layers' first pass starts at the same
    // end with the entry travel, and the travel between them is a real motion.
    let rect = RectXY {
        min_x_mm: 10.,
        min_y_mm: 10.,
        width_mm: 30.,
        length_mm: 20.,
    };
    let mut job = face_job(Some(rect), None, Some(6.), None);
    let OperationSettings::Face(settings) = &mut job.operations[0].settings else {
        panic!("face settings expected");
    };
    settings.entry_overrun_mm = Some(30.);
    settings.exit_overrun_mm = Some(0.);
    settings.pattern = FacePattern::ZigZag;
    settings.bottom.offset_mm = -2.;
    settings.stepdown_mm = Some(1.);
    settings.entry = FaceEntry::Min;
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        plan.generation_diagnostics
    );
    let rows = raster_rows_of(&plan);
    let radius = 5.;
    let low = rect.min_x_mm - radius;
    // The first pass of each layer enters at the chosen end; `Min` keeps the
    // anchor across layers instead of flipping it. Rows arrive in execution
    // order, so a new cut depth starts a new layer.
    let mut layers: Vec<(f64, f64)> = vec![];
    for row in &rows {
        match layers.last() {
            Some((z, _)) if (*z - row.end.z).abs() < 1e-9 => {}
            _ => layers.push((row.end.z, row.start.x)),
        }
    }
    let first_of_layer: Vec<f64> = layers.iter().map(|(_, start)| *start).collect();
    assert!(first_of_layer.len() >= 2, "{first_of_layer:?}");
    for (layer, start) in first_of_layer.iter().enumerate() {
        assert!(
            (start - (low - 30.)).abs() < 1e-9,
            "layer {layer} enters at {start} instead of the chosen end {}",
            low - 30.
        );
    }
    assert!(check_plan(&plan).unwrap().export_ready);
}

#[test]
fn a_cutter_that_cannot_plunge_is_never_sent_into_the_material() {
    // An interior rectangle: every pass entry stands on real stock, so the
    // descent is a plunge and the tool has to be marked able to make it.
    let interior = RectXY {
        min_x_mm: 20.,
        min_y_mm: 20.,
        width_mm: 60.,
        length_mm: 20.,
    };
    let mut job = face_job(Some(interior), None, Some(6.), None);
    job.tools[0].capabilities.plunge_capable = Some(false);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    let issue = plan
        .generation_diagnostics
        .iter()
        .find(|d| d.code == "FACE_ENTRY_UNSAFE")
        .unwrap_or_else(|| panic!("{:?}", plan.generation_diagnostics));
    // The refusal names the end, the position the cutter clears the stock
    // from, and the travel that reaches it. Twenty millimetres is the exact
    // distance from the coverage edge (X = 15) to the clearing position
    // (X = -5): the cutter's centre has to leave the stock by its radius.
    assert!(
        issue.message.contains("X minimum edge")
            && issue.message.contains("X = -5.000")
            && issue.message.contains("20.000 mm of entry travel"),
        "the refusal must name the end and the travel that clears the stock: {}",
        issue.message
    );

    // One reserve below the reported minimum still refuses; the reported
    // minimum itself puts every descent in the air, and the same tool plans
    // completely.
    face_settings_mut(&mut job).entry_overrun_mm = Some(19.9);
    let below = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert_eq!(
        below.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    face_settings_mut(&mut job).entry_overrun_mm = Some(20.);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        plan.generation_diagnostics
    );
    assert!(check_plan(&plan).unwrap().export_ready);
}

#[test]
fn an_undeclared_plunge_capability_is_reported_before_the_plunge() {
    let interior = RectXY {
        min_x_mm: 20.,
        min_y_mm: 20.,
        width_mm: 60.,
        length_mm: 20.,
    };
    let mut job = face_job(Some(interior), None, Some(6.), None);
    job.tools[0].capabilities.plunge_capable = None;
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(
        plan.generation_diagnostics.iter().any(|d| {
            d.code == "MISSING_MACHINING_SETTING" && d.message.contains("plunge_capable")
        }),
        "{:?}",
        plan.generation_diagnostics
    );
}

#[test]
fn a_whole_stock_face_at_zero_overrun_still_plans_for_a_tool_that_cannot_plunge() {
    // The W1 acceptance line: with no entry travel the pass starts with the
    // cutter tangent to the stock edge, which has zero overlap with the
    // material, so the descent there is a side entry and needs no capability.
    let mut job = face_job(None, None, Some(6.), None);
    job.tools[0].capabilities.plunge_capable = Some(false);
    let settings = face_settings_mut(&mut job);
    settings.entry = FaceEntry::Min;
    settings.entry_overrun_mm = Some(0.);
    settings.exit_overrun_mm = Some(0.);
    settings.bottom.offset_mm = -2.;
    settings.stepdown_mm = Some(1.);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        plan.generation_diagnostics
    );
    assert!(check_plan(&plan).unwrap().export_ready);
    // Every descent is at the tangent entry: the coverage's X minimum minus
    // the cutter radius, once per depth layer, all on the same end.
    let mut descents = 0;
    for index in 0..plan.motions.len() {
        let motion = &plan.motions[index];
        let previous = if index == 0 {
            motion.start.z
        } else {
            plan.motions[index - 1].end.z
        };
        if motion.end.z >= previous - 1e-9 {
            continue;
        }
        descents += 1;
        assert!(
            (motion.end.x - -5.).abs() < 1e-9,
            "a descent at X = {} instead of the tangent entry",
            motion.end.x
        );
    }
    assert_eq!(descents, 2, "one entry per depth layer");
}

#[test]
fn the_basic_checks_refuse_an_entry_into_material_no_tool_may_make() {
    let interior = RectXY {
        min_x_mm: 20.,
        min_y_mm: 20.,
        width_mm: 60.,
        length_mm: 20.,
    };
    let job = face_job(Some(interior), None, Some(6.), None);
    // The planner admits this job: the tool is marked able to plunge.
    let mut plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert!(check_plan(&plan).unwrap().export_ready);

    // A rapid straight down through material is never authorized, whatever the
    // tool claims — the check must catch a plan that says otherwise.
    let entry = plan
        .motions
        .iter()
        .find(|m| m.purpose == MotionPurpose::Entry)
        .expect("the interior face has an entry");
    let index = entry.id;
    plan.motions[index].interpolation = Interpolation::Rapid;
    plan.motions[index].feed_mm_min = None;
    let report = check_plan(&plan).unwrap();
    assert!(!report.export_ready);
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "PLAN_ENTRY_UNVERIFIED"),
        "{:?}",
        report.findings
    );

    // The same descent, as a feed, is refused for a tool marked unable to
    // plunge: the check reads the plan's own tool snapshot, not the planner.
    let mut plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    plan.job_snapshot.tools[0].capabilities.plunge_capable = Some(false);
    let report = check_plan(&plan).unwrap();
    assert!(!report.export_ready);
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "PLAN_ENTRY_UNVERIFIED"),
        "{:?}",
        report.findings
    );
}

/// Every motion of a plan must start where the previous one ended. The post
/// emits one block per motion endpoint, so a transition the plan leaves out is
/// executed as a straight move from wherever the tool really is — the
/// 2026-09-13 field report's facing crash (finding 1.1).
#[test]
fn every_motion_starts_where_the_previous_one_ended() {
    for angle in [0., 90.] {
        for pattern in [FacePattern::ZigZag, FacePattern::OneWay] {
            for entry in [
                FaceEntry::Min,
                FaceEntry::Max,
                FaceEntry::Alternate,
                FaceEntry::At {
                    coordinate_mm: -20.,
                },
            ] {
                let mut job = face_job(None, None, Some(6.), None);
                let settings = face_settings_mut(&mut job);
                settings.pass_angle_deg = Some(angle);
                settings.pattern = pattern;
                settings.entry = entry;
                settings.entry_overrun_mm = Some(4.);
                settings.exit_overrun_mm = Some(2.);
                settings.bottom.offset_mm = -2.;
                settings.stepdown_mm = Some(1.);
                let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
                assert_eq!(
                    plan.operation_results[0].generation_status,
                    GenerationStatus::Complete,
                    "{pattern:?} at {angle} degrees with {entry:?}: {:?}",
                    plan.generation_diagnostics
                );
                assert!(plan.motions.len() > 8, "the fixture has a raster");
                for index in 1..plan.motions.len() {
                    let previous = plan.motions[index - 1].end;
                    let start = plan.motions[index].start;
                    let gap = (start.x - previous.x)
                        .abs()
                        .max((start.y - previous.y).abs())
                        .max((start.z - previous.z).abs());
                    assert!(
                        gap < 1e-9,
                        "{pattern:?} at {angle} degrees with {entry:?}: motion {} starts {gap:.3} mm from where motion {} ends",
                        plan.motions[index].id,
                        plan.motions[index - 1].id
                    );
                }
                assert!(check_plan(&plan).unwrap().export_ready);
            }
        }
    }
}

#[test]
fn the_basic_checks_refuse_a_chain_that_skips_a_position() {
    let mut job = face_job(None, None, Some(6.), None);
    let settings = face_settings_mut(&mut job);
    settings.bottom.offset_mm = -2.;
    settings.stepdown_mm = Some(1.);
    let mut plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert!(check_plan(&plan).unwrap().export_ready);
    // The pre-fix shape: the retract ends at the clearance plane and the next
    // motion claims to start somewhere else, exactly as the planner used to
    // hand the next depth layer its own entry point.
    // The second depth layer's entry: the first motion of a stage is bridged by
    // the post, so only a later transition can hide a gap.
    let entry = plan
        .motions
        .iter()
        .position(|m| m.purpose == MotionPurpose::Entry && m.id > 0)
        .expect("the two-layer face plan has a second entry");
    let moved = plan.motions[entry].start;
    plan.motions[entry].start = cam_core::motion::Position {
        x: moved.x + 12.,
        ..moved
    };
    let report = check_plan(&plan).unwrap();
    assert!(!report.export_ready);
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "PLAN_MOTION_DISCONTINUITY" && f.message.contains("12.000")),
        "{:?}",
        report.findings
    );
}

#[test]
fn every_descent_happens_at_the_chosen_entry() {
    // Whole stock (100 x 60) with a 10 mm cutter: the pass spans X -5 … 105.
    for (entry, expected) in [
        (FaceEntry::Min, -5. - 4.),
        (FaceEntry::Max, 105. + 4.),
        (
            FaceEntry::At {
                coordinate_mm: -25.,
            },
            -25.,
        ),
    ] {
        for pattern in [FacePattern::ZigZag, FacePattern::OneWay] {
            let mut job = face_job(None, None, Some(6.), None);
            let settings = face_settings_mut(&mut job);
            settings.pattern = pattern;
            settings.entry = entry;
            settings.entry_overrun_mm = Some(4.);
            settings.exit_overrun_mm = Some(2.);
            settings.bottom.offset_mm = -2.;
            settings.stepdown_mm = Some(1.);
            let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
            assert_eq!(
                plan.operation_results[0].generation_status,
                GenerationStatus::Complete,
                "{entry:?}: {:?}",
                plan.generation_diagnostics
            );
            let mut descents = 0;
            for index in 0..plan.motions.len() {
                let motion = &plan.motions[index];
                // The stage's first motion descends from the bridge position.
                let previous = if index == 0 {
                    motion.start.z
                } else {
                    plan.motions[index - 1].end.z
                };
                if motion.end.z >= previous - 1e-9 {
                    continue;
                }
                descents += 1;
                assert!(
                    (motion.end.x - expected).abs() < 1e-9,
                    "{pattern:?} with {entry:?}: a descent at X = {} instead of {expected}",
                    motion.end.x
                );
            }
            // One entry per depth layer when the passes link at depth; every
            // pass is its own plunge when they do not.
            let layers = 2;
            if pattern == FacePattern::ZigZag {
                assert_eq!(descents, layers, "{entry:?}: one entry per layer");
            } else {
                assert!(descents > layers, "{entry:?}: every pass descends");
            }
        }
    }
}

#[test]
fn an_explicit_entry_position_states_the_travel_it_implies() {
    // The coordinate and the travel are the same fact: `X = -25` on a pass
    // that spans X -5 … 105 is 20 mm of entry travel.
    let mut travelled = face_job(None, None, Some(6.), None);
    let mut positioned = face_job(None, None, Some(6.), None);
    for (job, entry, travel) in [
        (&mut travelled, FaceEntry::Min, 20.),
        (
            &mut positioned,
            FaceEntry::At {
                coordinate_mm: -25.,
            },
            0.,
        ),
    ] {
        let settings = face_settings_mut(job);
        settings.entry = entry;
        settings.entry_overrun_mm = Some(travel);
        settings.exit_overrun_mm = Some(2.);
        settings.bottom.offset_mm = -2.;
        settings.stepdown_mm = Some(1.);
    }
    let travelled = OperationPlan::plan_job(&travelled, &PlanLimits::default()).unwrap();
    let positioned = OperationPlan::plan_job(&positioned, &PlanLimits::default()).unwrap();
    assert_eq!(
        travelled.operation_results[0].generation_status,
        GenerationStatus::Complete
    );
    assert_eq!(
        positioned.operation_results[0].generation_status,
        GenerationStatus::Complete
    );
    let ends = |plan: &cam_core::sequence::OperationPlan| -> Vec<[f64; 4]> {
        plan.motions
            .iter()
            .map(|m| [m.start.x, m.start.y, m.end.x, m.end.y])
            .collect()
    };
    assert_eq!(ends(&travelled), ends(&positioned));
}

#[test]
fn an_entry_inside_the_pass_span_is_refused_with_the_allowed_positions() {
    let mut job = face_job(None, None, Some(6.), None);
    face_settings_mut(&mut job).entry = FaceEntry::At { coordinate_mm: 25. };
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    let issue = plan
        .generation_diagnostics
        .iter()
        .find(|d| d.code == "FACE_ENTRY_INSIDE_COVERAGE")
        .unwrap_or_else(|| panic!("{:?}", plan.generation_diagnostics));
    assert!(
        issue.message.contains("X = 25.000")
            && issue.message.contains("X = -5.000")
            && issue.message.contains("X = 105.000"),
        "the refusal names the position and both allowed ones: {}",
        issue.message
    );
}

#[test]
fn one_entry_leaves_the_other_end_to_the_exit_travel() {
    // The allowed envelope follows the entry: with one entry the end it names
    // reserves the entry travel and the other end reserves only the exit
    // travel, which is what keeps the generated raster inside its own bounds.
    let rect = RectXY {
        min_x_mm: 10.,
        min_y_mm: 10.,
        width_mm: 30.,
        length_mm: 20.,
    };
    let mut job = face_job(Some(rect), None, Some(6.), None);
    let settings = face_settings_mut(&mut job);
    settings.entry = FaceEntry::Min;
    settings.entry_overrun_mm = Some(30.);
    settings.exit_overrun_mm = Some(0.);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    let rows = raster_rows_of(&plan);
    assert!(rows.len() >= 2, "{rows:?}");
    // The pass spans X 5 … 45 with no exit travel; the entry end is 30 mm out.
    assert!(
        rows.iter().any(|row| (row.start.x - -25.).abs() < 1e-9),
        "the chosen end carries the entry travel: {rows:?}"
    );
    assert!(
        rows.iter()
            .all(|row| row.end.x <= 45. + 1e-9 && row.start.x >= -25. - 1e-9),
        "nothing travels past the travel the settings authorize: {rows:?}"
    );
}
