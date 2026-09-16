//! The display's motion stream stays **one motion per plan motion**.
//!
//! Every index-keyed table the display owns — the stage spans, the timeline
//! groups, the path vertices, the picker and the knife headings — addresses
//! motions by the plan's index. A programmed arc therefore travels inside one
//! motion rather than becoming a chord walk: expanding it here silently shifted
//! all of those tables, which showed up as the tool being a whole stage ahead
//! of the material it was cutting.
use cam_gui_runtime::{
    compute,
    session::{self, Command},
};
use cam_service::retained::Retained;

/// `(plan motions, display motions, arcs)` for one job, with the arc fit turned
/// on when the job does not ask for it: the fit is opt-in through the job's
/// tolerances, and a stream with arcs in it is what this file is about.
fn streams(job: &str, fit_arcs: bool) -> (usize, usize, usize) {
    let mut value: serde_json::Value = serde_json::from_str(job).expect("a job document");
    if fit_arcs {
        let tolerances = value["tolerances"].as_object_mut().expect("tolerances");
        // A fit may spend neither the motion nor the verification budget, so the
        // test asks for the smaller of the two.
        let budget = ["motion_tolerance_mm", "verification_tolerance_mm"]
            .iter()
            .filter_map(|key| tolerances.get(*key).and_then(|value| value.as_f64()))
            .fold(f64::INFINITY, f64::min);
        assert!(budget.is_finite() && budget > 0., "{tolerances:?}");
        tolerances.insert(
            "arc_fit_tolerance_mm".into(),
            serde_json::json!(budget.min(0.005)),
        );
    }
    let job = value.to_string();
    let (meta, payload) =
        session::execute(&mut Retained::new(), Command::generate(job)).expect("the job generates");
    let scene = compute::Scene {
        meta,
        payload: std::sync::Arc::new(payload),
    };
    let input = scene.sim_input().unwrap().expect("a motion stream");
    let arcs = input
        .motions
        .iter()
        .filter(|motion| motion.arc.is_some())
        .count();
    (scene.motion_count(), input.motions.len(), arcs)
}

#[test]
fn a_carving_keeps_one_display_motion_per_plan_motion() {
    let (plan, display, arcs) =
        streams(include_str!("../../../fixtures/gui2/flower.job.json"), true);
    assert!(plan > 0);
    assert_eq!(
        display, plan,
        "the display stream must stay one motion per plan motion"
    );
    assert!(
        arcs > 0,
        "this carving's fit emits arcs, so the stream exercises them"
    );
}

#[test]
fn a_facing_job_keeps_one_display_motion_per_plan_motion() {
    let (plan, display, _) = streams(include_str!("../../../../real_data/facing_job.json"), false);
    assert!(plan > 0);
    assert_eq!(display, plan);
}
