//! Knife tip simplification (plan section 12.4 step 2b): the merge is bounded
//! by a declared tolerance, measured after the fact, never removes a corner,
//! and can be turned off. `Some(0)` must reproduce the unsimplified program.
use cam_core::{
    checks::check_plan,
    job::{PlanningTolerances, SourceSnapshot},
    project::{
        CamJob, DragKnifeSettings, HeightRef, HeightReference, JobTool, KnifeAlignment,
        KnifeAssignment, Operation, OperationSettings, RectXY, SetupSettings, StartSelection,
        StockSetup, ToolCapabilities, ToolGeometry,
    },
    sequence::{GenerationStatus, KnifePathSimplification, OperationPlan, PlanLimits},
    svg::ImportOptions,
    toolpath::MotionPurpose,
};

/// A flattened curve with a real corner in it: the cubic ends on a steep
/// tangent, so the following line turns by more than the 30-degree threshold.
const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="cut" fill="none" stroke="#000" stroke-width="0.2" d="M5 5 C 10 15, 20 15, 25 5 L 35 5"/></svg>"##;

const KNIFE: &str = "blade";

fn settings(simplification: Option<f64>) -> DragKnifeSettings {
    DragKnifeSettings {
        chains: vec!["cut-chain-0".into()],
        assignment: KnifeAssignment {
            tool_id: KNIFE.into(),
            cutting_feed_mm_min: Some(150.),
            plunge_feed_mm_min: Some(60.),
            swivel_feed_mm_min: Some(50.),
            max_stepdown_mm: Some(1.),
        },
        top: HeightRef {
            reference: HeightReference::StockTop,
            offset_mm: 0.,
        },
        bottom: HeightRef {
            reference: HeightReference::StockTop,
            offset_mm: -0.5,
        },
        stepdown_mm: Some(0.5),
        swivel_depth_mm: Some(0.1),
        corner_threshold_deg: Some(30.),
        through_cut_allowance_mm: None,
        start: StartSelection::Automatic,
        closure_overlap_mm: None,
        alignment: KnifeAlignment {
            initial_heading_deg: Some(0.),
        },
        path_simplification_mm: simplification,
    }
}

fn job(simplification: Option<f64>) -> CamJob {
    let job = CamJob {
        name: "knife-simplification".into(),
        source: Some(SourceSnapshot {
            filename: "wave.svg".into(),
            svg: SVG.into(),
        }),
        import: ImportOptions::default(),
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(3.),
                xy: Some(RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: 40.,
                    length_mm: 30.,
                }),
            },
            work_zero: Default::default(),
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: None,
        },
        tools: vec![JobTool {
            id: KNIFE.into(),
            name: "drag knife".into(),
            geometry: Some(ToolGeometry::DragKnife(cam_core::project::DragKnifeSpec {
                blade_offset_mm: 1.,
                max_cut_depth_mm: 2.,
            })),
            capabilities: ToolCapabilities::default(),
        }],
        operations: vec![Operation {
            id: "knife-1".into(),
            name: "knife-1".into(),
            enabled: true,
            settings: OperationSettings::DragKnife(settings(simplification)),
        }],
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: None,
            arc_fit_tolerance_mm: None,
        },
    };
    job.validate().unwrap();
    job
}

fn planned(simplification: Option<f64>) -> OperationPlan {
    let job = job(simplification);
    OperationPlan::plan_job(&job, &PlanLimits::default()).unwrap()
}

fn simplification_of(plan: &OperationPlan) -> KnifePathSimplification {
    plan.operation_results[0]
        .named_outputs
        .iter()
        .find(|output| output.kind == "knife_path_simplification")
        .and_then(|output| output.path_simplification)
        .expect("the knife publishes its simplification evidence")
}

#[test]
fn the_default_merge_removes_vertices_within_its_declared_tolerance() {
    let exact = planned(Some(0.));
    let merged = planned(None);
    assert_eq!(
        exact.operation_results[0].generation_status,
        GenerationStatus::Complete
    );
    assert_eq!(
        merged.operation_results[0].generation_status,
        GenerationStatus::Complete
    );

    let off = simplification_of(&exact);
    assert_eq!(off.tolerance_mm, 0.);
    assert_eq!(off.vertices_before, off.vertices_after);
    assert_eq!(off.max_removed_deviation_mm, 0.);

    let on = simplification_of(&merged);
    // The planner's declared share of the 0.01 mm motion tolerance.
    assert_eq!(on.tolerance_mm, 0.0025);
    assert!(on.vertices_after < on.vertices_before, "{on:?}");
    assert!(
        on.max_removed_deviation_mm <= on.tolerance_mm,
        "a merged vertex moved further than the declared tolerance: {on:?}"
    );
    assert!(
        merged.motions.len() < exact.motions.len(),
        "the merge must remove moves: {} against {}",
        merged.motions.len(),
        exact.motions.len()
    );
}

#[test]
fn the_corner_survives_the_merge_and_still_swivels() {
    for simplification in [Some(0.), None] {
        let plan = planned(simplification);
        assert!(
            plan.motions
                .iter()
                .any(|motion| motion.purpose == MotionPurpose::KnifeSwivel),
            "the 60-degree corner must keep its swivel at {simplification:?}"
        );
    }
}

#[test]
fn a_simplification_over_the_motion_tolerance_is_refused_by_name() {
    let plan = planned(Some(0.05));
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|i| i.code == "KNIFE_SIMPLIFY_RANGE"),
        "{:?}",
        plan.generation_diagnostics
    );
    assert!(plan.motions.is_empty());
    // Nothing may be exported from it: the plan checks refuse output for an
    // incomplete operation (the export path requires `export_ready`).
    let checks = check_plan(&plan).unwrap();
    assert!(!checks.export_ready);
    assert!(
        checks
            .findings
            .iter()
            .any(|finding| finding.code == "PLAN_GENERATION_INCOMPLETE")
    );
}
