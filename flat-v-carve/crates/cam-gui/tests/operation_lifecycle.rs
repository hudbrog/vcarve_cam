use cam_gui_runtime::{
    app::Document,
    operation_authoring::{self, Action},
    session::{self, Command},
};
use cam_service::retained::Retained;

#[test]
fn delete_save_reopen_add_knife_preserves_job_context() {
    let original =
        session::open(include_str!("../../../fixtures/gui4/lettering.job.json")).unwrap();
    let mut service = Retained::new();
    let (reply, _) = session::execute(
        &mut service,
        Command::Operation {
            job: original.to_json().unwrap(),
            action: Action::Delete,
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
        session::execute(
            &mut service,
            Command::Generate {
                job: empty.to_json().unwrap()
            }
        )
        .unwrap_err()
        .contains("Add an operation")
    );
    let (reply, _) = session::execute(
        &mut service,
        Command::Operation {
            job: empty.to_json().unwrap(),
            action: Action::AddKnife,
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
    assert!(operation_authoring::apply(&knife, Action::AddVcarve).is_err());
    let empty = operation_authoring::apply(&knife, Action::Delete).unwrap();
    let second = operation_authoring::apply(&empty, Action::AddKnife).unwrap();
    assert_eq!(
        cam_gui_runtime::knife::settings(&second)
            .unwrap()
            .assignment
            .tool_id,
        "knife-tool-2"
    );
    assert_eq!(&second.tools[..knife.tools.len()], knife.tools);
    let milling = operation_authoring::apply(&empty, Action::AddVcarve).unwrap();
    assert!(session::carving(&milling).unwrap().components.is_empty());
    Document::new(milling).validate().unwrap();
}
