//! C2 face workflow: FaceResult consumption by the Flat V-carve adapter in
//! its local frame, uniform-top admission, and the plan-section-19.1
//! face-then-carve fixture with bottom-zero output.
use cam_core::{
    checks::CheckStatus,
    geometry::Point,
    post::{
        LinuxCncProfile,
        sequence::{PreparedExecution, SequenceProfile},
    },
    project::{
        AnchorFraction, CamJob, FaceArea, FaceMargins, FacePattern, FaceSettings, FlatVcarveMode,
        FlatVcarveRoughSettings, FlatVcarveSettings, HeightRef, HeightReference, JobTool,
        MillingAssignment, Operation, OperationSettings, RectXY, SetupSettings, StockSetup,
        ToolCapabilities, ToolGeometry, WorkZero, WorkZeroXY, WorkZeroZ,
    },
    sequence::{GenerationStatus, OperationPlan, PlanLimits, TrustedPlan},
    toolpath::MotionEffect,
};

const LEGACY_PROFILE: &str = include_str!("../../../../real_data/machine-profile.json");
const CARVE_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="pocket" fill-rule="evenodd" d="M5 5h30v20h-30z"/></svg>"#;

fn tools() -> Vec<JobTool> {
    vec![
        JobTool {
            id: "endmill".into(),
            name: "4mm endmill".into(),
            geometry: Some(ToolGeometry::Endmill(cam_core::project::EndmillGeometry {
                diameter_mm: 4.,
                cutting_length_mm: 8.,
            })),
            capabilities: ToolCapabilities {
                plunge_capable: Some(true),
                ramp_capable: Some(false),
            },
        },
        JobTool {
            id: "vbit".into(),
            name: "90 degree V-bit".into(),
            geometry: Some(ToolGeometry::Vbit(cam_core::model::VBitSpec {
                included_angle_deg: 90.,
                tip_diameter_mm: 0.,
                max_cutting_diameter_mm: 12.,
                cutting_height_mm: 6.,
            })),
            capabilities: ToolCapabilities {
                plunge_capable: Some(true),
                ramp_capable: None,
            },
        },
    ]
}

fn milling(tool: &str) -> MillingAssignment {
    MillingAssignment {
        tool_id: tool.into(),
        spindle_rpm: Some(10_000.),
        spindle_direction: Some(cam_core::project::SpindleDirection::Clockwise),
        cutting_feed_mm_min: Some(300.),
        plunge_feed_mm_min: Some(100.),
        max_stepdown_mm: Some(1.),
        stepover_mm: Some(1.5),
    }
}

fn face_op(id: &str, rect: RectXY, depth: f64) -> Operation {
    Operation {
        id: id.into(),
        name: "Face".into(),
        enabled: true,
        settings: OperationSettings::Face(FaceSettings {
            area: FaceArea::Rectangle { rect },
            margins: FaceMargins::default(),
            entry_overrun_mm: Some(1.),
            exit_overrun_mm: Some(1.),
            top: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: 0.,
            },
            bottom: HeightRef {
                reference: HeightReference::StockTop,
                offset_mm: -depth,
            },
            stepdown_mm: Some(depth),
            stepover_mm: Some(3.),
            pass_angle_deg: Some(0.),
            pattern: FacePattern::ZigZag,
            assignment: milling("endmill"),
        }),
    }
}

fn carve_op(id: &str, face_id: Option<&str>, depth: f64) -> Operation {
    Operation {
        id: id.into(),
        name: "Carve".into(),
        enabled: true,
        settings: OperationSettings::FlatVcarve(FlatVcarveSettings {
            component_ids: vec!["pocket::0".into()],
            mode: FlatVcarveMode::EndmillOnly,
            top: HeightRef {
                reference: match face_id {
                    Some(id) => HeightReference::FaceResult {
                        operation_id: id.into(),
                    },
                    None => HeightReference::StockTop,
                },
                offset_mm: 0.,
            },
            endmill: milling("endmill"),
            vbit: milling("vbit"),
            max_depth_mm: Some(depth),
            wall_allowance_mm: Some(0.5),
            max_floor_ridge_mm: Some(0.),
            max_detail_residual_mm: Some(0.),
            rough: Some(FlatVcarveRoughSettings {
                strategy: cam_core::pocket::ClearingStrategy::DepthDependent,
                entry: cam_core::pocket::EntryStrategy::Plunge,
                max_layers: 16,
                max_loops_per_layer: 64,
                max_motions: 10_000,
            }),
            finish: None,
        }),
    }
}

fn job(operations: Vec<Operation>) -> CamJob {
    CamJob {
        schema_version: 4,
        name: "Face then carve".into(),
        source: Some(cam_core::job::SourceSnapshot {
            filename: "art.svg".into(),
            svg: CARVE_SVG.into(),
        }),
        import: Default::default(),
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
            work_zero: WorkZero {
                xy: WorkZeroXY::StockAnchor {
                    x_fraction: AnchorFraction::Center,
                    y_fraction: AnchorFraction::Center,
                },
                z: WorkZeroZ::StockBottom,
            },
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: Some(cam_core::geometry::Point::new(0., 0.)),
        },
        tools: tools(),
        operations,
        tolerances: cam_core::job::PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
        },
        legacy_machine_profile: None,
    }
}

fn sequence_profile() -> SequenceProfile {
    SequenceProfile {
        schema_version: 2,
        id: "printnc".into(),
        work_offset: "G54".into(),
        clearance_z_mm: 5.,
        decimal_places: 3,
        program_start_position_mm: None,
        length_compensation: cam_core::post::LengthCompensation::MacroManaged,
        path_control: cam_core::post::PathControl::ExactPath,
        tools: vec![
            cam_core::post::sequence::SequenceToolMapping {
                tool_id: "endmill".into(),
                tool_number: 1,
                length_offset_number: None,
            },
            cam_core::post::sequence::SequenceToolMapping {
                tool_id: "vbit".into(),
                tool_number: 2,
                length_offset_number: None,
            },
        ],
        spindle_spinup_seconds: 0.5,
        coolant: cam_core::post::Coolant::Off,
        m6: legacy_m6(),
    }
}
fn legacy_m6() -> cam_core::post::M6Contract {
    LinuxCncProfile::from_json(LEGACY_PROFILE).unwrap().m6
}

/// The plan-section-19.1 fixture: face 0.5, then carve 2 below the faced
/// plane. Carve top=-0.5, bottom=-2.5, stock bottom=-8, and the bottom-zero
/// output reaches +5.5.
#[test]
fn face_then_carve_uses_the_faced_plane_and_original_work_zero() {
    let face_rect = RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 40.,
        length_mm: 30.,
    };
    let job = job(vec![
        face_op("face-1", face_rect, 0.5),
        carve_op("carve-1", Some("face-1"), 2.),
    ]);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    for result in &plan.operation_results {
        assert_eq!(
            result.generation_status,
            GenerationStatus::Complete,
            "operation {} complete: {:?}",
            result.operation_id,
            plan.generation_diagnostics
        );
    }
    // The face plane is published at -0.5.
    let plane = &plan.operation_results[0].named_outputs[0];
    assert_eq!(plane.z_mm, Some(-0.5));

    // Carve motions live in setup coordinates below the faced plane: the
    // deepest cut is exactly top - 2 = -2.5, and the clearance moves stay at
    // the original +5 plane (not the local 5.5).
    let carve_start = plan
        .stages
        .iter()
        .find(|s| s.operation_id == "carve-1")
        .unwrap()
        .motion_range
        .0;
    let carve_motions = &plan.motions[carve_start..];
    let deepest = carve_motions
        .iter()
        .map(|m| m.start.z.min(m.end.z))
        .fold(f64::INFINITY, f64::min);
    assert!(
        (deepest - -2.5).abs() < 1e-9,
        "deepest carve cut {deepest} != -2.5"
    );
    let highest = carve_motions
        .iter()
        .map(|m| m.start.z.max(m.end.z))
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        (highest - 5.).abs() < 1e-9,
        "clearance stays at the original +5 plane, got {highest}"
    );

    // The stock history composes both: faced floor -0.5 outside the carve,
    // carved floor at -2.5 inside it, stock bottom still -8.
    let history = plan.stock_history(None).unwrap();
    // Artwork placement: pocket 5..35 x 5..25 on a 40x30 page at scale 1.
    let outside_carve = Point::new(6., 6.);
    let inside_carve = Point::new(20., 15.);
    let faced_floor = history.material_top_at(outside_carve).unwrap();
    assert!(
        (faced_floor - -0.5).abs() < 1e-9,
        "faced floor {faced_floor} != -0.5"
    );
    let carved_floor = history.material_top_at(inside_carve).unwrap();
    assert!(
        (carved_floor - -2.5).abs() < 1e-6,
        "carved floor {carved_floor} != -2.5"
    );

    // Bottom-zero numeric output: deepest -2.5 becomes machine +5.5.
    let trusted = TrustedPlan::from_generated(plan);
    let profile = sequence_profile();
    let prepared = PreparedExecution::prepare(&trusted, &profile).unwrap();
    assert_eq!(prepared.machine_offset_mm, [20., 15., -8.]);
    let export = prepared.export(&trusted, &profile).unwrap();
    assert_eq!(export.report.basic_checks.status, CheckStatus::Passed);
    assert!(
        export.program.gcode.contains("Z5.500"),
        "deepest cut exports at machine Z5.500"
    );
    assert!(
        export.program.gcode.contains("Z13.000"),
        "clearance +5 exports at machine Z13.000"
    );
}

/// A carve crossing outside the established planar coverage is rejected; no
/// assumed global surface shift.
#[test]
fn partial_face_reference_crossing_is_rejected() {
    // The face covers only the left half; the artwork spans the full width.
    let partial = RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 20.,
        length_mm: 30.,
    };
    let job = job(vec![
        face_op("face-1", partial, 0.5),
        carve_op("carve-1", Some("face-1"), 2.),
    ]);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[1].generation_status,
        GenerationStatus::Incomplete
    );
    let crossing = plan
        .generation_diagnostics
        .iter()
        .find(|d| d.code == "SURFACE_REFERENCE_OUTSIDE_COVERAGE")
        .expect("crossing diagnostic");
    assert!(crossing.message.contains("carve-1"));
    // The face itself still completed; export of the sequence is unavailable.
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete
    );
    assert!(!cam_core::checks::check_plan(&plan).unwrap().export_ready);
}

/// Earlier removal below the faced plane inside the carve target invalidates
/// the fresh-planar assumption.
#[test]
fn earlier_removal_below_the_faced_plane_is_rejected() {
    let full = RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 40.,
        length_mm: 30.,
    };
    let job = job(vec![
        face_op("face-1", full, 0.5),
        face_op("face-2", full, 3.),
        carve_op("carve-1", Some("face-1"), 2.),
    ]);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[2].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "SURFACE_REFERENCE_OUTSIDE_COVERAGE"
                && d.message.contains("below the faced plane"))
    );
}

/// A carve without a face reference keeps stock-top behavior (regression for
/// the legacy equivalence contract).
#[test]
fn stock_top_carve_keeps_legacy_coordinates() {
    let job = job(vec![carve_op("carve-1", None, 2.)]);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete
    );
    let deepest = plan
        .motions
        .iter()
        .map(|m| m.start.z.min(m.end.z))
        .fold(f64::INFINITY, f64::min);
    assert!((deepest - -2.).abs() < 1e-9, "deepest cut {deepest} != -2.");
    let cutting = plan
        .motions
        .iter()
        .any(|m| m.effect == MotionEffect::MillingSweep);
    assert!(cutting);
}
