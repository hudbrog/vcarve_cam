//! C1 face core: rectangle raster, depth layers, extensions, entry logic and
//! coverage checks (plan sections 11 and 19.1).
use cam_core::{
    checks::{CheckStatus, check_plan},
    geometry::Point,
    project::{
        CamJob, FaceArea, FaceMargins, FacePattern, FaceSettings, HeightRef, HeightReference,
        JobTool, MillingAssignment, Operation, OperationSettings, RectXY, SetupSettings,
        StockSetup, ToolCapabilities, ToolGeometry, WorkZero, WorkZeroXY, WorkZeroZ,
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
        schema_version: 4,
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
        },
        legacy_machine_profile: None,
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
