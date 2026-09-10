//! F1 knife stage data contract: spindle-off process preparation between
//! milling stages, the pivot/tip display contract on knife motions, and the
//! unadvertised planner boundary. The plans here are hand-assembled to the
//! exact sequence contract; the knife planner itself ships in F2 and stays
//! unadvertised.
use cam_core::{
    checks::{CheckStatus, check_plan},
    geometry::Point,
    job::PlanningTolerances,
    motion::Position,
    post::{
        Coolant, LengthCompensation, LinuxCncProfile, PathControl,
        sequence::{PreparedExecution, SequenceProfile, SequenceToolMapping, verify_program},
    },
    project::{
        CamJob, DragKnifeSettings, DragKnifeSpec, EndmillGeometry, FaceArea, FaceSettings,
        HeightRef, JobTool, KnifeAlignment, KnifeAssignment, MillingAssignment, Operation,
        OperationSettings, SetupSettings, StockSetup, ToolCapabilities, ToolGeometry,
    },
    sequence::{
        CoolantIntent, ExecutionItem, ExecutionStage, GenerationStatus, OperationPlan,
        OperationResult, PathControlIntent, PlanIssue, PlanLimits, ProcessIntent, ProcessSpindle,
        StageRole, TrustedPlan,
    },
    toolpath::{Interpolation, MotionEffect, MotionPurpose, PlannedMotion, knife_tip},
};

const ENDMILL: &str = "endmill";
const KNIFE: &str = "knife";

fn face(id: &str) -> Operation {
    Operation {
        id: id.into(),
        name: id.into(),
        enabled: true,
        settings: OperationSettings::Face(FaceSettings {
            area: FaceArea::EntireStock,
            margins: Default::default(),
            entry_overrun_mm: Some(1.),
            exit_overrun_mm: Some(1.),
            top: HeightRef {
                reference: Default::default(),
                offset_mm: 0.,
            },
            bottom: HeightRef {
                reference: Default::default(),
                offset_mm: -0.5,
            },
            stepdown_mm: Some(0.5),
            stepover_mm: Some(3.),
            pass_angle_deg: Some(0.),
            pattern: Default::default(),
            assignment: milling(),
        }),
    }
}

fn milling() -> MillingAssignment {
    MillingAssignment {
        tool_id: ENDMILL.into(),
        spindle_rpm: Some(10000.),
        spindle_direction: Some(cam_core::project::SpindleDirection::Clockwise),
        cutting_feed_mm_min: Some(300.),
        plunge_feed_mm_min: Some(100.),
        max_stepdown_mm: Some(0.5),
        stepover_mm: Some(3.),
    }
}

fn knife_operation() -> Operation {
    Operation {
        id: "knife-1".into(),
        name: "knife-1".into(),
        enabled: true,
        settings: OperationSettings::DragKnife(DragKnifeSettings {
            chains: vec![],
            assignment: KnifeAssignment {
                tool_id: KNIFE.into(),
                cutting_feed_mm_min: Some(150.),
                plunge_feed_mm_min: Some(60.),
                swivel_feed_mm_min: Some(50.),
                max_stepdown_mm: Some(1.),
            },
            top: HeightRef {
                reference: Default::default(),
                offset_mm: 0.,
            },
            bottom: HeightRef {
                reference: cam_core::project::HeightReference::OperationTop,
                offset_mm: -1.,
            },
            stepdown_mm: Some(1.),
            swivel_depth_mm: Some(0.2),
            corner_threshold_deg: Some(30.),
            through_cut_allowance_mm: None,
            start: Default::default(),
            closure_overlap_mm: None,
            alignment: KnifeAlignment::default(),
        }),
    }
}

fn job() -> CamJob {
    let job = CamJob {
        schema_version: 4,
        name: "knife-stage".into(),
        source: None,
        import: Default::default(),
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(8.),
                xy: Some(cam_core::project::RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: 100.,
                    length_mm: 60.,
                }),
            },
            work_zero: Default::default(),
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: None,
        },
        tools: vec![
            JobTool {
                id: ENDMILL.into(),
                name: "6 mm endmill".into(),
                geometry: Some(ToolGeometry::Endmill(EndmillGeometry {
                    diameter_mm: 6.,
                    cutting_length_mm: 20.,
                })),
                capabilities: ToolCapabilities {
                    plunge_capable: Some(true),
                    ramp_capable: None,
                },
            },
            JobTool {
                id: KNIFE.into(),
                name: "drag knife".into(),
                geometry: Some(ToolGeometry::DragKnife(DragKnifeSpec {
                    blade_offset_mm: 1.,
                    max_cut_depth_mm: 3.,
                })),
                capabilities: Default::default(),
            },
        ],
        operations: vec![face("face-1"), knife_operation(), face("face-2")],
        tolerances: PlanningTolerances::default(),
        legacy_machine_profile: None,
    };
    job.validate().unwrap();
    job
}

fn pos(x: f64, y: f64, z: f64) -> Position {
    Position::new(Point::new(x, y), z)
}

#[allow(clippy::too_many_arguments)]
fn motion(
    id: usize,
    operation: &str,
    stage: &str,
    tool: &str,
    purpose: MotionPurpose,
    effect: MotionEffect,
    interpolation: Interpolation,
    start: Position,
    end: Position,
    feed: Option<f64>,
    heading: Option<(f64, f64)>,
) -> PlannedMotion {
    PlannedMotion {
        id,
        operation_id: operation.into(),
        stage_id: stage.into(),
        tool_id: tool.into(),
        contour_id: None,
        pass_id: 0,
        layer: 0,
        interpolation,
        purpose,
        effect,
        start,
        end,
        feed_mm_min: feed,
        blade_heading_deg: heading,
    }
}

/// The face → knife → face plan of plan section 19.1: a recurring endmill
/// around one knife stage, assembled to the exact sequence contract.
fn plan() -> OperationPlan {
    let motions = vec![
        // face-1 (T1): plunge, cut, retract.
        motion(
            0,
            "face-1",
            "face-1-face",
            ENDMILL,
            MotionPurpose::Entry,
            MotionEffect::MillingSweep,
            Interpolation::LinearFeed,
            pos(0., 0., 0.),
            pos(0., 0., -0.5),
            Some(100.),
            None,
        ),
        motion(
            1,
            "face-1",
            "face-1-face",
            ENDMILL,
            MotionPurpose::Rough,
            MotionEffect::MillingSweep,
            Interpolation::LinearFeed,
            pos(0., 0., -0.5),
            pos(5., 0., -0.5),
            Some(300.),
            None,
        ),
        motion(
            2,
            "face-1",
            "face-1-face",
            ENDMILL,
            MotionPurpose::Clearance,
            MotionEffect::None,
            Interpolation::Rapid,
            pos(5., 0., -0.5),
            pos(5., 0., 5.),
            None,
            None,
        ),
        // knife-1 (T3): the plan-section-19.1 right-angle corner. Programmed
        // XY is the holder pivot; the blade (offset 1) trails by 180 degrees
        // of travel. The swivel pivots around the tip corner (20, 0).
        motion(
            3,
            "knife-1",
            "knife-1-knife",
            KNIFE,
            MotionPurpose::Clearance,
            MotionEffect::None,
            Interpolation::Rapid,
            pos(5., 0., 5.),
            pos(11., 0., 5.),
            None,
            Some((180., 180.)),
        ),
        motion(
            4,
            "knife-1",
            "knife-1-knife",
            KNIFE,
            MotionPurpose::Entry,
            MotionEffect::KnifeTrace,
            Interpolation::LinearFeed,
            pos(11., 0., 5.),
            pos(11., 0., -1.),
            Some(60.),
            Some((180., 180.)),
        ),
        motion(
            5,
            "knife-1",
            "knife-1-knife",
            KNIFE,
            MotionPurpose::KnifeCut,
            MotionEffect::KnifeTrace,
            Interpolation::LinearFeed,
            pos(11., 0., -1.),
            pos(21., 0., -1.),
            Some(150.),
            Some((180., 180.)),
        ),
        motion(
            6,
            "knife-1",
            "knife-1-knife",
            KNIFE,
            MotionPurpose::KnifeSwivel,
            MotionEffect::KnifeTrace,
            Interpolation::LinearFeed,
            pos(21., 0., -1.),
            pos(20., 1., -1.),
            Some(50.),
            Some((180., 270.)),
        ),
        motion(
            7,
            "knife-1",
            "knife-1-knife",
            KNIFE,
            MotionPurpose::KnifeCut,
            MotionEffect::KnifeTrace,
            Interpolation::LinearFeed,
            pos(20., 1., -1.),
            pos(20., 11., -1.),
            Some(150.),
            Some((270., 270.)),
        ),
        motion(
            8,
            "knife-1",
            "knife-1-knife",
            KNIFE,
            MotionPurpose::Clearance,
            MotionEffect::None,
            Interpolation::Rapid,
            pos(20., 11., -1.),
            pos(20., 11., 5.),
            None,
            Some((270., 270.)),
        ),
        // face-2 (T1 again): the recurring tool is re-selected.
        motion(
            9,
            "face-2",
            "face-2-face",
            ENDMILL,
            MotionPurpose::Entry,
            MotionEffect::MillingSweep,
            Interpolation::LinearFeed,
            pos(30., 0., 0.),
            pos(30., 0., -0.5),
            Some(100.),
            None,
        ),
        motion(
            10,
            "face-2",
            "face-2-face",
            ENDMILL,
            MotionPurpose::Rough,
            MotionEffect::MillingSweep,
            Interpolation::LinearFeed,
            pos(30., 0., -0.5),
            pos(35., 0., -0.5),
            Some(300.),
            None,
        ),
        motion(
            11,
            "face-2",
            "face-2-face",
            ENDMILL,
            MotionPurpose::Clearance,
            MotionEffect::None,
            Interpolation::Rapid,
            pos(35., 0., -0.5),
            pos(35., 0., 5.),
            None,
            None,
        ),
    ];
    let stages = vec![
        ExecutionStage {
            stage_id: "face-1-face".into(),
            operation_id: "face-1".into(),
            tool_id: ENDMILL.into(),
            role: StageRole::Face,
            motion_range: (0, 3),
            entry_position: pos(0., 0., 0.),
            exit_position: pos(5., 0., 5.),
        },
        ExecutionStage {
            stage_id: "knife-1-knife".into(),
            operation_id: "knife-1".into(),
            tool_id: KNIFE.into(),
            role: StageRole::Knife,
            motion_range: (3, 9),
            entry_position: pos(5., 0., 5.),
            exit_position: pos(20., 11., 5.),
        },
        ExecutionStage {
            stage_id: "face-2-face".into(),
            operation_id: "face-2".into(),
            tool_id: ENDMILL.into(),
            role: StageRole::Face,
            motion_range: (9, 12),
            entry_position: pos(30., 0., 0.),
            exit_position: pos(35., 0., 5.),
        },
    ];
    let milling_intent = ProcessIntent {
        spindle: ProcessSpindle::Milling {
            rpm: 10000.,
            direction: Some(cam_core::project::SpindleDirection::Clockwise),
        },
        coolant: CoolantIntent::UseMachineProfile,
        path_control: PathControlIntent::UseMachineProfile,
    };
    let knife_intent = ProcessIntent {
        spindle: ProcessSpindle::Off,
        coolant: CoolantIntent::UseMachineProfile,
        path_control: PathControlIntent::UseMachineProfile,
    };
    let execution = vec![
        ExecutionItem::ToolChange {
            tool_id: ENDMILL.into(),
        },
        ExecutionItem::SetProcessIntent {
            intent: milling_intent.clone(),
        },
        ExecutionItem::RunStage {
            stage_id: "face-1-face".into(),
        },
        ExecutionItem::ToolChange {
            tool_id: KNIFE.into(),
        },
        ExecutionItem::SetProcessIntent {
            intent: knife_intent,
        },
        ExecutionItem::RunStage {
            stage_id: "knife-1-knife".into(),
        },
        ExecutionItem::ToolChange {
            tool_id: ENDMILL.into(),
        },
        ExecutionItem::SetProcessIntent {
            intent: milling_intent,
        },
        ExecutionItem::RunStage {
            stage_id: "face-2-face".into(),
        },
    ];
    let results = ["face-1", "knife-1", "face-2"]
        .into_iter()
        .zip(["face-1-face", "knife-1-knife", "face-2-face"])
        .map(|(operation, stage)| OperationResult {
            operation_id: operation.into(),
            generation_status: GenerationStatus::Complete,
            stage_ids: vec![stage.into()],
            stock_before_id: format!("{operation}-before"),
            stock_after_id: format!("{operation}-after"),
            named_outputs: vec![],
            legacy_stage_evidence: vec![],
            legacy_pass_evidence: vec![],
        })
        .collect();
    OperationPlan {
        artifact_kind: cam_core::sequence::OPERATION_PLAN_ARTIFACT_KIND.into(),
        schema_version: cam_core::sequence::OPERATION_PLAN_SCHEMA_VERSION,
        engine_version: env!("CARGO_PKG_VERSION").into(),
        job_snapshot: job(),
        input_fingerprint: "input".into(),
        execution_fingerprint: "execution".into(),
        operation_results: results,
        stages,
        motions,
        execution,
        preparation_requirements: vec![],
        generation_diagnostics: vec![PlanIssue {
            code: "none".into(),
            message: String::new(),
            operation_id: None,
            stage_id: None,
        }],
    }
}

fn profile() -> SequenceProfile {
    SequenceProfile {
        schema_version: 2,
        id: "knife-profile".into(),
        work_offset: "G54".into(),
        clearance_z_mm: 5.,
        decimal_places: 3,
        program_start_position_mm: None,
        length_compensation: LengthCompensation::MacroManaged,
        path_control: PathControl::Blend {
            tolerance_mm: 0.01,
            naive_cam_tolerance_mm: None,
        },
        tools: vec![
            SequenceToolMapping {
                tool_id: ENDMILL.into(),
                tool_number: 1,
                length_offset_number: None,
            },
            SequenceToolMapping {
                tool_id: KNIFE.into(),
                tool_number: 3,
                length_offset_number: None,
            },
        ],
        spindle_spinup_seconds: 0.5,
        coolant: Coolant::Flood,
        m6: LinuxCncProfile::from_json(include_str!("../../../../real_data/machine-profile.json"))
            .unwrap()
            .m6,
    }
}

/// Insert a line right before the first motion of the knife stage.
fn inject_before_knife_motion(gcode: &str, line: &str) -> String {
    let mut lines: Vec<String> = gcode.lines().map(str::to_owned).collect();
    let knife_comment = lines
        .iter()
        .position(|l| l.contains("CAM Stage knife"))
        .expect("stage comment");
    let motion_at = (knife_comment + 1..lines.len())
        .find(|i| lines[*i].starts_with("G0 ") || lines[*i].starts_with("G1 "))
        .expect("knife motion");
    lines.insert(motion_at, line.into());
    lines.join("\n") + "\n"
}

#[test]
fn knife_stage_between_milling_stages_runs_spindle_and_coolant_off() {
    let plan = plan();
    let checks = check_plan(&plan).unwrap();
    assert_eq!(checks.status, CheckStatus::Passed, "{:?}", checks.findings);

    // Under a blend/flood profile the knife stage still prepares spindle
    // off, coolant off and exact path; the milling stages keep the profile.
    let trusted = TrustedPlan::from_generated(plan);
    let profile = profile();
    let prepared = PreparedExecution::prepare(&trusted, &profile).unwrap();
    let face_process = &prepared.stages[0].process;
    assert!(matches!(
        face_process.spindle,
        cam_core::post::sequence::PreparedSpindle::On { .. }
    ));
    assert_eq!(face_process.coolant, Coolant::Flood);
    assert_eq!(
        face_process.path_control,
        PathControl::Blend {
            tolerance_mm: 0.01,
            naive_cam_tolerance_mm: None
        }
    );
    let knife_process = &prepared.stages[1].process;
    assert!(matches!(
        knife_process.spindle,
        cam_core::post::sequence::PreparedSpindle::Off
    ));
    assert_eq!(knife_process.coolant, Coolant::Off);
    assert_eq!(knife_process.path_control, PathControl::ExactPath);

    let export = prepared.export(&trusted, &profile).unwrap();
    let gcode = &export.program.gcode;
    let lines: Vec<&str> = gcode.lines().collect();
    let tool_changes: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.contains(" M6"))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        lines.iter().filter(|l| l.contains(" M6")).count(),
        3,
        "one tool change per stage: T1, T3, T1"
    );
    // The knife group: after its M6, no spindle/coolant start appears before
    // the next tool change, and its modal group is exact path (G61) even
    // though the profile blends (G64).
    let knife_m6 = tool_changes[1];
    let next_m6 = tool_changes[2];
    let knife_block = &lines[knife_m6 + 1..next_m6];
    assert!(
        knife_block
            .iter()
            .all(|l| !l.contains("M3") && !l.contains("M4")),
        "tool changes never turn the spindle on for the knife: {knife_block:?}"
    );
    assert!(
        knife_block
            .iter()
            .all(|l| !l.contains("M8") && !l.contains("M7")),
        "coolant stays off during knife motion: {knife_block:?}"
    );
    assert!(
        knife_block.iter().any(|l| l.contains("G61")),
        "the knife stage re-establishes exact path: {knife_block:?}"
    );
    assert!(
        !knife_block.iter().any(|l| l.contains("G64")),
        "no blend tolerance leaks into swivel geometry"
    );
    // Explicit spindle-off and coolant-off follow the knife tool change.
    assert_eq!(lines[knife_m6 + 1], "M5", "explicit spindle off after M6");
    assert_eq!(lines[knife_m6 + 2], "M9", "explicit coolant off after M6");
    // The later milling stage restores its own state.
    let restore_block = &lines[next_m6 + 1..];
    assert!(restore_block.iter().any(|l| l.contains("M3 S10000")));
    assert!(restore_block.iter().any(|l| l.contains("M8")));
    // The emitted bytes verify against the plan through independent readback.
    verify_program(&prepared, &trusted, &export.program).unwrap();
}

#[test]
fn readback_rejects_spindle_or_coolant_running_during_knife_motion() {
    let trusted = TrustedPlan::from_generated(plan());
    let profile = profile();
    let prepared = PreparedExecution::prepare(&trusted, &profile).unwrap();
    let export = prepared.export(&trusted, &profile).unwrap();

    let spindle_on = inject_before_knife_motion(&export.program.gcode, "M3 S1000");
    let error = verify_program(
        &prepared,
        &trusted,
        &cam_core::post::sequence::SequenceProgram {
            filename: export.program.filename.clone(),
            gcode: spindle_on,
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "PROCESS_SPINDLE_STATE");

    let coolant_on = inject_before_knife_motion(&export.program.gcode, "M8");
    let error = verify_program(
        &prepared,
        &trusted,
        &cam_core::post::sequence::SequenceProgram {
            filename: export.program.filename.clone(),
            gcode: coolant_on,
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "PROCESS_COOLANT_STATE");
}

#[test]
fn knife_motions_carry_blade_headings_and_never_mill() {
    // The pivot/tip display contract: every knife-stage motion carries the
    // modeled heading; milling motions never do; knife effects and purposes
    // stay inside knife stages.
    let mut missing = plan();
    missing.motions[4].blade_heading_deg = None;
    let report = check_plan(&missing).unwrap();
    assert_eq!(report.status, CheckStatus::Failed);
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "PLAN_KNIFE_HEADING")
    );

    let mut stray = plan();
    stray.motions[0].blade_heading_deg = Some((0., 0.));
    let report = check_plan(&stray).unwrap();
    assert_eq!(report.status, CheckStatus::Failed);
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "PLAN_KNIFE_HEADING")
    );

    let mut milled = plan();
    milled.motions[5].effect = MotionEffect::MillingSweep;
    let report = check_plan(&milled).unwrap();
    assert_eq!(report.status, CheckStatus::Failed);
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "PLAN_EFFECT_ROLE_MISMATCH")
    );

    let mut traced = plan();
    traced.motions[1].effect = MotionEffect::KnifeTrace;
    traced.motions[1].purpose = MotionPurpose::KnifeCut;
    let report = check_plan(&traced).unwrap();
    assert_eq!(report.status, CheckStatus::Failed);
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == "PLAN_EFFECT_ROLE_MISMATCH")
    );

    // knife_tip derives the visible tip from the pivot and heading (plan
    // sections 12.1/12.5): the section-19.1 corner numbers.
    let close = |a: Point, b: Point| a.distance(b) < 1e-9;
    assert!(close(
        knife_tip(Point::new(11., 0.), 180., 1.),
        Point::new(10., 0.)
    ));
    assert!(close(
        knife_tip(Point::new(10., 1.), 270., 1.),
        Point::new(10., 0.)
    ));
    assert!(close(
        knife_tip(Point::new(21., 0.), 180., 1.),
        Point::new(20., 0.)
    ));
    // Every knife motion of the fixture carries a heading at both ends.
    for motion in &plan().motions[3..9] {
        assert!(
            motion.blade_heading_deg.is_some(),
            "motion {} carries the modeled heading",
            motion.id
        );
    }
}

#[test]
fn the_knife_planner_stays_unadvertised_and_unplanned() {
    // The document accepts knife operations, but no planner is advertised:
    // planning an enabled knife operation is a located rejection, never a
    // silent skip or a partial plan.
    let error = OperationPlan::plan_job(&job(), &PlanLimits::default()).unwrap_err();
    assert_eq!(error.code, "OPERATION_PLANNER_UNAVAILABLE");
    assert!(error.message.contains("knife-1"), "{error}");
    // A disabled knife operation plans the rest of the job.
    let mut mixed = job();
    mixed.operations[1].enabled = false;
    let plan = OperationPlan::plan_job(&mixed, &PlanLimits::default()).unwrap();
    assert_eq!(plan.operation_results.len(), 2);
}
