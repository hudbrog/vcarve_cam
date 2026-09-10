//! D3 profile workflow: the plan-section-19.1 mixed fixture — Face (T1) ->
//! Flat V-carve (T1 rough, T2 finish) -> Profile (T1 again) — with the
//! recurring tool re-selected in order, process state re-established per
//! stage, complete material history in one stock view, and face-plane
//! references from profile tops admitted only inside the faced coverage.
use cam_core::{
    checks::{CheckStatus, check_plan},
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    post::{
        LinuxCncProfile,
        sequence::{PreparedExecution, SequenceProfile, SequenceToolMapping},
    },
    project::{
        AnchorFraction, CamJob, ContourSide, CutDirection, FaceArea, FaceMargins, FacePattern,
        FaceSettings, FlatVcarveMode, FlatVcarveRoughSettings, FlatVcarveSettings, HeightRef,
        HeightReference, JobTool, MillingAssignment, Operation, OperationSettings, ProfileContour,
        ProfileSettings, RectXY, SetupSettings, SpindleDirection, StockSetup, ToolCapabilities,
        ToolGeometry, WorkZero, WorkZeroXY, WorkZeroZ,
    },
    sequence::{ExecutionItem, GenerationStatus, OperationPlan, PlanLimits, TrustedPlan},
    toolpath::{MotionEffect, MotionPurpose},
};

const LEGACY_PROFILE: &str = include_str!("../../../../real_data/machine-profile.json");
/// Pocket 5..35 x 5..25 on a 40x30 page at scale 1: one filled component
/// whose outer contour (`pocket-0-outer`) is the profile target.
const ART_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="pocket" fill-rule="evenodd" d="M5 5h30v20h-30z"/></svg>"#;

fn tools() -> Vec<JobTool> {
    vec![
        JobTool {
            id: "endmill".into(),
            name: "4mm endmill".into(),
            geometry: Some(ToolGeometry::Endmill(cam_core::project::EndmillGeometry {
                diameter_mm: 4.,
                cutting_length_mm: 12.,
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
                tip_diameter_mm: 1.,
                max_cutting_diameter_mm: 12.,
                cutting_height_mm: 5.,
            })),
            capabilities: ToolCapabilities {
                plunge_capable: Some(true),
                ramp_capable: None,
            },
        },
    ]
}

fn milling(tool: &str, feed: f64, stepover: f64) -> MillingAssignment {
    MillingAssignment {
        tool_id: tool.into(),
        spindle_rpm: Some(10_000.),
        spindle_direction: Some(SpindleDirection::Clockwise),
        cutting_feed_mm_min: Some(feed),
        plunge_feed_mm_min: Some(100.),
        max_stepdown_mm: Some(8.),
        stepover_mm: Some(stepover),
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
            assignment: milling("endmill", 300., 1.5),
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
            mode: FlatVcarveMode::Combined,
            top: HeightRef {
                reference: match face_id {
                    Some(id) => HeightReference::FaceResult {
                        operation_id: id.into(),
                    },
                    None => HeightReference::StockTop,
                },
                offset_mm: 0.,
            },
            endmill: milling("endmill", 300., 1.5),
            vbit: milling("vbit", 250., 0.5),
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
            finish: Some(cam_core::vcarve::VBitPlanningSettings {
                max_paths: 4096,
                max_motions: 100_000,
                max_curve_segments: 20_000,
                max_depth_passes: 8,
                max_cleanup_iterations: 2,
                quality_sample_spacing_mm: 0.5,
                max_quality_samples: 20_000,
                reachability_max_cells: 4096,
                stock_slices: 4,
            }),
        }),
    }
}

fn profile_op(id: &str, face_id: Option<&str>, through: Option<f64>) -> Operation {
    let top = HeightRef {
        reference: match face_id {
            Some(id) => HeightReference::FaceResult {
                operation_id: id.into(),
            },
            None => HeightReference::StockTop,
        },
        offset_mm: 0.,
    };
    Operation {
        id: id.into(),
        name: "Profile".into(),
        enabled: true,
        settings: OperationSettings::Profile(ProfileSettings {
            contours: vec![ProfileContour {
                contour_id: "pocket-0-outer".into(),
                side: ContourSide::Outside,
                traversal: None,
            }],
            assignment: milling("endmill", 350., 1.5),
            top,
            bottom: HeightRef {
                reference: HeightReference::StockBottom,
                offset_mm: -through.unwrap_or(0.),
            },
            stepdown_mm: Some(3.),
            through_cut_allowance_mm: through,
            direction: Some(CutDirection::Climb),
            order: Default::default(),
            start: Default::default(),
            finish: Default::default(),
            entry: Default::default(),
            lead_in: Default::default(),
            lead_out: Default::default(),
            tabs: None,
        }),
    }
}

fn job(operations: Vec<Operation>) -> CamJob {
    CamJob {
        schema_version: 4,
        name: "Face, carve, profile".into(),
        source: Some(SourceSnapshot {
            filename: "art.svg".into(),
            svg: ART_SVG.into(),
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
            start_xy_mm: Some(Point::new(0., 0.)),
        },
        tools: tools(),
        operations,
        tolerances: PlanningTolerances {
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
            SequenceToolMapping {
                tool_id: "endmill".into(),
                tool_number: 1,
                length_offset_number: None,
            },
            SequenceToolMapping {
                tool_id: "vbit".into(),
                tool_number: 2,
                length_offset_number: None,
            },
        ],
        spindle_spinup_seconds: 0.5,
        coolant: cam_core::post::Coolant::Off,
        m6: LinuxCncProfile::from_json(LEGACY_PROFILE).unwrap().m6,
    }
}

fn full_stock() -> RectXY {
    RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 40.,
        length_mm: 30.,
    }
}

/// The D3 acceptance fixture: face, carve, then a profile that brings the
/// endmill back after the V-bit stage. Tool changes follow execution order
/// (no regrouping by tool), every stage re-establishes its process intent,
/// the material history composes all three operations, and the ordered
/// export survives numeric readback.
#[test]
fn face_carve_profile_recurring_tool_preserves_process_order_and_stock_history() {
    let job = job(vec![
        face_op("face-1", full_stock(), 0.5),
        carve_op("carve-1", Some("face-1"), 2.),
        profile_op("profile-1", Some("face-1"), Some(0.2)),
    ]);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    for result in &plan.operation_results {
        assert_eq!(
            result.generation_status,
            GenerationStatus::Complete,
            "operation {} incomplete: {:?}",
            result.operation_id,
            plan.generation_diagnostics
        );
    }
    assert_eq!(check_plan(&plan).unwrap().status, CheckStatus::Passed);

    // Stage order and tools: face T1 -> carve rough T1 -> carve finish T2 ->
    // profile T1. The endmill recurs and is re-selected in execution order.
    let stage_summary: Vec<(&str, &str, &str)> = plan
        .stages
        .iter()
        .map(|s| {
            (
                s.stage_id.as_str(),
                s.tool_id.as_str(),
                match s.role {
                    cam_core::sequence::StageRole::Face => "face",
                    cam_core::sequence::StageRole::VcarveRough => "vcarve_rough",
                    cam_core::sequence::StageRole::VcarveFinish => "vcarve_finish",
                    cam_core::sequence::StageRole::ProfileRough => "profile_rough",
                    other => panic!("unexpected role {other:?}"),
                },
            )
        })
        .collect();
    assert_eq!(
        stage_summary,
        vec![
            ("face-1-face", "endmill", "face"),
            ("carve-1-vcarve-rough", "endmill", "vcarve_rough"),
            ("carve-1-vcarve-finish", "vbit", "vcarve_finish"),
            ("profile-1-profile-rough", "endmill", "profile_rough"),
        ]
    );

    // Execution: one tool change per tool gap (consecutive same-tool stages
    // share it), and a process intent before every stage run.
    let mut tool_changes: Vec<&str> = vec![];
    let mut runs_after_change: Vec<usize> = vec![];
    let mut stages_run = 0usize;
    let mut intents = 0usize;
    for item in &plan.execution {
        match item {
            ExecutionItem::ToolChange { tool_id } => {
                tool_changes.push(tool_id);
                runs_after_change.push(stages_run);
            }
            ExecutionItem::SetProcessIntent { .. } => intents += 1,
            ExecutionItem::RunStage { .. } => stages_run += 1,
        }
    }
    assert_eq!(stages_run, 4, "every stage executes exactly once");
    assert_eq!(intents, 4, "every stage re-establishes its intent");
    assert_eq!(tool_changes, vec!["endmill", "vbit", "endmill"]);

    // Depth schedule of the through profile: below the faced plane at -0.5
    // with stepdown 3, ending exactly at -8.2 (plan fixture: 8 mm stock,
    // 0.2 mm below-stock permission).
    let profile_first = plan
        .stages
        .iter()
        .find(|s| s.operation_id == "profile-1")
        .unwrap()
        .motion_range
        .0;
    let mut cut_depths: Vec<f64> = plan.motions[profile_first..]
        .iter()
        .filter(|m| m.purpose == MotionPurpose::Rough && m.effect == MotionEffect::MillingSweep)
        .map(|m| m.end.z)
        .collect();
    cut_depths.dedup();
    cut_depths.sort_by(|a, b| b.partial_cmp(a).unwrap());
    let expected = [-3.5, -6.5, -8.2];
    assert_eq!(cut_depths.len(), expected.len(), "three depth passes");
    for (got, want) in cut_depths.iter().zip(expected) {
        assert!((got - want).abs() < 1e-9, "layer {got} != {want}");
    }

    // Complete material history in one stock view: faced floor outside every
    // later cut, carved floor inside the pocket, profile corridor cut through
    // to the physical bottom.
    let history = plan.stock_history(None).unwrap();
    let top_at = |x: f64, y: f64| history.material_top_at(Point::new(x, y)).unwrap();
    let faced = top_at(0.5, 0.5);
    assert!((faced - -0.5).abs() < 1e-9, "faced floor {faced} != -0.5");
    let carved = top_at(20., 15.);
    assert!(
        (carved - -2.5).abs() < 1e-6,
        "carved floor {carved} != -2.5"
    );
    // The outside profile's cutter band runs 1..5 mm from the pocket edge.
    let through = top_at(3., 15.);
    assert!(
        (through - -8.).abs() < 1e-9,
        "profile corridor cut through to the stock bottom, got {through}"
    );

    // Ordered export through the recurring tool: M6 groups appear as
    // T1, T2, T1 in execution order and the readback verifies the bytes.
    let trusted = TrustedPlan::from_generated(plan);
    let profile = sequence_profile();
    let prepared = PreparedExecution::prepare(&trusted, &profile).unwrap();
    let export = prepared.export(&trusted, &profile).unwrap();
    assert_eq!(export.report.basic_checks.status, CheckStatus::Passed);
    let mut tool_numbers: Vec<u32> = vec![];
    for line in export.program.gcode.lines() {
        if let Some(rest) = line.strip_prefix('T')
            && let Some(number) = rest.strip_suffix(" M6")
        {
            tool_numbers.push(number.parse().unwrap());
        }
    }
    assert_eq!(
        tool_numbers,
        vec![1, 1, 2, 1],
        "T1,T1,T2,T1 in execution order"
    );
    // The recurring endmill re-establishes its spindle state after the V-bit.
    assert!(export.program.gcode.contains("M3"));
}

/// A profile top referencing a face plane admits only corridors inside the
/// faced coverage; crossing the boundary keeps the profile incomplete
/// without disturbing the face itself.
#[test]
fn profile_referencing_partial_face_coverage_is_rejected() {
    let partial = RectXY {
        min_x_mm: 0.,
        min_y_mm: 0.,
        width_mm: 20.,
        length_mm: 30.,
    };
    let job = job(vec![
        face_op("face-1", partial, 0.5),
        profile_op("profile-1", Some("face-1"), None),
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
        .expect("coverage diagnostic");
    assert!(crossing.message.contains("pocket-0-outer"));
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete
    );
    assert!(!check_plan(&plan).unwrap().export_ready);
}

/// A profile top inside the faced coverage resolves against the plane; the
/// stock-bottom bottom with through allowance keeps its explicit permission.
#[test]
fn profile_inside_faced_coverage_resolves_against_the_plane() {
    let job = job(vec![
        face_op("face-1", full_stock(), 0.5),
        profile_op("profile-1", Some("face-1"), None),
    ]);
    let plan = OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[1].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        plan.generation_diagnostics
    );
    // Top at the faced plane -0.5; bottom exactly at the stock bottom -8.
    let profile_first = plan
        .stages
        .iter()
        .find(|s| s.operation_id == "profile-1")
        .unwrap()
        .motion_range
        .0;
    let depths: Vec<f64> = plan.motions[profile_first..]
        .iter()
        .filter(|m| m.purpose == MotionPurpose::Rough && m.effect == MotionEffect::MillingSweep)
        .map(|m| m.end.z)
        .collect();
    assert!(depths.iter().all(|z| *z <= -0.5 + 1e-9));
    assert!(
        (depths.iter().cloned().fold(f64::INFINITY, f64::min) - -8.).abs() < 1e-9,
        "deepest pass ends exactly at the stock bottom"
    );
}
