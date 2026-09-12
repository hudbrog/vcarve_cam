use cam_core::project::v5::{
    self,
    resources::{self as core, AssignmentRole as Role, ProfileStatus},
};
use cam_gui_runtime::{
    app::Document,
    authoring,
    resources::{Catalog, ResourceCommand as R, StoredCatalog},
    session::{self, Command},
};
use cam_service::retained::Retained;
const JOB: &str = include_str!("../../../fixtures/gui4/lettering.job.json");
const LIBRARY: &str = include_str!("../../../fixtures/gui5/library.json");

#[test]
fn page_stock_respects_physical_units_and_placement_without_changing_z_or_assignments() {
    let mut job = authoring::import_svg(
        "inch.svg".into(),
        include_str!("../../../fixtures/gui2/new-carving.svg").into(),
    )
    .unwrap();
    let rect = job.setup.stock.xy.as_ref().unwrap();
    assert!((rect.width_mm - 50.8).abs() < 1e-9);
    assert!((rect.length_mm - 25.4).abs() < 1e-9);
    job.artwork[0].placement.origin_mm = cam_core::geometry::Point { x: 2., y: 3. };
    job.artwork[0].placement.rotation_deg = 90.;
    job.artwork[0].placement.scale = 2.;
    job.setup.stock.thickness_mm = Some(22.);
    let action = R::StockPage {
        item: job.artwork[0].id.clone(),
    };
    assert_eq!(action.clear_fields(&job), vec![40, 41, 42, 43]);
    let captured = action.execute(&job).unwrap();
    let rect = captured.setup.stock.xy.as_ref().unwrap();
    for (actual, expected) in [
        (rect.min_x_mm, -44.8),
        (rect.min_y_mm, -4.),
        (rect.width_mm, 50.8),
        (rect.length_mm, 101.6),
    ] {
        assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
    }
    assert_eq!(captured.setup.stock.thickness_mm, Some(22.));
    assert_eq!(captured.setup.work_zero, job.setup.work_zero);
    assert_eq!(captured.operations, job.operations);
}

#[test]
fn direct_library_tool_selection_copies_geometry_and_clears_only_changed_assignment() {
    let before = job();
    let action = R::SelectLibraryTool {
        catalog: catalog(),
        tool: "endmill".into(),
        operation: "carving".into(),
        role: Role::Endmill,
    };
    assert!(action.clear_fields(&before).contains(&2));
    let after = action.clone().execute(&before).unwrap();
    assert_ne!(
        session::settings(&after).endmill.tool_id,
        session::settings(&before).endmill.tool_id
    );
    assert_eq!(
        session::settings(&after).vbit,
        session::settings(&before).vbit
    );
    assert!(
        session::settings(&after)
            .endmill
            .cutting_feed_mm_min
            .is_none()
    );
    assert!(action.clear_fields(&after).is_empty());
    assert_eq!(action.execute(&after).unwrap(), after);
}

#[test]
fn new_machine_requires_real_contract_details_but_can_be_completed_saved_and_applied() {
    let mut machine = cam_gui_runtime::resources::new_machine("my-router".into());
    assert!(machine.validate_shape().is_err());
    let problems = cam_gui_runtime::resources::machine_problems(&machine);
    assert_eq!(problems.len(), 5, "{problems:?}");
    assert!(problems[0].contains("notes/reference"));
    assert!(machine.tools.is_empty());
    machine.m6 = catalog().machines[0].m6.clone();
    machine.validate_shape().unwrap();
    assert!(cam_gui_runtime::resources::machine_problems(&machine).is_empty());
    let mut library = Catalog::empty("machines".into());
    library.machines.push(machine.clone());
    let saved = StoredCatalog::next(None, library).unwrap();
    saved.validate().unwrap();
    let applied = R::Machine { profile: machine }.execute(&job()).unwrap();
    assert_eq!(
        applied.machine_configuration.unwrap().origin.name,
        "my-router"
    );
}
fn job() -> v5::CamJobV5 {
    session::open(JOB).unwrap()
}
fn catalog() -> Catalog {
    Catalog::decode(LIBRARY).unwrap()
}
fn apply(job: &v5::CamJobV5, role: Role, catalog: Catalog) -> v5::CamJobV5 {
    R::Apply {
        operation: "carving".into(),
        role,
        catalog,
        tool: if role == Role::Endmill {
            "endmill"
        } else {
            "vbit"
        }
        .into(),
        preset: if role == Role::Endmill {
            "rough"
        } else {
            "finish"
        }
        .into(),
    }
    .execute(job)
    .unwrap()
}
#[test]
fn baselines_include_unset_values_reset_offline_and_reapply_is_explicit() {
    let mut lib = catalog();
    lib.library.tools[0].cutting_presets[0].stepover_mm = None;
    let original = job();
    let applied = apply(&original, Role::Endmill, lib.clone());
    assert_eq!(session::settings(&applied).endmill.stepover_mm, None);
    assert_eq!(
        session::settings(&applied).vbit,
        session::settings(&original).vbit
    );
    assert_eq!(
        core::assignment_statuses(&applied)[0].status,
        ProfileStatus::Applied
    );
    let mut edited = applied.clone();
    authoring::settings_mut(&mut edited)
        .endmill
        .cutting_feed_mm_min = Some(999.);
    assert_eq!(
        core::assignment_statuses(&edited)[0].status,
        ProfileStatus::Modified
    );
    let reopened = session::open(&edited.to_json().unwrap()).unwrap();
    let reset = R::Reset {
        operation: "carving".into(),
        role: Role::Endmill,
    }
    .execute(&reopened)
    .unwrap();
    assert_eq!(reset, applied);
    lib.library.revision = 2;
    lib.library.tools[0].cutting_presets[0].cutting_feed_mm_min = Some(1500.);
    assert_ne!(
        session::settings(&reset).endmill.cutting_feed_mm_min,
        Some(1500.)
    );
    let reapplied = R::Reapply {
        operation: "carving".into(),
        role: Role::Endmill,
        catalog: lib.clone(),
    }
    .execute(&reset)
    .unwrap();
    assert_eq!(
        session::settings(&reapplied).endmill.cutting_feed_mm_min,
        Some(1500.)
    );
    assert_eq!(
        session::settings(&reapplied)
            .endmill
            .applied_profile
            .as_ref()
            .unwrap()
            .revision_at_application,
        2
    );
    lib.id = "unrelated-library".into();
    assert!(
        R::Reapply {
            operation: "carving".into(),
            role: Role::Endmill,
            catalog: lib
        }
        .execute(&reset)
        .unwrap_err()
        .contains("source library")
    );
}
#[test]
fn physical_identity_add_use_and_shared_edits_preserve_assignment_boundaries() {
    let original = apply(&job(), Role::Endmill, catalog());
    let (first, id) =
        core::add_library_tool(&original, &catalog().library, &catalog().id, "endmill").unwrap();
    assert_ne!(
        id, "endmill",
        "matching local ID must not overwrite a physical tool"
    );
    assert_eq!(first.job.tools[0], original.tools[0]);
    assert_eq!(first.job.operations, original.operations);
    let (again, same) =
        core::add_library_tool(&first.job, &catalog().library, &catalog().id, "endmill").unwrap();
    assert_eq!(id, same);
    assert_eq!(again.job.tools.len(), first.job.tools.len());
    let mut other = catalog();
    other.id = "other-library".into();
    let (distinct, other_id) =
        core::add_library_tool(&first.job, &other.library, &other.id, "endmill").unwrap();
    assert_ne!(id, other_id);
    assert_eq!(distinct.job.tools.len(), first.job.tools.len() + 1);
    let used = R::UseTool {
        operation: "carving".into(),
        role: Role::Endmill,
        tool: id.clone(),
    }
    .execute(&first.job)
    .unwrap();
    let a = &session::settings(&used).endmill;
    assert_eq!(a.cutting_feed_mm_min, None);
    assert_eq!(a.spindle_direction, None);
    assert!(a.applied_profile.is_none());
    assert_eq!(
        session::settings(&used).vbit,
        session::settings(&original).vbit
    );
    let mut tool = used.tools.iter().find(|t| t.id == id).unwrap().clone();
    tool.name = "Explicitly edited physical tool".into();
    let edited = R::EditTool { tool }.execute(&used).unwrap();
    assert_eq!(edited.operations, used.operations);
    let mut revised = catalog();
    if let cam_core::tool_library::LibraryGeometry::Endmill(g) =
        &mut revised.library.tools[0].geometry
    {
        g.diameter_mm = 3.;
    }
    let (added, new_id) =
        core::add_library_tool(&first.job, &revised.library, &revised.id, "endmill").unwrap();
    assert_ne!(id, new_id);
    assert_eq!(added.job.tools[0], first.job.tools[0]);
    let mut dangling = original.clone();
    dangling.tools.retain(|t| t.id != "endmill");
    let (safe, safe_id) =
        core::add_library_tool(&dangling, &catalog().library, &catalog().id, "endmill").unwrap();
    assert_ne!(safe_id, "endmill");
    assert_eq!(session::settings(&safe.job).endmill.tool_id, "endmill");
}
#[test]
fn library_roundtrip_validation_and_capture_are_independent_of_job() {
    let original = job();
    let mut lib = catalog();
    let t = cam_gui_runtime::resources::capture_tool(
        &original.tools[0],
        "new".into(),
        "Copied geometry".into(),
    )
    .unwrap();
    assert!(t.cutting_presets.is_empty());
    let p = cam_gui_runtime::resources::capture_assignment(
        &original,
        Role::Endmill,
        "captured".into(),
        "Captured".into(),
    )
    .unwrap();
    assert_eq!(
        p.cutting_feed_mm_min,
        session::settings(&original).endmill.cutting_feed_mm_min
    );
    lib.library.tools.push(t);
    lib.library.tools[0].cutting_presets.push(p);
    let saved = StoredCatalog::next(Some(8), lib).unwrap();
    assert_eq!(saved.revision, 9);
    assert_eq!(saved.snapshot.library.revision, 9);
    let json = serde_json::to_string(&saved.snapshot).unwrap();
    assert_eq!(Catalog::decode(&json).unwrap().library.tools.len(), 3);
    assert_eq!(original, job());
    assert!(Catalog::decode(&json.replace("\"schema\":1", "\"schema\":999")).is_err());
}
#[test]
fn shared_geometry_preserves_each_assignment_and_same_tool_preserves_raw_edits() {
    let mut shared = job();
    let mut second = shared.operations[0].clone();
    second.id = "other-carving".into();
    if let v5::OperationSettingsV5::FlatVcarve(s) = &mut second.settings {
        s.endmill.cutting_feed_mm_min = Some(777.);
    }
    shared.operations.push(second);
    let uses = core::assignment_statuses(&shared)
        .into_iter()
        .filter(|s| s.tool_id == "endmill")
        .collect::<Vec<_>>();
    assert_eq!(uses.len(), 2);
    assert_eq!(uses[0].operation_id, "carving");
    assert_eq!(uses[1].operation_id, "other-carving");
    let mut tool = shared.tools[0].clone();
    if let Some(cam_core::project::ToolGeometry::Endmill(g)) = &mut tool.geometry {
        g.diameter_mm = 2.4;
    }
    let edited = R::EditTool { tool }.execute(&shared).unwrap();
    assert_eq!(edited.operations, shared.operations);
    assert_ne!(edited.tools[0], shared.tools[0]);
    let same = R::UseTool {
        operation: "carving".into(),
        role: Role::Endmill,
        tool: "endmill".into(),
    };
    assert!(same.clear_fields(&job()).is_empty());
    let mut doc = Document::new(job());
    assert!(doc.edit(2, "-".into()).is_err());
    let raw = doc.raw.clone();
    doc.job = same.execute(&doc.job).unwrap();
    assert_eq!(doc.raw, raw);
    assert!(doc.pending());
}
#[test]
fn real_resource_values_generate_simulate_export_and_machine_edits_revalidate() {
    let mut job = apply(
        &apply(&job(), Role::Endmill, catalog()),
        Role::Vbit,
        catalog(),
    );
    authoring::set_mode(&mut job, cam_core::project::FlatVcarveMode::Combined);
    let mut service = Retained::new();
    let (generated, _) =
        session::execute(&mut service, Command::generate(job.to_json().unwrap())).unwrap();
    assert_eq!(generated.report["gui2"]["kind"], "generated");
    let handle = generated.report["gui2"]["handle"]
        .as_str()
        .unwrap()
        .to_string();
    let motions = generated.sim.as_ref().unwrap().motions;
    let (stock, payload) = session::execute(
        &mut service,
        Command::Seek {
            handle: handle.clone(),
            prefix: motions,
        },
    )
    .unwrap();
    assert_eq!(stock.report["gui2"]["prefix"], motions);
    assert!(!payload.is_empty());
    let mut machine = catalog().machines.remove(0);
    machine.work_offset = "G55".into();
    job = R::Machine { profile: machine }.execute(&job).unwrap();
    let (check, _) = session::execute(
        &mut service,
        Command::validate_plan(job.to_json().unwrap(), handle.clone()),
    )
    .unwrap();
    assert_eq!(check.report["gui2"]["kind"], "revalidated");
    let (prepared, _) = session::execute(
        &mut service,
        Command::Prepare {
            job: job.to_json().unwrap(),
            handle: handle.clone(),
        },
    )
    .unwrap();
    assert_eq!(prepared.report["gui2"]["kind"], "prepared");
    let opened = session::open(&job.to_json().unwrap()).unwrap();
    assert_eq!(opened, job);
    let (unused, _) =
        core::add_library_tool(&job, &catalog().library, &catalog().id, "endmill").unwrap();
    assert!(
        v5::machine::resolve_sequence_profile(&unused.job, &v5::ReadinessScope::AllEnabled).is_ok()
    );
    let mut bad = job.clone();
    bad.machine_configuration.as_mut().unwrap().tools[1].tool_number = Some(3);
    assert!(
        session::execute(
            &mut service,
            Command::Prepare {
                job: bad.to_json().unwrap(),
                handle
            }
        )
        .is_err()
    );
}
#[test]
fn machine_partial_fields_are_recoverable_and_block_preparation() {
    let mut doc = Document::new(job());
    assert!(doc.edit(35, "-".into()).is_err());
    assert!(doc.pending());
    let mut snapshot = doc.snapshot();
    snapshot.workspace = Default::default();
    assert!(snapshot.validate().is_ok());
    doc.edit(35, "4".into()).unwrap();
    assert!(!doc.pending());
    assert_eq!(
        doc.job
            .machine_configuration
            .as_ref()
            .unwrap()
            .decimal_places,
        Some(4)
    );
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn native_conditional_resource_save_preserves_conflicting_edit_and_last_good_bytes() {
    use cam_gui_runtime::file_io::Store;
    let directory = std::env::temp_dir().join(format!("gui5-resources-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let store = Store::new(directory.clone());
    assert!(store.load_resources().unwrap().is_none());
    let first = store.save_resources(None, catalog()).unwrap();
    let mut stale = first.snapshot.clone();
    stale.library.tools[0].name = "My edits".into();
    store
        .save_resources(Some(first.revision), catalog())
        .unwrap();
    let before = std::fs::read(directory.join("resources.json")).unwrap();
    assert!(
        store
            .save_resources(Some(first.revision), stale.clone())
            .unwrap_err()
            .contains("conflict")
    );
    assert_eq!(stale.library.tools[0].name, "My edits");
    assert_eq!(
        std::fs::read(directory.join("resources.json")).unwrap(),
        before
    );
    let current = store.load_resources().unwrap().unwrap();
    assert_eq!(current.revision, 2);
    store.save_resources(Some(current.revision), stale).unwrap();
    assert_eq!(
        store
            .load_resources()
            .unwrap()
            .unwrap()
            .snapshot
            .library
            .tools[0]
            .name,
        "My edits"
    );
    std::fs::remove_file(directory.join("resources.json")).unwrap();
    std::fs::remove_file(directory.join("session.lock")).unwrap();
    std::fs::remove_dir(directory).unwrap();
}
