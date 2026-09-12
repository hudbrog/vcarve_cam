#[path = "support/sim_setup.rs"]
mod sim_setup;
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
    let (scene, payload) =
        gui::execute(&mut service, Command::Generate { job: input.clone() }).unwrap();
    assert_eq!(scene.motions, 22_883);
    assert_eq!(scene.rough_vertices / 2, 7_048);
    assert!(scene.stock.is_some());
    assert!(scene.sim.is_some());
    let handle = scene.report["gui2"]["handle"].as_str().unwrap().to_owned();
    let plan = service.generated_plan(&handle).unwrap();
    let original = cam_core::job::Job::from_json(include_str!(
        "../../../../real_data/flower_box-svg.job-real.json"
    ))
    .unwrap();
    let legacy = cam_core::vcarve::plan_combined_with_receipt(&original)
        .unwrap()
        .0;
    let old = legacy.endmill.motions.iter().chain(&legacy.vbit_motions);
    for (old, new) in old.zip(&plan.trusted.plan().motions) {
        assert_eq!(old.start, new.start);
        assert_eq!(old.end, new.end);
        assert_eq!(old.feed_mm_min, new.feed_mm_min);
        assert_eq!(old.tool_id, new.tool_id);
        assert_eq!(old.kind.cutting(), new.is_cutting());
    }
    // The scene consumes this very execution, including both cumulative stages.
    let scene = compute::Scene {
        meta: scene,
        payload: std::sync::Arc::new(payload),
    };
    let sim = scene.sim_input().unwrap().unwrap();
    let slices = cam_service::inspection::Inspection::combined(&legacy)
        .slices
        .into_iter()
        .map(|s| s.info)
        .collect::<Vec<_>>();
    let mut reference = sim_setup::build(&original, &legacy, &slices).unwrap();
    // Match physical bounds and cell policy; the old automatic box and the
    // canonical artwork-derived automatic box differ slightly.
    reference.stock = sim.stock;
    reference.resolution = sim.resolution;
    let reference = cam_gui_runtime::stock_preview::build(&reference, 7_048).unwrap();
    for (index, frame) in reference.meta.frames.iter().enumerate() {
        assert_eq!(
            frame.checksum,
            scene.meta.stock.as_ref().unwrap().frames[index].checksum
        );
        assert_eq!(
            reference.cells[index].as_slice(),
            scene.stock_cells(index).unwrap()
        );
    }
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
    let changed = set_value(&reopened, 2, Some(710.)).unwrap();
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
    assert!(
        gui::open(include_str!(
            "../../../../real_data/flower_box-svg.job-real.json"
        ))
        .is_err(),
        "GUI2 is schema-5 only"
    );
    let mut collection = original.clone();
    let mut extra = collection.operations[0].clone();
    extra.id = "extra".into();
    collection.operations.push(extra);
    assert!(
        gui::open(&collection.to_json().unwrap())
            .unwrap_err()
            .contains("one Flat V-carve")
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
