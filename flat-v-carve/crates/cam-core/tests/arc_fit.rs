//! Milling arc fitting (plan section 8.4, option 4): a declared tolerance
//! rewrites a stage's polyline into lines and arcs, never across a semantic
//! breakpoint, and the result is *measured* against the polyline it replaced.
use cam_core::{
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    project::{
        CamJob, ContourSide, JobTool, MillingAssignment, Operation, OperationSettings,
        ProfileContour, ProfileSettings, RectXY, SetupSettings, SpindleDirection, StockSetup,
        ToolCapabilities, ToolGeometry, WorkZero, WorkZeroXY, WorkZeroZ,
    },
    sequence::{GenerationStatus, OperationPlan, PlanLimits},
    toolpath::{Interpolation, MotionEffect, MotionPurpose, PlannedMotion},
};

/// A 40 mm circle: the profile offset of it is a long smooth polyline, which
/// is what the fit exists to collapse.
const CIRCLE_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="50mm" height="50mm" viewBox="0 0 50 50"><circle id="cut" cx="25" cy="25" r="20"/></svg>"#;

fn job(fit: Option<f64>) -> CamJob {
    let settings = ProfileSettings {
        contours: vec![ProfileContour {
            contour_id: "cut-0-outer".into(),
            side: ContourSide::Outside,
            traversal: None,
        }],
        assignment: MillingAssignment {
            tool_id: "endmill".into(),
            spindle_rpm: Some(12_000.),
            spindle_direction: Some(SpindleDirection::Clockwise),
            cutting_feed_mm_min: Some(400.),
            plunge_feed_mm_min: Some(120.),
            max_stepdown_mm: Some(8.),
            stepover_mm: None,
        },
        top: cam_core::project::HeightRef {
            reference: cam_core::project::HeightReference::StockTop,
            offset_mm: 0.,
        },
        bottom: cam_core::project::HeightRef {
            reference: cam_core::project::HeightReference::StockTop,
            offset_mm: -2.,
        },
        stepdown_mm: Some(2.),
        through_cut_allowance_mm: None,
        direction: Some(cam_core::project::CutDirection::Climb),
        order: Default::default(),
        start: Default::default(),
        finish: Default::default(),
        entry: Default::default(),
        lead_in: Default::default(),
        lead_out: Default::default(),
        tabs: None,
    };
    let job = CamJob {
        name: "arc-fit".into(),
        source: Some(SourceSnapshot {
            filename: "circle.svg".into(),
            svg: CIRCLE_SVG.into(),
        }),
        import: Default::default(),
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(6.),
                xy: Some(RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: 50.,
                    length_mm: 50.,
                }),
            },
            work_zero: WorkZero {
                xy: WorkZeroXY::SetupOrigin,
                z: WorkZeroZ::StockTop,
            },
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: None,
        },
        tools: vec![JobTool {
            id: "endmill".into(),
            name: "6mm endmill".into(),
            geometry: Some(ToolGeometry::Endmill(cam_core::project::EndmillGeometry {
                diameter_mm: 6.,
                cutting_length_mm: 15.,
            })),
            capabilities: ToolCapabilities {
                plunge_capable: Some(true),
                ramp_capable: Some(false),
            },
        }],
        operations: vec![Operation {
            id: "profile-1".into(),
            name: "Profile".into(),
            enabled: true,
            settings: OperationSettings::Profile(settings),
        }],
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
            arc_fit_tolerance_mm: fit,
        },
    };
    job.validate().unwrap();
    job
}

fn plan(fit: Option<f64>) -> OperationPlan {
    let plan = OperationPlan::plan_job(&job(fit), &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        plan.generation_diagnostics
    );
    plan
}

/// A motion as the plan executes it: sample points from start to end, with
/// arcs followed as arcs.
fn sampled(motion: &PlannedMotion) -> Vec<Point> {
    let (from, to) = (motion.start.xy(), motion.end.xy());
    let Interpolation::ArcFeed(arc) = motion.interpolation else {
        return vec![from, to];
    };
    let Some(sweep) = arc.sweep_rad(from, to) else {
        return vec![from, to];
    };
    // Sample finely enough that the sampling's own chord error is far below
    // the tolerance being checked: sagitta = step^2 / (8 r).
    let radius = arc.radius(from).unwrap_or(1.);
    let step = (8. * radius * 1e-6).sqrt().max(1e-6);
    let steps = ((radius * sweep / step).ceil() as usize).clamp(1, 20_000);
    (0..=steps)
        .filter_map(|step| arc.point_at(from, to, step as f64 / steps as f64))
        .collect()
}

/// Independent check of the fit's promise: the largest distance from any
/// vertex of the *unfitted* polyline to the fitted path.
fn max_deviation(polyline: &[PlannedMotion], fitted: &[PlannedMotion]) -> f64 {
    let mut samples: Vec<Point> = vec![];
    for motion in fitted {
        for point in sampled(motion) {
            if samples.last() != Some(&point) {
                samples.push(point);
            }
        }
    }
    let mut worst: f64 = 0.;
    for motion in polyline {
        for point in [motion.start.xy(), motion.end.xy()] {
            let mut nearest = f64::INFINITY;
            for pair in samples.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let dx = b.x - a.x;
                let dy = b.y - a.y;
                let length_squared = dx * dx + dy * dy;
                let t = if length_squared <= 1e-24 {
                    0.
                } else {
                    (((point.x - a.x) * dx + (point.y - a.y) * dy) / length_squared).clamp(0., 1.)
                };
                nearest = nearest.min(point.distance(Point::new(a.x + t * dx, a.y + t * dy)));
            }
            worst = worst.max(nearest);
        }
    }
    worst
}

#[test]
fn a_declared_tolerance_collapses_the_polyline_within_its_budget() {
    let exact = plan(None);
    let fitted = plan(Some(0.005));
    let tolerance = fitted.operation_results[0]
        .named_outputs
        .iter()
        .find(|output| output.kind == "arc_fit")
        .and_then(|output| output.arc_fit)
        .expect("the fit publishes its evidence");
    assert_eq!(tolerance.tolerance_mm, 0.005);
    assert_eq!(tolerance.motions_before, exact.motions.len());
    assert_eq!(tolerance.motions_after, fitted.motions.len());
    // A profile pass around a 40 mm circle: 1 261 motions become 6, two of
    // them arcs. The bound is loose so a planner change does not make this
    // fixture fail for the wrong reason.
    assert!(
        fitted.motions.len() < 20,
        "the profile pass collapses to a handful of primitives, not {}",
        fitted.motions.len()
    );
    assert!(tolerance.arcs_emitted > 0, "{tolerance:?}");
    assert!(
        fitted.motions.len() * 2 < exact.motions.len(),
        "the fit must collapse the polyline: {} against {}",
        fitted.motions.len(),
        exact.motions.len()
    );
    assert!(
        tolerance.max_deviation_mm <= tolerance.tolerance_mm,
        "{tolerance:?}"
    );
    // Measured again, outside the fit: every vertex of the original polyline
    // lies inside the declared tolerance of the fitted path.
    let measured = max_deviation(&exact.motions, &fitted.motions);
    assert!(
        measured <= 0.005 + 1e-6,
        "the fitted path deviates {measured} mm from the resolved polyline"
    );
}

#[test]
fn no_tolerance_and_zero_leave_the_planner_output_untouched() {
    let exact = plan(None);
    for fit in [Some(0.), None] {
        let same = plan(fit);
        assert_eq!(same.motions.len(), exact.motions.len());
        for (a, b) in same.motions.iter().zip(&exact.motions) {
            assert_eq!(a.interpolation, b.interpolation);
            assert_eq!(a.start, b.start);
            assert_eq!(a.end, b.end);
        }
        assert!(
            same.operation_results[0]
                .named_outputs
                .iter()
                .all(|output| output.kind != "arc_fit"),
            "an unset tolerance publishes no fit evidence"
        );
    }
}

#[test]
fn semantic_breakpoints_survive_the_fit() {
    let exact = plan(Some(0.005));
    let entries = |plan: &OperationPlan| {
        plan.motions
            .iter()
            .filter(|motion| motion.purpose == MotionPurpose::Entry)
            .map(|motion| (motion.start, motion.end, motion.feed_mm_min))
            .collect::<Vec<_>>()
    };
    let untouched = plan(None);
    assert_eq!(entries(&exact), entries(&untouched));
    // Only cutting purposes are ever rewritten.
    for motion in &exact.motions {
        if matches!(motion.interpolation, Interpolation::ArcFeed(_)) {
            assert!(
                matches!(motion.purpose, MotionPurpose::Rough | MotionPurpose::Finish),
                "{:?}",
                motion.purpose
            );
            assert_eq!(motion.effect, MotionEffect::MillingSweep);
        }
    }
}

/// A rewrite that changes the motion stream must leave the plan's own evidence
/// addressing it: the ranges the inspector and the preview read are recomputed
/// against the fitted motions, not left pointing at numbering that no longer
/// exists.
#[test]
fn the_rewrite_leaves_every_evidence_range_addressing_the_fitted_stream() {
    // The V-carve adapter is the one that publishes per-execution evidence, so
    // this check runs on its fixture rather than on the profile circle.
    const FLOWER: &str = include_str!("../../../fixtures/gui2/flower.job.json");
    let mut job: cam_core::project::v5::CamJobV5 = serde_json::from_str(FLOWER).unwrap();
    job.tolerances.arc_fit_tolerance_mm = Some(0.005);
    job.validate_structure().unwrap();
    let exact: cam_core::project::v5::CamJobV5 = serde_json::from_str(FLOWER).unwrap();
    let plan_v5 = |job: &cam_core::project::v5::CamJobV5| {
        cam_core::sequence::OperationPlanV5::plan_job_v5(
            job,
            &cam_core::project::v5::ReadinessScope::AllEnabled,
            &PlanLimits::default(),
        )
        .unwrap()
    };
    let fitted = plan_v5(&job);
    let unfitted = plan_v5(&exact);
    assert_ne!(
        fitted.motions.len(),
        unfitted.motions.len(),
        "this fixture must actually be rewritten"
    );
    let result = &fitted.operation_results[0];
    assert!(!result.legacy_pass_evidence.is_empty());
    for evidence in &result.legacy_pass_evidence {
        assert!(
            evidence.end_motion_id > evidence.first_motion_id,
            "pass {} kept an empty range",
            evidence.pass_id
        );
        for motion in &fitted.motions[evidence.first_motion_id..evidence.end_motion_id] {
            assert_eq!(
                motion.pass_id, evidence.pass_id,
                "range of pass {} covers motion {} of pass {}",
                evidence.pass_id, motion.id, motion.pass_id
            );
        }
    }
    for evidence in &result.legacy_stage_evidence {
        let stage = fitted
            .stages
            .iter()
            .find(|stage| stage.stage_id == evidence.stage_id)
            .expect("stage evidence names a plan stage");
        assert_eq!(evidence.legacy_first_motion_id, stage.motion_range.0);
        assert_eq!(evidence.legacy_end_motion_id, stage.motion_range.1);
    }
}
