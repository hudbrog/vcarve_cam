use cam_gui_runtime::{
    app::{Document, set_value},
    compute,
    recovery::Stored,
    session::{self as gui, Command},
};
use cam_service::{collection::CollectionCommand as C, retained::Retained};
use serde_json::json;

fn profile(job: &str) -> String {
    let (meta, _) = gui::execute(
        &mut Retained::new(),
        Command::ApplyProfile {
            job: job.into(),
            json: gui::PROFILE.into(),
        },
    )
    .unwrap();
    meta.job
}

#[test]
fn canonical_flower_retains_exact_execution_and_prepares_without_replanning() {
    let input = profile(gui::FLOWER);
    let mut service = Retained::new();
    let (scene, payload) = gui::execute(&mut service, Command::generate(input.clone())).unwrap();
    assert_eq!(scene.motions, 22_883);
    assert_eq!(scene.rough_vertices / 2, 7_048);
    assert!(scene.stock.is_some());
    assert!(scene.sim.is_some());
    let handle = scene.report["gui2"]["handle"].as_str().unwrap().to_owned();
    let plan = service.generated_plan(&handle).unwrap();
    // The scene consumes this very execution, including both cumulative stages.
    let scene = compute::Scene {
        meta: scene,
        payload: std::sync::Arc::new(payload),
    };
    let sim = scene.sim_input().unwrap().unwrap();
    // Real worker replay, including backward seeks, agrees with cold replay.
    for prefix in [7_048, 22_883, 1_234, 17_111, 0] {
        let (meta, payload) = gui::execute(
            &mut service,
            Command::Seek {
                handle: handle.clone(),
                prefix,
            },
        )
        .unwrap();
        let mut cold = cam_gui_runtime::sim::Field::new(
            sim.stock,
            &sim.tools,
            scene.meta.stock.as_ref().unwrap().cell_mm,
        )
        .unwrap();
        for motion in &sim.motions[..prefix] {
            cold.apply(motion, 0., 1.).unwrap();
        }
        let response = compute::Scene {
            meta,
            payload: std::sync::Arc::new(payload),
        };
        assert_eq!(response.stock_cells(0).unwrap(), cold.packed_tile_bytes());
    }
    for (display, motion) in sim.motions.iter().zip(&plan.trusted.plan().motions) {
        assert_eq!(
            (display.x0, display.y0, display.z0),
            (motion.start.x, motion.start.y, motion.start.z)
        );
        assert_eq!(
            (display.x1, display.y1, display.z1),
            (motion.end.x, motion.end.y, motion.end.z)
        );
    }
    let (prepared, _) = gui::execute(
        &mut service,
        Command::Prepare {
            job: input.clone(),
            handle: handle.clone(),
        },
    )
    .unwrap();
    let output = &prepared.report["gui2"];
    assert_eq!(output["retained"]["plansRun"], 1);
    let bytes = output["file"]["gcode"].as_str().unwrap();
    assert!(!bytes.is_empty());
    assert_eq!(compute::hash(bytes.as_bytes()), output["file"]["sha256"]);
    let reread = service
        .execute(C::ReadPreparedBytes {
            bundle_handle: output["bundle"]["bundleHandle"].as_str().unwrap().into(),
            filename: output["file"]["filename"].as_str().unwrap().into(),
        })
        .unwrap();
    assert_eq!(reread["file"], output["file"]);
    let reopened = gui::open(&prepared.job).unwrap();
    assert_eq!(reopened, gui::open(&input).unwrap());
    let operation = reopened.operations[0].id.clone();
    let changed = set_value(&reopened, &operation, 2, Some(710.)).unwrap();
    let stale = gui::execute(
        &mut service,
        Command::Prepare {
            job: changed.to_json().unwrap(),
            handle,
        },
    )
    .unwrap_err();
    assert!(stale.contains("RETAINED_PLAN_STALE"), "{stale}");
}

#[test]
fn real_edits_pending_text_recovery_and_unsupported_documents() {
    let mut document = Document::new(gui::open(gui::FLOWER).unwrap());
    let original = document.job.clone();
    assert!(document.edit(0, "-".into()).is_err());
    assert!(document.pending());
    assert_eq!(document.job, original);
    let stored = Stored {
        revision: 1,
        snapshot: document.snapshot(),
    };
    let restored = Stored::decode(&serde_json::to_string(&stored).unwrap()).unwrap();
    assert_eq!(restored.snapshot, stored.snapshot);
    document.edit(0, "2.1".into()).unwrap();
    assert!(!document.pending());
    assert_eq!(gui::settings(&document.job).max_depth_mm, Some(2.1));
    document.edit(2, "750".into()).unwrap();
    assert_eq!(
        gui::settings(&document.job).endmill.cutting_feed_mm_min,
        Some(750.)
    );
    // Anything that is not a schema-5 document is refused by name; nothing is
    // converted (compatibility policy, plan §0).
    assert!(
        gui::open(r#"{"schema_version":4,"name":"older","tools":[],"operations":[]}"#).is_err(),
        "the workspace reads schema-5 documents only"
    );
    let mut collection = original.clone();
    let mut extra = collection.operations[0].clone();
    extra.id = "extra".into();
    collection.operations.push(extra);
    // GUI7 orders multiple supported operations; the envelope is the kind and
    // count of operations, not "exactly one".
    let ordered = gui::open(&collection.to_json().unwrap()).unwrap();
    assert_eq!(ordered.operations.len(), 2);
    let mut beyond = collection.clone();
    beyond.operations.truncate(1);
    while beyond.operations.len() <= cam_gui_runtime::operation_authoring::MAX_OPERATIONS {
        let mut duplicate = beyond.operations[0].clone();
        duplicate.id = format!("op-{}", beyond.operations.len());
        beyond.operations.push(duplicate);
    }
    assert!(
        gui::open(&beyond.to_json().unwrap())
            .unwrap_err()
            .contains("up to")
    );
    let mut future = json!(original);
    future["schema_version"] = json!(6);
    assert!(gui::open(&future.to_string()).is_err());
}

#[test]
fn profile_is_explicit_portable_and_old_profile_refused() {
    let job = gui::open(gui::FLOWER).unwrap();
    assert!(job.machine_configuration.is_none());
    let applied = gui::open(&profile(gui::FLOWER)).unwrap();
    assert_eq!(
        applied.machine_configuration.as_ref().unwrap().tools.len(),
        2
    );
    assert_eq!(gui::open(&applied.to_json().unwrap()).unwrap(), applied);
    assert!(
        gui::execute(
            &mut Retained::new(),
            Command::ApplyProfile {
                job: gui::FLOWER.into(),
                json: include_str!("../../../../real_data/machine-profile.json").into()
            }
        )
        .is_err()
    );
}

#[test]
fn late_completion_never_replaces_a_newer_draft() {
    let mut app = cam_gui_runtime::app::App::default();
    app.document = Some(Document::new(gui::open(gui::FLOWER).unwrap()));
    app.revision = 12;
    let ctx = egui::Context::default();
    let result = gui::execute(
        &mut Retained::new(),
        Command::Open {
            json: profile(gui::FLOWER),
        },
    );
    app.accept(999, result, &ctx);
    assert_eq!(app.revision, 12);
    assert!(app.document.unwrap().job.machine_configuration.is_none());
}
