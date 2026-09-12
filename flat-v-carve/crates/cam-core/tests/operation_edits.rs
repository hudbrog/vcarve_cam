//! GUI7c: the shared schema-5 operation-list commands. Ordering, enablement
//! and creation have exactly one implementation, so the desktop and browser
//! clients cannot diverge from the engine's own document validation.
use cam_core::project::HeightReference;
use cam_core::project::v5::{
    self, CamJobV5, OperationSettingsV5,
    commands::{self, NewOperationKind},
    references::ReadinessScope,
};
use cam_core::sequence::{GenerationStatus, OperationPlanV5, PlanLimits};

const JOB: &str = include_str!("../../../fixtures/gui4/lettering.job.json");

fn job() -> CamJobV5 {
    v5::CamJobV5::from_json(JOB).unwrap()
}

#[test]
fn add_move_enable_rename_and_delete_preserve_everything_else() {
    let original = job();
    let carving = original.operations[0].id.clone();
    let with_face = commands::add_operation(&original, NewOperationKind::Face, "face-1", "Face")
        .unwrap()
        .job;
    assert_eq!(with_face.operations.len(), 2);
    assert_eq!(with_face.operations[0].id, carving);
    assert_eq!(with_face.artwork, original.artwork);
    assert_eq!(with_face.setup, original.setup);
    // The face operation carries its own unset tool snapshot; nothing about
    // the existing endmill/vbit assignments changed.
    let removed = commands::remove_operation(&with_face, "face-1")
        .unwrap()
        .job;
    assert_eq!(removed.operations, original.operations);
    assert_eq!(removed.artwork, original.artwork);
    assert_eq!(
        removed.machine_configuration,
        original.machine_configuration
    );
    assert_eq!(
        &removed.tools[..original.tools.len()],
        &original.tools[..],
        "existing tool snapshots are untouched"
    );
    let moved = commands::move_operation(&with_face, "face-1", 0)
        .unwrap()
        .job;
    assert_eq!(
        moved
            .operations
            .iter()
            .map(|operation| operation.id.as_str())
            .collect::<Vec<_>>(),
        vec!["face-1", carving.as_str()]
    );
    let disabled = commands::set_operation_enabled(&moved, "face-1", false)
        .unwrap()
        .job;
    assert!(!disabled.operations[0].enabled);
    let renamed = commands::rename_operation(&disabled, "face-1", "Rough face")
        .unwrap()
        .job;
    assert_eq!(renamed.operations[0].name, "Rough face");
    let duplicated = commands::duplicate_operation(&renamed, "face-1", "face-2")
        .unwrap()
        .job;
    assert_eq!(duplicated.operations.len(), 3);
    assert_eq!(duplicated.operations[1].id, "face-2");
    assert_eq!(
        duplicated.operations[1].settings, renamed.operations[0].settings,
        "a duplicate keeps its source's settings"
    );
    // Out-of-range moves and unknown IDs are refused without touching the job.
    assert!(commands::move_operation(&with_face, "face-1", 5).is_err());
    assert!(commands::move_operation(&with_face, "missing", 0).is_err());
    assert!(commands::remove_operation(&with_face, "missing").is_err());
    assert!(commands::add_operation(&with_face, NewOperationKind::Face, "face-1", "Face").is_err());
}

#[test]
fn a_reordered_height_dependency_is_saveable_and_reported() {
    let original = job();
    let carving = original.operations[0].id.clone();
    let with_face = commands::add_operation(&original, NewOperationKind::Face, "face-1", "Face")
        .unwrap()
        .job;
    // Bind the carve to the face plane, then disable the face: the document
    // stays structurally valid and the carve plans incomplete with a located
    // reason instead of silently facing the stock top.
    let mut bound = with_face.clone();
    {
        let OperationSettingsV5::FlatVcarve(settings) = &mut bound
            .operations
            .iter_mut()
            .find(|operation| operation.id == carving)
            .unwrap()
            .settings
        else {
            panic!("carving")
        };
        settings.top.reference = HeightReference::FaceResult {
            operation_id: "face-1".into(),
        };
    }
    bound.validate_structure().unwrap();
    let disabled = commands::set_operation_enabled(&bound, "face-1", false)
        .unwrap()
        .job;
    assert!(disabled.validate_structure().is_ok());
    // The dependency is legal while both are ordered and enabled.
    let ordered = commands::set_operation_enabled(&with_face, "face-1", true)
        .unwrap()
        .job;
    let mut ordered = ordered;
    {
        let OperationSettingsV5::FlatVcarve(settings) = &mut ordered
            .operations
            .iter_mut()
            .find(|operation| operation.id == carving)
            .unwrap()
            .settings
        else {
            panic!("carving")
        };
        settings.top.reference = HeightReference::FaceResult {
            operation_id: "face-1".into(),
        };
    }
    ordered.validate_structure().unwrap();
    let plan = OperationPlanV5::plan_job_v5(
        &ordered,
        &ReadinessScope::AllEnabled,
        &PlanLimits::default(),
    )
    .unwrap();
    // The face itself is incomplete (no settings yet), so the carve sees no
    // published plane; the point is the located reason, not a silent default.
    assert!(
        plan.operation_results
            .iter()
            .any(|result| { result.generation_status != GenerationStatus::Complete })
    );
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|issue| issue.operation_id.as_deref() == Some(carving.as_str())),
        "{:?}",
        plan.generation_diagnostics
    );
}

#[test]
fn created_operations_leave_machining_values_unset() {
    let empty = v5::CamJobV5 {
        schema_version: 5,
        name: "Ordered operations".into(),
        setup: Default::default(),
        artwork: vec![],
        tools: vec![],
        operations: vec![],
        tolerances: Default::default(),
        machine_configuration: None,
        legacy_machine_profile: None,
    };
    for (kind, prefix) in [
        (NewOperationKind::Face, "endmill"),
        (NewOperationKind::DragKnife, "knife-tool"),
    ] {
        let outcome = commands::add_operation(&empty, kind, "op-1", "New operation").unwrap();
        let tool = outcome
            .job
            .tools
            .iter()
            .find(|tool| tool.id.starts_with(prefix))
            .expect("a tool snapshot is created");
        assert!(tool.geometry.is_none(), "no cutter geometry is invented");
        match &outcome.job.operations[0].settings {
            OperationSettingsV5::Face(settings) => {
                assert!(settings.assignment.cutting_feed_mm_min.is_none());
                assert!(settings.assignment.spindle_direction.is_none());
                assert!(settings.stepdown_mm.is_none());
            }
            OperationSettingsV5::DragKnife(settings) => {
                assert!(settings.assignment.cutting_feed_mm_min.is_none());
                assert!(settings.stepdown_mm.is_none());
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    let vcarve = commands::add_operation(&empty, NewOperationKind::FlatVcarve, "op-1", "Carve")
        .unwrap()
        .job;
    let OperationSettingsV5::FlatVcarve(settings) = &vcarve.operations[0].settings else {
        panic!("flat v-carve")
    };
    assert!(settings.components.is_empty());
    assert!(settings.max_depth_mm.is_none());
    assert_eq!(vcarve.tools.len(), 2, "endmill and V-bit target");
}
