use cam_core::project::{
    self,
    v5::{self, OperationSettingsV5},
};
use cam_gui_runtime::{
    app::Document,
    compute, knife,
    session::{self, Command},
};
use cam_service::retained::Retained;
use serde_json::json;
const SVG: &str = include_str!("../../../fixtures/gui6/chains.svg");

#[test]
fn knife_start_uses_qualified_source_anchor_and_core_planning() {
    let job = fixture();
    let chains = knife::chains(&job).unwrap();
    let closed = chains.iter().find(|c| c.closed).unwrap();
    let anchored = knife::set_start(&job, Some(&closed.reference), 0.5).unwrap();
    assert_ne!(
        knife::settings(&anchored).unwrap().start,
        knife::settings(&job).unwrap().start
    );
    let mut service = Retained::new();
    let (meta, _) =
        session::execute(&mut service, Command::generate(anchored.to_json().unwrap())).unwrap();
    assert_eq!(meta.report["gui2"]["checks"]["exportReady"], true);
    let open = chains.iter().find(|c| !c.closed).unwrap();
    assert!(knife::set_start(&job, Some(&open.reference), 0.5).is_err());
    assert!(knife::set_start(&job, Some(&closed.reference), 1.0).is_err());
    assert_eq!(knife::set_start(&anchored, None, 0.).unwrap(), job);
}

#[test]
fn filled_regions_require_explicit_centerline_source() {
    let filled = "<svg xmlns='http://www.w3.org/2000/svg' width='20mm' height='20mm' viewBox='0 0 20 20'><path d='M2 2H10V10H2Z'/></svg>";
    assert!(
        knife::import_svg("filled.svg".into(), filled.into())
            .unwrap_err()
            .contains("fill=none")
    );
    let flower =
        v5::CamJobV5::from_json(include_str!("../../../fixtures/gui6/flower-knife.job.json"))
            .unwrap();
    assert_eq!(knife::settings(&flower).unwrap().chains.len(), 1);
    let mut service = Retained::new();
    let (meta, _) =
        session::execute(&mut service, Command::generate(flower.to_json().unwrap())).unwrap();
    assert_eq!(meta.motions, 813);
    assert_eq!(meta.report["gui2"]["checks"]["exportReady"], true);
    assert!(
        meta.stock
            .unwrap()
            .frames
            .iter()
            .all(|f| f.stats.removed_volume_mm3 == 0.)
    );
}

#[test]
fn knife_cross_source_selection_does_not_rebind_after_replacement() {
    let mut service = Retained::new();
    let original = fixture();
    let selected = knife::settings(&original).unwrap().chains.clone();
    let (added, _) = session::execute(
        &mut service,
        Command::Artwork {
            job: original.to_json().unwrap(),
            operation_id: "knife".into(),
            action: session::ArtworkCommand::Add {
                filename: "second.svg".into(),
                svg: SVG.into(),
            },
        },
    )
    .unwrap();
    let added = session::open(&added.job).unwrap();
    assert_eq!(knife::settings(&added).unwrap().chains, selected);
    let choices = knife::chains(&added).unwrap();
    assert_eq!(choices.len(), 4);
    let assigned = knife::select(
        &added,
        &choices
            .iter()
            .map(|c| c.reference.clone())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert_eq!(knife::settings(&assigned).unwrap().chains.len(), 4);
    let (replaced, _) = session::execute(
        &mut service,
        Command::Artwork {
            job: assigned.to_json().unwrap(),
            operation_id: "knife".into(),
            action: session::ArtworkCommand::Replace {
                item: original.artwork[0].id.clone(),
                filename: "changed.svg".into(),
                svg: SVG.replace("15", "16"),
            },
        },
    )
    .unwrap();
    let replaced = session::open(&replaced.job).unwrap();
    assert!(knife::select(&replaced, &selected).is_err());
    assert_eq!(knife::settings(&replaced).unwrap().chains.len(), 4);
}

fn fixture() -> v5::CamJobV5 {
    let job = knife::import_svg("chains.svg".into(), SVG.into()).unwrap();
    let chains = knife::chains(&job).unwrap();
    let mut job = knife::select(
        &job,
        &chains
            .iter()
            .map(|c| c.reference.clone())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    job.setup.stock.thickness_mm = Some(2.);
    job.setup.clearance_above_stock_mm = Some(5.);
    job.setup.start_xy_mm = Some(cam_core::geometry::Point::new(0., 0.));
    job.tools[0].geometry = Some(project::ToolGeometry::DragKnife(project::DragKnifeSpec {
        blade_offset_mm: 1.,
        max_cut_depth_mm: 2.,
    }));
    let OperationSettingsV5::DragKnife(s) = &mut job.operations[0].settings else {
        unreachable!()
    };
    s.assignment.cutting_feed_mm_min = Some(150.);
    s.assignment.plunge_feed_mm_min = Some(50.);
    s.assignment.swivel_feed_mm_min = Some(75.);
    s.assignment.max_stepdown_mm = Some(1.);
    s.stepdown_mm = Some(1.);
    s.swivel_depth_mm = Some(0.5);
    s.corner_threshold_deg = Some(20.);
    s.bottom.offset_mm = -1.;
    s.alignment.initial_heading_deg = Some(180.);
    let mut profile =
        cam_core::post::sequence::SequenceProfile::from_json(session::PROFILE).unwrap();
    profile.tools[0].tool_id = "knife-tool".into();
    profile.tools.truncate(1);
    job = v5::machine::apply_machine_configuration(&job, &profile, "GUI6 fixture")
        .unwrap()
        .job;
    job
}

#[test]
fn knife_import_keeps_open_closed_chains_explicit_and_incomplete() {
    let job = knife::import_svg("chains.svg".into(), SVG.into()).unwrap();
    let chains = knife::chains(&job).unwrap();
    assert!(chains.iter().any(|c| c.closed));
    assert!(chains.iter().any(|c| !c.closed));
    assert!(knife::settings(&job).unwrap().chains.is_empty());
    assert!(job.tools[0].geometry.is_none());
    assert!(job.setup.stock.thickness_mm.is_none());
    assert!(
        knife::settings(&job)
            .unwrap()
            .alignment
            .initial_heading_deg
            .is_none()
    );
    assert_eq!(session::open(&job.to_json().unwrap()).unwrap(), job);
    let mut doc = Document::new(job);
    assert!(!doc.pending());
    assert!(doc.edit(61, "1.".into()).is_err());
    assert!(doc.pending());
    doc.edit(61, "1".into()).unwrap_err(); // second geometry field still absent
    doc.edit(62, "2".into()).unwrap();
    assert!(!doc.pending());
    let before = doc.job.clone();
    assert!(doc.edit(72, "-".into()).is_err());
    assert_eq!(doc.job, before);
    assert!(doc.pending());
    let stored = serde_json::to_string(&doc.snapshot()).unwrap();
    assert!(stored.contains("Initial heading"));
}

#[test]
fn knife_execution_preserves_stock_and_replays_exact_checked_bytes() {
    let job = fixture();
    let text = job.to_json().unwrap();
    let mut service = Retained::new();
    let (meta, payload) =
        session::execute(&mut service, session::Command::generate(text.clone())).unwrap();
    assert!(meta.motions > 10, "{}", meta.report);
    assert_eq!(meta.report["gui2"]["checks"]["exportReady"], true);
    assert!(
        !meta.report["gui2"]["knifeMotions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let stock = meta.stock.as_ref().unwrap();
    for frame in &stock.frames {
        assert_eq!(frame.stats.removed_volume_mm3, 0.);
        assert_eq!(frame.checksum, stock.frames[0].checksum);
    }
    let handle = meta.report["gui2"]["handle"].as_str().unwrap().to_owned();
    let scene = compute::Scene {
        meta,
        payload: std::sync::Arc::new(payload),
    };
    assert!(matches!(
        scene.sim_input().unwrap().unwrap().tools[0],
        cam_gui_runtime::sim::ToolSpec::Knife { .. }
    ));
    for prefix in [scene.motion_count() / 2, 0, scene.motion_count()] {
        let (seek, _) = session::execute(
            &mut service,
            Command::Seek {
                handle: handle.clone(),
                prefix,
            },
        )
        .unwrap();
        assert_eq!(seek.stock.unwrap().frames[0].stats.removed_volume_mm3, 0.);
    }
    let (output, _) = session::execute(
        &mut service,
        Command::Prepare {
            job: text.clone(),
            handle: handle.clone(),
        },
    )
    .unwrap();
    let r = &output.report["gui2"];
    let e = &r["bundle"]["report"]["knifeEvidence"];
    assert_eq!(e["status"], "within");
    assert_eq!(e["programSha256"], r["file"]["sha256"]);
    assert_eq!(
        compute::hash(r["file"]["gcode"].as_str().unwrap().as_bytes()),
        e["programSha256"]
    );
    assert_eq!(r["retained"]["plansRun"], 1);
    assert_eq!(session::open(&output.job).unwrap(), job);
    let mut changed = job.clone();
    changed
        .machine_configuration
        .as_mut()
        .unwrap()
        .decimal_places = Some(4);
    let (next, _) = session::execute(
        &mut service,
        Command::Prepare {
            job: changed.to_json().unwrap(),
            handle: handle.clone(),
        },
    )
    .unwrap();
    assert_eq!(next.report["gui2"]["retained"]["plansRun"], 1);
    assert_eq!(
        next.report["gui2"]["bundle"]["report"]["knifeEvidence"]["outputDecimalPlaces"],
        4
    );
    changed.setup.work_zero.xy = project::WorkZeroXY::CustomPoint { x_mm: 1., y_mm: 2. };
    let (next, _) = session::execute(
        &mut service,
        Command::Prepare {
            job: changed.to_json().unwrap(),
            handle,
        },
    )
    .unwrap();
    assert_eq!(
        next.report["gui2"]["bundle"]["report"]["knifeEvidence"]["machineOffsetMm"],
        json!([1., 2., 0.])
    );
}
