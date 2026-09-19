//! D3 drill planning: marker points resolve through the catalogue into a
//! per-hole motion sequence — clearance travel, rapid to the R plane, feeding
//! plunges with optional pecking and bottom dwell — all expanded to explicit
//! linear moves. The checks gate authorizes drill plunges, the stock history
//! receives hole disks, the point-selection command binds references, and the
//! export readback verifies the expanded program including `G4 P` dwells.
use cam_core::{
    checks::{CheckStatus, check_plan},
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    model::DrillSpec,
    post::sequence::{PreparedExecution, SequenceProfile, SequenceToolMapping},
    project::{
        CamJob, DrillHoleOrder, DrillPeckMode, DrillPeckSettings, DrillSettings, HeightRef,
        HeightReference, MillingAssignment, Operation, OperationSettings, RectXY, SetupSettings,
        SpindleDirection, StockSetup, ToolCapabilities, ToolGeometry, WorkZero, v5,
    },
    sequence::{OperationPlan, PlanLimits, StageRole, TrustedPlan},
    svg::{ImportOptions, Placement},
};

const MARKERS: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><circle id="a" cx="10" cy="15" r="2.5" fill="#000"/><circle id="b" cx="30" cy="10" r="4" fill="#000"/></svg>"##;

fn drill_assignment() -> MillingAssignment {
    MillingAssignment {
        tool_id: "drill".into(),
        spindle_rpm: Some(6000.),
        spindle_direction: Some(SpindleDirection::Clockwise),
        cutting_feed_mm_min: None,
        plunge_feed_mm_min: Some(120.),
        max_stepdown_mm: None,
        stepover_mm: None,
    }
}

fn drill_settings(points: &[&str]) -> DrillSettings {
    DrillSettings {
        points: points.iter().map(|p| (*p).to_string()).collect(),
        assignment: drill_assignment(),
        top: HeightRef {
            reference: HeightReference::StockTop,
            offset_mm: 0.,
        },
        bottom: HeightRef {
            reference: HeightReference::StockBottom,
            offset_mm: 0.,
        },
        retract_height: HeightRef {
            reference: HeightReference::StockTop,
            offset_mm: 2.,
        },
        depth_reference: Default::default(),
        breakthrough_extra_mm: None,
        peck: None,
        dwell_at_bottom_s: None,
        hole_order: DrillHoleOrder::XThenY,
        warn_drill_exceeds_marker: false,
    }
}

fn job(settings: DrillSettings) -> CamJob {
    CamJob {
        name: "drill".into(),
        source: Some(SourceSnapshot {
            filename: "markers.svg".into(),
            svg: MARKERS.into(),
        }),
        import: ImportOptions {
            placement: Placement::default(),
            ..Default::default()
        },
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(8.),
                xy: Some(RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: 40.,
                    length_mm: 30.,
                }),
            },
            work_zero: WorkZero::default(),
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: None,
        },
        tools: vec![cam_core::project::JobTool {
            id: "drill".into(),
            name: "5mm drill".into(),
            geometry: Some(ToolGeometry::Drill(DrillSpec {
                diameter_mm: 5.,
                tip_angle_deg: 118.,
                cutting_length_mm: 30.,
            })),
            capabilities: ToolCapabilities {
                plunge_capable: Some(true),
                ramp_capable: None,
            },
        }],
        operations: vec![Operation {
            id: "holes".into(),
            name: "Holes".into(),
            enabled: true,
            settings: OperationSettings::Drill(settings),
        }],
        tolerances: PlanningTolerances::default(),
    }
}

#[test]
fn single_plunge_drills_every_point_in_x_then_y_order() {
    let plan = OperationPlan::plan_job(
        &job(drill_settings(&["b-point", "a-point"])),
        &PlanLimits::default(),
    )
    .unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        cam_core::sequence::GenerationStatus::Complete
    );
    assert_eq!(plan.stages.len(), 1);
    assert_eq!(plan.stages[0].role, StageRole::Drill);
    assert_eq!(plan.stages[0].tool_id, "drill");
    let motions = &plan.motions;
    // Two holes × (approach, plunge, retract) + one climb + one travel.
    assert_eq!(motions.len(), 8);
    let summary: Vec<(f64, f64, f64, f64)> = motions
        .iter()
        .map(|m| (m.end.x, m.end.y, m.end.z, m.feed_mm_min.unwrap_or(0.)))
        .collect();
    assert_eq!(
        summary,
        vec![
            // X-then-Y order visits a (10, 15) despite b being selected first.
            (10., 15., 2., 0.),    // rapid down to the R plane
            (10., 15., -8., 120.), // feed through the full 8 mm stock
            (10., 15., 2., 0.),    // rapid back to R
            (10., 15., 5., 0.),    // climb to clearance
            (30., 20., 5., 0.),    // travel at clearance
            (30., 20., 2., 0.),    // rapid down to R
            (30., 20., -8., 120.), // feed
            (30., 20., 2., 0.),    // rapid back to R
        ]
    );
    // The safety gate authorizes the plunges: a plunge-capable drill descends
    // into stock, feeds are set, and rapids never carry feeds.
    let checks = check_plan(&plan).unwrap();
    assert_eq!(
        checks.status,
        CheckStatus::Passed,
        "findings: {:?}",
        checks.findings
    );
}

#[test]
fn as_selected_order_preserves_the_document_selection() {
    let mut settings = drill_settings(&["b-point", "a-point"]);
    settings.hole_order = DrillHoleOrder::AsSelected;
    let plan = OperationPlan::plan_job(&job(settings), &PlanLimits::default()).unwrap();
    let first_plunge = plan
        .motions
        .iter()
        .find(|m| m.feed_mm_min.is_some())
        .unwrap();
    assert_eq!((first_plunge.end.x, first_plunge.end.y), (30., 20.));
}

#[test]
fn full_retract_pecks_clear_chips_to_the_r_plane() {
    let mut settings = drill_settings(&["a-point"]);
    settings.peck = Some(DrillPeckSettings {
        mode: DrillPeckMode::FullRetract,
        depth_mm: 2.,
        reduction_mm: 0.,
        min_depth_mm: 2.,
        retract_mm: None,
    });
    let plan = OperationPlan::plan_job(&job(settings), &PlanLimits::default()).unwrap();
    let zs: Vec<f64> = plan.motions.iter().map(|m| m.end.z).collect();
    assert_eq!(
        zs,
        vec![
            2.,     // rapid to R
            -2.,    // feed peck 1
            2.,     // full retract to R
            -1.746, // rapid back to 0.254 above the reached depth
            -4.,    // feed peck 2
            2.,     // full retract
            -3.746, // rapid re-entry
            -6.,    // feed peck 3
            2.,     // full retract
            -5.746, // rapid re-entry
            -8.,    // feed peck 4 reaches the bottom
            2.,     // final retract
        ]
    );
    assert_eq!(check_plan(&plan).unwrap().status, CheckStatus::Passed);
}

#[test]
fn chip_break_pecks_stay_inside_the_hole() {
    let mut settings = drill_settings(&["a-point"]);
    settings.peck = Some(DrillPeckSettings {
        mode: DrillPeckMode::ChipBreak,
        depth_mm: 2.,
        reduction_mm: 0.,
        min_depth_mm: 2.,
        retract_mm: Some(0.5),
    });
    let plan = OperationPlan::plan_job(&job(settings), &PlanLimits::default()).unwrap();
    let zs: Vec<f64> = plan.motions.iter().map(|m| m.end.z).collect();
    assert_eq!(
        zs,
        vec![
            2.,   // rapid to R
            -2.,  // feed peck 1
            -1.5, // small in-hole retract
            -4.,  // feed peck 2 (feeding back through the retracted span)
            -3.5, // small in-hole retract
            -6.,  // feed peck 3
            -5.5, // small in-hole retract
            -8.,  // feed peck 4 reaches the bottom
            2.,   // final retract to R
        ]
    );
}

#[test]
fn peck_reduction_shrinks_successive_bites_to_the_minimum() {
    let mut settings = drill_settings(&["a-point"]);
    settings.peck = Some(DrillPeckSettings {
        mode: DrillPeckMode::ChipBreak,
        depth_mm: 3.,
        reduction_mm: 1.,
        min_depth_mm: 1.,
        retract_mm: Some(0.25),
    });
    let plan = OperationPlan::plan_job(&job(settings), &PlanLimits::default()).unwrap();
    // Levels below the top: 3, 5, 6, 7, 8 — the reduction shrinks bites to
    // the 1 mm minimum and the last peck lands exactly at depth.
    let feeds: Vec<f64> = plan
        .motions
        .iter()
        .filter(|m| m.feed_mm_min.is_some())
        .map(|m| m.end.z)
        .collect();
    assert_eq!(feeds, vec![-3., -5., -6., -7., -8.]);
}

#[test]
fn dwell_and_full_diameter_reference_deepen_the_hole() {
    let mut settings = drill_settings(&["a-point"]);
    settings.depth_reference = cam_core::project::DrillDepthReference::FullDiameter;
    settings.dwell_at_bottom_s = Some(0.25);
    let plan = OperationPlan::plan_job(&job(settings), &PlanLimits::default()).unwrap();
    // The 118° tip extension of a Ø5 drill is 1.502152 mm past the bottom.
    let final_z = -8. - 1.502_152;
    let plunge = plan
        .motions
        .iter()
        .find(|m| m.feed_mm_min.is_some())
        .unwrap();
    assert!((plunge.end.z - final_z).abs() < 1e-5);
    // The dwell follows the plunge, holds the position, and carries no feed.
    let dwell = &plan.motions[plan.motions.iter().position(|m| m.id == plunge.id).unwrap() + 1];
    assert_eq!(
        dwell.interpolation,
        cam_core::toolpath::Interpolation::Dwell { seconds: 0.25 }
    );
    assert_eq!(dwell.start, dwell.end);
    assert_eq!(dwell.feed_mm_min, None);
    assert_eq!(check_plan(&plan).unwrap().status, CheckStatus::Passed);
}

#[test]
fn missing_settings_point_reference_and_heights_report_as_incomplete() {
    let cases: Vec<(DrillSettings, &str)> = vec![
        (
            {
                let mut s = drill_settings(&[]);
                s.assignment.plunge_feed_mm_min = None;
                s
            },
            "MISSING_MACHINING_SETTING",
        ),
        (drill_settings(&["missing-point"]), "DRILL_POINT_REFERENCE"),
        (
            {
                let mut s = drill_settings(&["a-point"]);
                s.retract_height.offset_mm = 0.;
                s
            },
            "DRILL_RETRACT_BELOW_TOP",
        ),
        (
            {
                let mut s = drill_settings(&["a-point"]);
                s.retract_height.offset_mm = 10.;
                s
            },
            "DRILL_RETRACT_ABOVE_CLEARANCE",
        ),
    ];
    for (settings, code) in cases {
        let plan = OperationPlan::plan_job(&job(settings), &PlanLimits::default()).unwrap();
        assert_eq!(
            plan.operation_results[0].generation_status,
            cam_core::sequence::GenerationStatus::Incomplete,
            "expected incomplete for {code}"
        );
        assert!(
            plan.generation_diagnostics.iter().any(|d| d.code == code),
            "missing issue {code} for diagnostics {:?}",
            plan.generation_diagnostics
        );
    }
    // An unknown tool is a broken document, not an incomplete plan.
    let mut unknown = drill_settings(&["a-point"]);
    unknown.assignment.tool_id = "unknown".into();
    assert_eq!(
        OperationPlan::plan_job(&job(unknown), &PlanLimits::default())
            .unwrap_err()
            .code,
        "PROJECT_TOOL_REFERENCE"
    );
}

#[test]
fn a_point_outside_the_stock_is_rejected_and_the_drill_length_is_checked() {
    let mut tiny = job(drill_settings(&["b-point"]));
    tiny.setup.stock.xy = Some(RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 5.,
        length_mm: 5.,
    });
    let plan = OperationPlan::plan_job(&tiny, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        cam_core::sequence::GenerationStatus::Incomplete
    );
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "DRILL_POINT_OUTSIDE_STOCK")
    );

    let mut short = job(drill_settings(&["a-point"]));
    short.tools[0].geometry = Some(ToolGeometry::Drill(DrillSpec {
        diameter_mm: 5.,
        tip_angle_deg: 118.,
        cutting_length_mm: 4.,
    }));
    let plan = OperationPlan::plan_job(&short, &PlanLimits::default()).unwrap();
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "DRILL_CUTTING_LENGTH")
    );
}

#[test]
fn coincident_points_and_oversized_drills_are_advisories_not_blockers() {
    let twin_spots = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><circle id="p" cx="10" cy="15" r="1" fill="#000"/><circle id="q" cx="10" cy="15" r="1" fill="#000"/></svg>"##;
    let mut base = job(drill_settings(&["p-point", "q-point"]));
    base.source.as_mut().unwrap().svg = twin_spots.into();
    let plan = OperationPlan::plan_job(&base, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        cam_core::sequence::GenerationStatus::Complete
    );
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "DRILL_POINTS_COINCIDENT")
    );

    let mut warned = base.clone();
    if let OperationSettings::Drill(s) = &mut warned.operations[0].settings {
        s.warn_drill_exceeds_marker = true;
        s.points = vec!["p-point".into()];
    }
    let plan = OperationPlan::plan_job(&warned, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        cam_core::sequence::GenerationStatus::Complete
    );
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "DRILL_EXCEEDS_MARKER")
    );
}

#[test]
fn the_stock_history_records_hole_disks() {
    let plan = OperationPlan::plan_job(
        &job(drill_settings(&["a-point", "b-point"])),
        &PlanLimits::default(),
    )
    .unwrap();
    // Drilling to -8 through 8 mm stock removes the full thickness at the
    // hole centers; halfway between the holes nothing was cut.
    let final_stock = plan.stock_history(None).unwrap();
    assert_eq!(
        final_stock.material_top_at(Point::new(10., 15.)).unwrap(),
        -8.
    );
    assert_eq!(
        final_stock.material_top_at(Point::new(30., 20.)).unwrap(),
        -8.
    );
    assert_eq!(
        final_stock.material_top_at(Point::new(20., 17.5)).unwrap(),
        0.
    );
}

#[test]
fn the_export_emits_expanded_moves_and_verifies_the_dwell_readback() {
    let mut settings = drill_settings(&["a-point"]);
    settings.peck = Some(DrillPeckSettings {
        mode: DrillPeckMode::FullRetract,
        depth_mm: 4.,
        reduction_mm: 0.,
        min_depth_mm: 4.,
        retract_mm: None,
    });
    settings.dwell_at_bottom_s = Some(0.5);
    let plan = OperationPlan::plan_job(&job(settings), &PlanLimits::default()).unwrap();
    let checks = check_plan(&plan).unwrap();
    assert_eq!(
        checks.status,
        CheckStatus::Passed,
        "findings: {:?}",
        checks.findings
    );
    let profile = SequenceProfile {
        schema_version: 2,
        id: "drill-profile".into(),
        work_offset: "G54".into(),
        clearance_z_mm: 5.,
        decimal_places: 3,
        program_start_position_mm: None,
        length_compensation: cam_core::post::LengthCompensation::MacroManaged,
        path_control: cam_core::post::PathControl::ExactPath,
        tools: vec![SequenceToolMapping {
            tool_id: "drill".into(),
            tool_number: 3,
            length_offset_number: None,
        }],
        spindle_spinup_seconds: 0.5,
        holder: None,
        rapid_rate_mm_min: None,
        coolant: cam_core::post::Coolant::Off,
        m6: cam_core::post::M6Contract {
            reference: "test".into(),
            reviewed: true,
            return_position: cam_core::post::M6Return::CallerPosition,
            preserves_work_datum: true,
            local_offsets_unused: true,
            tool_offsets_z_only: true,
        },
    };
    let trusted = TrustedPlan::from_plan(&plan).unwrap();
    let prepared = PreparedExecution::prepare(&trusted, &profile).unwrap();
    let export = prepared.export(&trusted, &profile).unwrap();
    let gcode = &export.program.gcode;
    assert!(gcode.contains("T3 M6"));
    assert!(gcode.contains("M3 S6000"));
    // The spin-up dwell and the bottom dwell are both plain G4 P blocks.
    assert_eq!(gcode.matches("G4 P0.5").count(), 2);
    assert!(gcode.contains("G1 X10.000 Y15.000 Z-4.000 F120"));
    assert!(gcode.contains("G1 X10.000 Y15.000 Z-8.000 F120"));
    assert!(gcode.ends_with("M2\n"));
    assert_eq!(export.report.motion_count, plan.motions.len());
    assert_eq!(
        export.report.basic_checks.status,
        CheckStatus::Passed,
        "findings: {:?}",
        export.report.basic_checks.findings
    );
}

#[test]
fn v5_point_selection_and_planning_round_trip() {
    let base = v5_job();
    // Create the operation with defaults: everything unset, still saveable.
    let outcome = v5::commands::add_operation(
        &base,
        v5::commands::NewOperationKind::Drill,
        "holes",
        "Holes",
    )
    .unwrap();
    let mut job = outcome.job;
    assert!(job.operations[0].enabled);
    // Give the tool geometry and the assignment cutting values.
    job.tools[0].geometry = Some(ToolGeometry::Drill(DrillSpec {
        diameter_mm: 5.,
        tip_angle_deg: 118.,
        cutting_length_mm: 30.,
    }));
    job.tools[0].capabilities.plunge_capable = Some(true);
    let v5::OperationSettingsV5::Drill(settings) = &mut job.operations[0].settings else {
        panic!("drill settings expected");
    };
    settings.assignment.spindle_rpm = Some(6000.);
    settings.assignment.spindle_direction = Some(SpindleDirection::Clockwise);
    settings.assignment.plunge_feed_mm_min = Some(120.);
    // Select both points through the command, bound to the live revision.
    let combined = v5::artwork::inspect_artwork(&job).unwrap();
    let picks: Vec<v5::artwork::GeometryPick> = combined
        .item(&v5::ArtworkItemId("plate".into()))
        .unwrap()
        .point_entries
        .iter()
        .map(|entry| v5::artwork::GeometryPick {
            artwork_item_id: v5::ArtworkItemId("plate".into()),
            kind: v5::GeometryRefKind::Point,
            local_geometry_id: entry.reference.local_geometry_id.clone(),
        })
        .collect();
    let outcome = v5::commands::set_point_selection(&job, "holes", &picks).unwrap();
    let job = outcome.job;
    // The document round-trips through JSON.
    assert_eq!(
        v5::CamJobV5::from_json(&job.to_json().unwrap()).unwrap(),
        job
    );
    // Planning resolves the selection into motions.
    let plan = cam_core::sequence::OperationPlanV5::plan_job_v5(
        &job,
        &v5::ReadinessScope::AllEnabled,
        &PlanLimits::default(),
    )
    .unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        cam_core::sequence::GenerationStatus::Complete
    );
    assert_eq!(plan.stages[0].role, StageRole::Drill);
    // A non-point pick is refused.
    let bad = vec![v5::artwork::GeometryPick {
        artwork_item_id: v5::ArtworkItemId("plate".into()),
        kind: v5::GeometryRefKind::Centerline,
        local_geometry_id: "a-point".into(),
    }];
    assert!(v5::commands::set_point_selection(&job, "holes", &bad).is_err());
}

fn v5_job() -> v5::CamJobV5 {
    v5::CamJobV5 {
        schema_version: v5::CAM_JOB_V5_SCHEMA_VERSION,
        name: "drill".into(),
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(8.),
                xy: Some(RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: 40.,
                    length_mm: 30.,
                }),
            },
            work_zero: WorkZero::default(),
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: Some(Point::new(0., 0.)),
        },
        artwork: vec![v5::ArtworkItem {
            id: v5::ArtworkItemId("plate".into()),
            name: "Plate".into(),
            content: v5::ArtworkContent::Svg(SourceSnapshot {
                filename: "markers.svg".into(),
                svg: MARKERS.into(),
            }),
            import_settings: v5::SvgInterpretation {
                geometry_tolerance_mm: 0.001,
                ticks_per_mm: None,
            },
            placement: Placement {
                origin_mm: Point::new(0., 0.),
                scale: 1.,
                rotation_deg: 0.,
            },
        }],
        tools: vec![],
        operations: vec![],
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
            arc_fit_tolerance_mm: None,
        },
        machine_configuration: None,
    }
}

#[test]
fn an_offset_cannot_authorize_rapid_entry_into_untouched_stock() {
    for (retract, code) in [
        (-2., "DRILL_RETRACT_BELOW_SURFACE"),
        (2., "DRILL_TOP_BELOW_SURFACE"),
    ] {
        let mut settings = drill_settings(&["a-point"]);
        settings.top.offset_mm = -4.;
        settings.retract_height.offset_mm = retract;
        let plan = OperationPlan::plan_job(&job(settings), &PlanLimits::default()).unwrap();
        assert_eq!(
            plan.operation_results[0].generation_status,
            cam_core::sequence::GenerationStatus::Incomplete
        );
        assert!(plan.motions.is_empty());
        assert!(plan.generation_diagnostics.iter().any(|d| d.code == code));
        assert_ne!(check_plan(&plan).unwrap().status, CheckStatus::Passed);
    }
}

#[test]
fn below_stock_retract_requires_a_face_covering_the_entire_drill() {
    use cam_core::project::{EndmillGeometry, FaceArea, FaceMargins, FaceSettings, JobTool};
    for (point, width, ready) in [
        ("a-point", 10., true),
        ("b-point", 10., false),
        ("a-point", 6., false),
    ] {
        let mut settings = drill_settings(&[point]);
        settings.top = HeightRef {
            reference: HeightReference::FaceResult {
                operation_id: "face".into(),
            },
            offset_mm: 0.,
        };
        settings.retract_height.offset_mm = -2.;
        let mut base = job(settings);
        base.tolerances.motion_tolerance_mm = Some(0.01);
        base.tools.push(JobTool {
            id: "endmill".into(),
            name: "Face cutter".into(),
            geometry: Some(ToolGeometry::Endmill(EndmillGeometry {
                diameter_mm: 4.,
                cutting_length_mm: 20.,
            })),
            capabilities: ToolCapabilities {
                plunge_capable: Some(true),
                ramp_capable: None,
            },
        });
        let mut assignment = drill_assignment();
        assignment.tool_id = "endmill".into();
        assignment.cutting_feed_mm_min = Some(600.);
        assignment.max_stepdown_mm = Some(2.);
        assignment.stepover_mm = Some(2.);
        base.operations.insert(
            0,
            Operation {
                id: "face".into(),
                name: "Face".into(),
                enabled: true,
                settings: OperationSettings::Face(FaceSettings {
                    area: FaceArea::Rectangle {
                        rect: RectXY {
                            min_x_mm: 5.,
                            min_y_mm: 10.,
                            width_mm: width,
                            length_mm: 10.,
                        },
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
                        offset_mm: -4.,
                    },
                    stepdown_mm: Some(2.),
                    stepover_mm: Some(2.),
                    pass_angle_deg: Some(0.),
                    pattern: Default::default(),
                    assignment,
                }),
            },
        );
        let plan = OperationPlan::plan_job(&base, &PlanLimits::default()).unwrap();
        assert_eq!(
            plan.operation_results[0].generation_status,
            cam_core::sequence::GenerationStatus::Complete,
            "{:?}",
            plan.generation_diagnostics
        );
        assert_eq!(
            plan.operation_results[1].generation_status
                == cam_core::sequence::GenerationStatus::Complete,
            ready,
            "{:?}",
            plan.generation_diagnostics
        );
        if ready {
            assert_eq!(check_plan(&plan).unwrap().status, CheckStatus::Passed);
        } else {
            assert!(
                plan.generation_diagnostics
                    .iter()
                    .any(|d| d.code == "DRILL_RETRACT_BELOW_SURFACE")
            );
        }
    }
}

#[test]
fn pathological_peck_settings_fail_without_expanding_unbounded_motions() {
    for (depth, reduction, minimum, code) in [
        (1., 1., 1e-20, "DRILL_PECK_PROGRESS"),
        (1e-6, 0., 1e-6, "PROJECT_RESOURCE_LIMIT"),
    ] {
        let mut settings = drill_settings(&["a-point"]);
        settings.peck = Some(DrillPeckSettings {
            mode: DrillPeckMode::FullRetract,
            depth_mm: depth,
            reduction_mm: reduction,
            min_depth_mm: minimum,
            retract_mm: None,
        });
        let plan = OperationPlan::plan_job(&job(settings), &PlanLimits::default()).unwrap();
        assert!(plan.motions.is_empty());
        assert_eq!(
            plan.operation_results[0].generation_status,
            cam_core::sequence::GenerationStatus::Incomplete
        );
        assert!(
            plan.generation_diagnostics.iter().any(|d| d.code == code),
            "{:?}",
            plan.generation_diagnostics
        );
    }
}
