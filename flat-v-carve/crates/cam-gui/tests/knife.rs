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
fn filled_artwork_offers_its_own_outline_as_a_knife_line() {
    // A filled shape used to need "Create knife outlines" before a knife could
    // cut it, because only strokes were read as lines. Its drawn boundary is a
    // line in its own right, so the knife is offered it directly — no copy, no
    // conversion step.
    let filled = "<svg xmlns='http://www.w3.org/2000/svg' width='20mm' height='20mm' viewBox='0 0 20 20'><path d='M2 2H10V10H2Z'/></svg>";
    let job = knife::import_svg("filled.svg".into(), filled.into()).unwrap();
    let chains = knife::chains(&job).unwrap();
    assert_eq!(chains.len(), 1, "the square's own outline");
    assert!(chains[0].closed);
    // The operation starts with nothing selected: an import never assigns
    // geometry on its own.
    assert!(knife::settings(&job).unwrap().chains.is_empty());
    let chosen = knife::select(&job, &[chains[0].reference.clone()]).unwrap();
    assert_eq!(knife::settings(&chosen).unwrap().chains.len(), 1);

    // Offered is not enough: the line has to cut. Take the configured knife
    // job, swap in filled artwork and plan the shape's own outline.
    let mut configured =
        v5::CamJobV5::from_json(include_str!("../../../fixtures/gui6/knife.job.json")).unwrap();
    configured.artwork = vec![v5::ArtworkItem {
        id: v5::ArtworkItemId("artwork-1".into()),
        name: "square.svg".into(),
        content: v5::ArtworkContent::Svg(cam_core::job::SourceSnapshot {
            filename: "square.svg".into(),
            svg: "<svg xmlns='http://www.w3.org/2000/svg' width='20mm' height='20mm' viewBox='0 0 20 20'><path id='square' d='M4 4H16V16H4Z'/></svg>".into(),
        }),
        import_settings: v5::SvgInterpretation::default(),
        placement: Default::default(),
    }];
    configured.operations[0].settings = {
        let v5::OperationSettingsV5::DragKnife(mut settings) =
            configured.operations[0].settings.clone()
        else {
            unreachable!("the fixture is a knife job")
        };
        settings.chains.clear();
        v5::OperationSettingsV5::DragKnife(settings)
    };
    let outline = knife::chains(&configured)
        .unwrap()
        .into_iter()
        .find(|chain| chain.closed)
        .expect("the filled square's outline");
    let cut = knife::select(&configured, &[outline.reference]).unwrap();
    let mut service = Retained::new();
    let (meta, _) =
        session::execute(&mut service, Command::generate(cut.to_json().unwrap())).unwrap();
    assert_eq!(meta.report["gui2"]["checks"]["exportReady"], true);
    assert!(meta.motions > 0, "the outline must produce cutting motion");

    let flower =
        v5::CamJobV5::from_json(include_str!("../../../fixtures/gui6/flower-knife.job.json"))
            .unwrap();
    assert_eq!(knife::settings(&flower).unwrap().chains.len(), 1);
    let mut service = Retained::new();
    let (meta, _) =
        session::execute(&mut service, Command::generate(flower.to_json().unwrap())).unwrap();
    // 813 moves before the knife's path simplification landed, 267 after it:
    // the merge spends the declared tolerance on fewer, longer moves. The number
    // is pinned so a further change to the merge has to say so here.
    assert_eq!(meta.motions, 267);
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
    // The knife's swivels are arcs, and an arc travels inside one motion: the
    // display's stream stays one motion per plan motion, which is what keeps the
    // stage spans, the headings and the paths on the same index.
    assert_eq!(
        meta.sim.as_ref().map(|sim| sim.motions),
        Some(meta.motions),
        "the display stream must stay one motion per plan motion"
    );
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
    // A knife stage still has machine time: its cutting, plunge and swivel
    // feeds all reach the simulator stream, so the transport can play it at
    // the speed the holder will actually move instead of stepping motions.
    let input = scene.sim_input().unwrap().unwrap();
    let knife_feeds: Vec<f64> = input
        .motions
        .iter()
        .filter_map(|motion| motion.feed_mm_min)
        .collect();
    for expected in [150., 50., 75.] {
        assert!(
            knife_feeds
                .iter()
                .any(|feed| (*feed - expected).abs() < 1e-6),
            "the {expected} mm/min feed reaches the stream: {:?}",
            {
                let mut feeds = knife_feeds.clone();
                feeds.sort_by(f64::total_cmp);
                feeds.dedup();
                feeds
            }
        );
    }
    let table = cam_gui_runtime::sim::TimeTable::build(&input.motions, None).unwrap();
    assert!(
        table.total_seconds() > 0.,
        "a knife program has modeled motion time"
    );
    assert_eq!(table.motions(), input.motions.len());
    // The machine checks run for a knife stage too: its cuts are feeds and its
    // lifted moves are rapids, so a holder driven into the sheet beside the blade
    // would be reported here. This fixture states no assembly, and a knife that
    // removes no material cannot be inside any, so the report is empty.
    let warnings = scene.meta.report["gui2"]["warnings"]
        .as_array()
        .expect("the knife scene publishes its machine warnings");
    assert!(warnings.is_empty(), "{warnings:?}");
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
