//! GUI7 ordered-operation lifecycle: add, delete, enable, reorder and repair
//! through the shared schema-5 operation commands.
use cam_core::project::v5::{OperationSettingsV5, commands};
use cam_gui_runtime::{
    app::Document,
    operation_authoring::{self, Action, Kind},
    session::{self, Command},
};
use cam_service::retained::Retained;

#[test]
fn delete_save_reopen_and_add_preserve_job_context() {
    let original =
        session::open(include_str!("../../../fixtures/gui4/lettering.job.json")).unwrap();
    let mut service = Retained::new();
    let delete = Action::Delete {
        operation_id: original.operations[0].id.clone(),
    };
    let (reply, _) = session::execute(
        &mut service,
        Command::Operation {
            job: original.to_json().unwrap(),
            action: delete,
        },
    )
    .unwrap();
    let empty = session::open(&reply.job).unwrap();
    assert!(empty.operations.is_empty());
    assert_eq!(empty.artwork, original.artwork);
    assert_eq!(empty.setup, original.setup);
    assert_eq!(empty.tools, original.tools);
    assert_eq!(empty.machine_configuration, original.machine_configuration);
    Document::new(empty.clone()).snapshot().validate().unwrap();
    session::execute(
        &mut service,
        Command::Preview {
            job: empty.to_json().unwrap(),
        },
    )
    .unwrap();
    assert!(
        session::execute(&mut service, Command::generate(empty.to_json().unwrap()))
            .unwrap_err()
            .contains("Add an operation")
    );
    let add = operation_authoring::add(Kind::DragKnife, &empty);
    let (reply, _) = session::execute(
        &mut service,
        Command::Operation {
            job: empty.to_json().unwrap(),
            action: add,
        },
    )
    .unwrap();
    let knife = session::open(&reply.job).unwrap();
    assert_eq!(knife.setup, original.setup);
    assert_eq!(knife.machine_configuration, original.machine_configuration);
    for (actual, expected) in knife.artwork.iter().zip(&original.artwork) {
        assert_eq!(actual.content, expected.content);
        assert_eq!(actual.placement, expected.placement);
    }
    let settings = cam_gui_runtime::knife::settings(&knife).unwrap();
    assert!(settings.chains.is_empty());
    assert!(settings.assignment.cutting_feed_mm_min.is_none());
    assert!(knife.tools.last().unwrap().geometry.is_none());
    Document::new(knife.clone()).validate().unwrap();
}

#[test]
fn operations_order_enable_and_dependency_repair_are_explicit() {
    let original =
        session::open(include_str!("../../../fixtures/gui4/lettering.job.json")).unwrap();
    let carving = original.operations[0].id.clone();
    // GUI7 orders operations: a Face operation can now be added before the
    // established carving instead of replacing it.
    let face =
        operation_authoring::apply(&original, operation_authoring::add(Kind::Face, &original))
            .unwrap();
    assert_eq!(face.operations.len(), 2);
    assert_eq!(
        session::kind(&face, "face-1"),
        Some(session::OperationKind::Face)
    );
    assert_eq!(face.operations[0].id, carving);
    // Move the face operation first without touching any other field.
    let ordered = operation_authoring::apply(
        &face,
        Action::Move {
            operation_id: "face-1".into(),
            to_index: 0,
        },
    )
    .unwrap();
    assert_eq!(ordered.operations[0].id, "face-1");
    assert_eq!(ordered.operations[1].id, carving);
    // A carve that references the face plane becomes unresolved when the face
    // is disabled; the located issue names the operation that owns it.
    let bound = commands::set_operation_enabled(&ordered, "face-1", true)
        .unwrap()
        .job;
    let mut bound = bound;
    {
        let OperationSettingsV5::FlatVcarve(settings) = &mut bound
            .operations
            .iter_mut()
            .find(|operation| operation.id == carving)
            .unwrap()
            .settings
        else {
            panic!("carving is a Flat V-carve operation")
        };
        settings.top.reference = cam_core::project::HeightReference::FaceResult {
            operation_id: "face-1".into(),
        };
    }
    bound.validate_structure().unwrap();
    let disabled = commands::set_operation_enabled(&bound, "face-1", false)
        .unwrap()
        .job;
    // The carve's referenced plane is not published when the face is disabled:
    // planning reports an incomplete carve with a located reason instead of
    // silently facing the stock top.
    let plan = cam_core::sequence::OperationPlanV5::plan_job_v5(
        &disabled,
        &cam_core::project::v5::ReadinessScope::AllEnabled,
        &cam_core::sequence::PlanLimits::default(),
    )
    .unwrap();
    let carve_result = plan
        .operation_results
        .iter()
        .find(|result| result.operation_id == carving)
        .unwrap();
    assert_eq!(
        carve_result.generation_status,
        cam_core::sequence::GenerationStatus::Incomplete
    );
    assert!(
        plan.generation_diagnostics.iter().any(|issue| {
            issue.operation_id.as_deref() == Some(carving.as_str())
                && issue.message.contains("face-1")
        }),
        "the carve owns the unresolved face reference: {:?}",
        plan.generation_diagnostics
    );
    assert!(!cam_core::checks::check_plan_v5(&plan).unwrap().export_ready);
    // Enabling it again restores a resolvable dependency without other edits.
    let repaired = commands::set_operation_enabled(&disabled, "face-1", true)
        .unwrap()
        .job;
    let readiness = cam_core::project::v5::references::planning_readiness(
        &repaired,
        &cam_core::project::v5::ReadinessScope::AllEnabled,
    )
    .unwrap();
    assert!(readiness.ready(), "{:?}", readiness.blockers());

    // Delete only the selected operation; the other one stays in order.
    let single = operation_authoring::apply(
        &repaired,
        Action::Delete {
            operation_id: "face-1".into(),
        },
    )
    .unwrap();
    assert_eq!(single.operations.len(), 1);
    assert_eq!(single.operations[0].id, carving);
}

#[test]
fn prefix_generation_and_export_bind_the_selected_scope() {
    let original =
        session::open(include_str!("../../../fixtures/gui4/lettering.job.json")).unwrap();
    let carving = original.operations[0].id.clone();
    // A fully configured face operation after the carving gives two distinct
    // prefixes: the carving alone, and the whole enabled list.
    let mut job =
        operation_authoring::apply(&original, operation_authoring::add(Kind::Face, &original))
            .unwrap();
    {
        let mut candidate = job.clone();
        cam_gui_runtime::authoring::set_group_in(&mut candidate, "face-1", 12, &[3., 8.])
            .expect("face endmill geometry");
        job = candidate;
    }
    for (field, value) in [
        (2, 300.),  // cutting feed
        (10, 100.), // plunge feed
        (11, 10_000.),
        (8, 1.),   // stepdown
        (9, 1.5),  // stepover
        (88, 1.),  // tool stepdown limit
        (75, 0.),  // pass angle
        (87, -1.), // bottom offset: face 1 mm deep
    ] {
        job = cam_gui_runtime::app::set_value(&job, "face-1", field, Some(value))
            .unwrap_or_else(|error| panic!("field {field}: {error}"));
    }
    let face = cam_core::project::v5::commands::set_operation_enabled(&job, "face-1", true)
        .unwrap()
        .job;
    let face_settings = face
        .operations
        .iter()
        .find(|operation| operation.id == "face-1")
        .unwrap();
    assert!(matches!(
        face_settings.settings,
        OperationSettingsV5::Face(_)
    ));
    let mut service = Retained::new();
    let (prefix, _) = session::execute(
        &mut service,
        Command::Generate {
            job: face.to_json().unwrap(),
            scope: session::GenerateScope::ThroughOperation {
                operation_id: carving.clone(),
            },
        },
    )
    .unwrap();
    let report = &prefix.report["gui2"];
    assert_eq!(report["scope"]["kind"], "throughOperation");
    assert_eq!(report["scope"]["operationId"], carving);
    assert_eq!(report["checks"]["exportReady"], true);
    // The prefix excludes the later face operation.
    assert_eq!(report["scopeOperation"], carving);
    let (whole, _) =
        session::execute(&mut service, Command::generate(face.to_json().unwrap())).unwrap();
    assert!(
        whole.motions > prefix.motions,
        "the whole enabled list adds the face stage: {} vs {}",
        whole.motions,
        prefix.motions
    );
    assert_eq!(whole.report["gui2"]["scope"]["kind"], "allEnabled");
    // The prefix exports exactly the prefix, even though the document also
    // carries a later enabled operation.
    let handle = report["handle"].as_str().unwrap().to_owned();
    let (prepared, _) = session::execute(
        &mut service,
        Command::Prepare {
            job: face.to_json().unwrap(),
            handle,
        },
    )
    .unwrap();
    let program = prepared.report["gui2"]["file"]["gcode"].as_str().unwrap();
    assert!(
        program.lines().count() > 10,
        "the prefix export carries a real program"
    );
}
