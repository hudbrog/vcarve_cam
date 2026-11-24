use cam_core::project::v5;
use cam_gui_runtime::{
    app::Document,
    compute, knife,
    operation_authoring::{self, Kind},
    session::{self, ArtworkCommand, Command},
    sim::ToolSpec,
};
use cam_service::retained::Retained;

#[test]
fn editor_selection_and_generated_preview_keep_pen_down_at_the_join() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="20mm" viewBox="0 0 20 20"><path id="a" fill="none" stroke="black" d="M1 1L10 1"/><path id="b" fill="none" stroke="black" d="M10 1L10 9"/></svg>"#;
    let job = v5::authoring::from_svg("drawing.svg".into(), svg.into(), 0.001).unwrap();
    let job =
        operation_authoring::apply(&job, operation_authoring::add(Kind::CenterlinePlot, &job))
            .unwrap();
    let mut doc = Document::new(job);
    doc.edit(2, "300".into()).unwrap();
    doc.edit(10, "80".into()).unwrap();
    doc.edit(126, "0.2".into()).unwrap();
    let refs = knife::chains(&doc.job)
        .unwrap()
        .into_iter()
        .map(|c| c.reference)
        .collect();
    let mut service = Retained::new();
    let (bound, _) = session::execute(
        &mut service,
        Command::Artwork {
            job: doc.job.to_json().unwrap(),
            operation_id: doc.raw.operation.clone(),
            action: ArtworkCommand::PlotSelection { references: refs },
        },
    )
    .unwrap();
    let job = session::open(&bound.job).unwrap();
    let (meta, payload) =
        session::execute(&mut service, Command::generate(job.to_json().unwrap())).unwrap();
    assert_eq!(
        meta.report["gui2"]["checks"]["exportReady"], true,
        "{}",
        meta.report
    );
    let scene = compute::Scene {
        meta,
        payload: std::sync::Arc::new(payload),
    };
    let sim = scene.sim_input().unwrap().unwrap();
    assert_eq!(sim.tools, vec![ToolSpec::Pen]);
    let strokes: Vec<_> = sim
        .motions
        .iter()
        .enumerate()
        .filter(|(_, m)| m.kind == "draw")
        .collect();
    assert_eq!(strokes.len(), 2);
    assert_eq!(strokes[1].0, strokes[0].0 + 1);
    assert!(strokes.iter().all(|(_, m)| m.feed_mm_min == Some(300.)));
}
