//! E2 finishing: radial allowance carried by the rough offset, one final
//! contour at the exact offset depth-stepped like the rough work with its own
//! feed, tabs protected in every deep rough and finish pass, and zero
//! allowance preserved as a meaningful supplied value (plan sections 10.2,
//! 18, 19.1). Legacy V-carve allowances are untouched: those suites run
//! unchanged as the regression evidence.
use cam_core::{
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    project::{
        CamJob, ContourSide, CutDirection, JobTool, MillingAssignment, Operation,
        OperationSettings, ProfileContour, ProfileFinishSettings, ProfileSettings, RectXY,
        SetupSettings, SpindleDirection, StockSetup, TabPlacement, TabSettings, TabShape,
        ToolCapabilities, ToolGeometry, TraversalDirection, WorkZero, WorkZeroXY, WorkZeroZ,
    },
    sequence::{GenerationStatus, OperationPlan, PlanLimits, StageRole},
    toolpath::{MotionEffect, MotionPurpose},
};

const CIRCLE_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="50mm" height="50mm" viewBox="0 0 50 50"><circle id="cut" cx="25" cy="25" r="20"/></svg>"#;
const RECT_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="pocket" fill-rule="evenodd" d="M5 5h30v20h-30z"/></svg>"#;

fn endmill(diameter: f64) -> JobTool {
    JobTool {
        id: "endmill".into(),
        name: "endmill".into(),
        geometry: Some(ToolGeometry::Endmill(cam_core::project::EndmillGeometry {
            diameter_mm: diameter,
            cutting_length_mm: 15.,
        })),
        capabilities: ToolCapabilities {
            plunge_capable: Some(true),
            ramp_capable: Some(false),
        },
    }
}

fn assignment() -> MillingAssignment {
    MillingAssignment {
        tool_id: "endmill".into(),
        spindle_rpm: Some(12_000.),
        spindle_direction: Some(SpindleDirection::Clockwise),
        cutting_feed_mm_min: Some(400.),
        plunge_feed_mm_min: Some(120.),
        max_stepdown_mm: Some(8.),
        stepover_mm: None,
    }
}

fn finish_settings(allowance: f64, feed: f64) -> ProfileFinishSettings {
    ProfileFinishSettings {
        enabled: true,
        radial_allowance_mm: Some(allowance),
        feed_mm_min: Some(feed),
    }
}

fn profile_op(
    contour: &str,
    side: ContourSide,
    depth: f64,
    stepdown: f64,
    through: Option<f64>,
    finish: ProfileFinishSettings,
    tabs: Option<TabSettings>,
) -> Operation {
    Operation {
        id: "profile-1".into(),
        name: "Profile".into(),
        enabled: true,
        settings: OperationSettings::Profile(ProfileSettings {
            contours: vec![ProfileContour {
                contour_id: contour.into(),
                side,
                traversal: if side == ContourSide::On {
                    Some(TraversalDirection::Forward)
                } else {
                    None
                },
            }],
            assignment: assignment(),
            top: Default::default(),
            bottom: cam_core::project::HeightRef {
                reference: if through.is_some() {
                    cam_core::project::HeightReference::StockBottom
                } else {
                    cam_core::project::HeightReference::StockTop
                },
                offset_mm: -through.unwrap_or(depth),
            },
            stepdown_mm: Some(stepdown),
            through_cut_allowance_mm: through,
            direction: Some(CutDirection::Climb),
            order: Default::default(),
            start: Default::default(),
            finish,
            entry: Default::default(),
            lead_in: Default::default(),
            lead_out: Default::default(),
            tabs,
        }),
    }
}

fn job(svg: &str, operation: Operation, stock: (f64, f64, f64), diameter: f64) -> CamJob {
    CamJob {
        schema_version: 4,
        name: "finish".into(),
        source: Some(SourceSnapshot {
            filename: "art.svg".into(),
            svg: svg.into(),
        }),
        import: Default::default(),
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(stock.2),
                xy: Some(RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: stock.0,
                    length_mm: stock.1,
                }),
            },
            work_zero: WorkZero {
                xy: WorkZeroXY::SetupOrigin,
                z: WorkZeroZ::StockTop,
            },
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: None,
        },
        tools: vec![endmill(diameter)],
        operations: vec![operation],
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
        },
        legacy_machine_profile: None,
    }
}

fn cut_motions(
    plan: &OperationPlan,
    purpose: MotionPurpose,
) -> Vec<&cam_core::toolpath::PlannedMotion> {
    plan.motions
        .iter()
        .filter(|m| m.purpose == purpose && m.effect == MotionEffect::MillingSweep)
        .collect()
}

/// Every cutting-motion endpoint's distance from `center`, min..max.
fn radius_range(motions: &[&cam_core::toolpath::PlannedMotion], center: Point) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for motion in motions {
        for p in [motion.start, motion.end] {
            let r = center.distance(Point::new(p.x, p.y));
            min = min.min(r);
            max = max.max(r);
        }
    }
    (min, max)
}

/// Plan section 19.1: circle radius 20, cutter radius 3, rough allowance 0.5
/// — rough center radius 23.5 (outside) / 16.5 (inside), final pass at 23 /
/// 17. Rough and finish run as consecutive same-tool stages, the finish pass
/// is depth-stepped to the exact bottom, and each pass keeps its own feed.
#[test]
fn rough_and_finish_offsets_match_the_requested_allowance() {
    let center = Point::new(25., 25.);
    for (side, rough_r, finish_r) in [
        (ContourSide::Outside, 23.5, 23.),
        (ContourSide::Inside, 16.5, 17.),
    ] {
        let op = profile_op(
            "cut-0-outer",
            side,
            5.,
            2.,
            None,
            finish_settings(0.5, 200.),
            None,
        );
        let plan = OperationPlan::plan_job(
            &job(CIRCLE_SVG, op, (60., 60., 8.), 6.),
            &PlanLimits::default(),
        )
        .unwrap();
        assert_eq!(
            plan.operation_results[0].generation_status,
            GenerationStatus::Complete,
            "{side:?}: {:?}",
            plan.generation_diagnostics
        );
        assert!(cam_core::checks::check_plan(&plan).unwrap().export_ready);

        // Consecutive same-tool stages: all the rough work, then all the
        // finish work, one tool change overall.
        assert_eq!(plan.stages.len(), 2, "{side:?}");
        assert_eq!(plan.stages[0].role, StageRole::ProfileRough, "{side:?}");
        assert_eq!(plan.stages[1].role, StageRole::ProfileFinish, "{side:?}");
        assert_eq!(plan.stages[0].tool_id, plan.stages[1].tool_id);
        assert_eq!(plan.stages[0].stage_id, "profile-1-profile-rough");
        assert_eq!(plan.stages[1].stage_id, "profile-1-profile-finish");
        // The rough stage precedes every finish motion.
        assert!(plan.stages[0].motion_range.1 <= plan.stages[1].motion_range.0);

        let rough = cut_motions(&plan, MotionPurpose::Rough);
        let finish = cut_motions(&plan, MotionPurpose::Finish);
        assert!(!rough.is_empty() && !finish.is_empty(), "{side:?}");
        let (rmin, rmax) = radius_range(&rough, center);
        assert!(
            (rmin - rough_r).abs() < 0.02 && (rmax - rough_r).abs() < 0.02,
            "{side:?} rough radius {rmin}..{rmax}, expected {rough_r}"
        );
        let (fmin, fmax) = radius_range(&finish, center);
        assert!(
            (fmin - finish_r).abs() < 0.02 && (fmax - finish_r).abs() < 0.02,
            "{side:?} finish radius {fmin}..{fmax}, expected {finish_r}"
        );

        // Feeds stay separate: rough cuts at the cutting feed, finish cuts
        // at the finishing feed (plan section 10.1).
        assert!(
            rough
                .iter()
                .all(|m| (m.feed_mm_min.unwrap() - 400.).abs() < 1e-9)
        );
        assert!(
            finish
                .iter()
                .all(|m| (m.feed_mm_min.unwrap() - 200.).abs() < 1e-9),
            "{side:?} finish motions must carry the finishing feed"
        );

        // Depth-stepped finish: layers -2, -4, -5 exactly, ending at the
        // bottom like the rough work.
        let finish_layers: Vec<f64> = finish
            .iter()
            .map(|m| m.end.z)
            .chain(finish.iter().map(|m| m.start.z))
            .collect();
        assert!(
            finish_layers.iter().all(|z| {
                (z + 2.).abs() < 1e-9 || (z + 4.).abs() < 1e-9 || (z + 5.).abs() < 1e-9
            })
        );
        assert!(
            finish.iter().any(|m| (m.end.z + 5.).abs() < 1e-9),
            "{side:?}: the finish pass reaches the exact bottom"
        );
        let layer_count = finish
            .iter()
            .map(|m| m.layer)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(layer_count.len(), 3, "{side:?}: three finish layers");
    }

    // Finish disabled keeps the D2 single-rough-stage shape: no silent
    // finish appears and none is needed.
    let op = profile_op(
        "cut-0-outer",
        ContourSide::Outside,
        5.,
        8.,
        None,
        ProfileFinishSettings::default(),
        None,
    );
    let plan = OperationPlan::plan_job(
        &job(CIRCLE_SVG, op, (60., 60., 8.), 6.),
        &PlanLimits::default(),
    )
    .unwrap();
    assert_eq!(plan.stages.len(), 1);
    assert_eq!(plan.stages[0].role, StageRole::ProfileRough);
    assert!(cut_motions(&plan, MotionPurpose::Finish).is_empty());
}

/// Plan sections 10.2/10.3 and the 19.1 "tabs + finish" row: the finish
/// pass runs the same tab envelope as the rough work — every deep rough and
/// finish pass rises onto the bridge, so the held material survives both.
#[test]
fn finish_passes_protect_every_tab() {
    let op = profile_op(
        "pocket-0-outer",
        ContourSide::Outside,
        8.2,
        2.,
        Some(0.2),
        finish_settings(0.5, 200.),
        Some(TabSettings {
            height_mm: Some(2.),
            width_mm: Some(5.),
            shape: TabShape::Rectangular,
            placement: TabPlacement::Automatic {
                count: Some(2),
                spacing_mm: None,
            },
        }),
    );
    let plan = OperationPlan::plan_job(
        &job(RECT_SVG, op, (40., 30., 8.), 4.),
        &PlanLimits::default(),
    )
    .unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        plan.generation_diagnostics
    );
    assert!(cam_core::checks::check_plan(&plan).unwrap().export_ready);

    // Both the rough and the finish stage contain tab transitions that stay
    // at the tab top -6 (measured from the physical stock bottom).
    for stage_id in ["profile-1-profile-rough", "profile-1-profile-finish"] {
        let transitions: Vec<_> = plan
            .motions
            .iter()
            .filter(|m| m.stage_id == stage_id && m.purpose == MotionPurpose::TabTransition)
            .collect();
        assert!(
            transitions.len() >= 4,
            "{stage_id}: two tabs × rise/traverse/lower in its deep passes, got {}",
            transitions.len()
        );
        assert!(
            transitions
                .iter()
                .all(|m| (m.start.z.max(m.end.z) - -6.).abs() < 1e-9),
            "{stage_id}: transitions must hold the -6 tab top"
        );
    }

    // The composed stock history holds the bridges after BOTH passes: the
    // two tabbed side midpoints stay at -6 with their bridge width, the
    // other two cut through to the stock bottom.
    let history = plan.stock_history(None).unwrap();
    let top_at = |x: f64, y: f64| history.material_top_at(Point::new(x, y)).unwrap();
    let midpoints = [
        (3., 15., 0., 1.),
        (37., 15., 0., -1.),
        (20., 3., -1., 0.),
        (20., 27., 1., 0.),
    ];
    let mut held = 0;
    let mut through = 0;
    for &(x, y, dx, dy) in &midpoints {
        let depth = top_at(x, y);
        if (depth - -6.).abs() < 1e-6 {
            held += 1;
            assert!(
                (top_at(x + dx * 1.75, y + dy * 1.75) - -6.).abs() < 1e-6,
                "bridge interior at ({x},{y}) must survive the finish pass"
            );
            assert!(
                (top_at(x + dx * 3.1, y + dy * 3.1) - -8.).abs() < 1e-6,
                "material beyond the bridge at ({x},{y}) must still cut through"
            );
        } else {
            assert!(
                (depth - -8.).abs() < 1e-6,
                "untabbed midpoint ({x},{y}) cut through, got {depth}"
            );
            through += 1;
        }
    }
    assert_eq!((held, through), (2, 2));

    // The finish motions cut at the finishing feed everywhere, including
    // their tab rises (plunge feed) — never the rough feed by accident.
    let finish = cut_motions(&plan, MotionPurpose::Finish);
    assert!(!finish.is_empty());
    assert!(
        finish
            .iter()
            .all(|m| (m.feed_mm_min.unwrap() - 200.).abs() < 1e-9)
    );
}

/// Zero allowance is a meaningful supplied value (plan section 1.3): the
/// finish pass runs at the exact final offset, and serde keeps Some(0.0)
/// distinct from unset.
#[test]
fn zero_allowance_finish_is_meaningful() {
    let op = profile_op(
        "cut-0-outer",
        ContourSide::Outside,
        5.,
        8.,
        None,
        finish_settings(0., 200.),
        None,
    );
    let plan = OperationPlan::plan_job(
        &job(CIRCLE_SVG, op, (60., 60., 8.), 6.),
        &PlanLimits::default(),
    )
    .unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        plan.generation_diagnostics
    );
    // Rough and finish both run at the final offset 23: the allowance of
    // zero moves nothing, but the finish pass still exists with its feed.
    let (rmin, rmax) = radius_range(
        &cut_motions(&plan, MotionPurpose::Rough),
        Point::new(25., 25.),
    );
    let (fmin, fmax) = radius_range(
        &cut_motions(&plan, MotionPurpose::Finish),
        Point::new(25., 25.),
    );
    assert!((rmin - 23.).abs() < 0.02 && (rmax - 23.).abs() < 0.02);
    assert!((fmin - 23.).abs() < 0.02 && (fmax - 23.).abs() < 0.02);

    // The document contract preserves the supplied zero exactly.
    let settings = finish_settings(0., 200.);
    let json = serde_json::to_string(&settings).unwrap();
    assert!(json.contains("\"radial_allowance_mm\":0.0"), "{json}");
    let parsed: ProfileFinishSettings = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.radial_allowance_mm, Some(0.));
}

/// Finishing values are validated with located diagnostics: missing fields
/// name their paths, nonsense ranges and on-contour allowances are explicit
/// errors, never silent adjustments.
#[test]
fn finish_values_are_validated_and_located() {
    // Missing allowance/feed are located missing fields, not planning errors.
    let mut op = profile_op(
        "cut-0-outer",
        ContourSide::Outside,
        5.,
        8.,
        None,
        ProfileFinishSettings {
            enabled: true,
            radial_allowance_mm: None,
            feed_mm_min: None,
        },
        None,
    );
    let document = job(CIRCLE_SVG, op.clone(), (60., 60., 8.), 6.);
    let OperationSettings::Profile(settings) = &document.operations[0].settings else {
        unreachable!()
    };
    let missing = cam_core::operations::profile::missing_fields(&document, "profile-1", settings);
    assert!(
        missing.iter().any(|d| d
            .field_path
            .as_deref()
            .unwrap_or("")
            .contains("finish.radial_allowance_mm")),
        "{missing:?}"
    );
    assert!(
        missing.iter().any(|d| d
            .field_path
            .as_deref()
            .unwrap_or("")
            .contains("finish.feed_mm_min")),
        "{missing:?}"
    );
    let plan = OperationPlan::plan_job(&document, &PlanLimits::default()).unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Incomplete
    );
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "MISSING_MACHINING_SETTING"
                && d.message.contains("finish.radial_allowance_mm"))
    );

    // Negative allowance and non-positive feed are rejected by document
    // validation before planning: supplied values are validated immediately
    // (plan section 5.2), with the field named.
    for (allowance, feed) in [(-0.1, 200.), (0.5, 0.)] {
        if let OperationSettings::Profile(settings) = &mut op.settings {
            settings.finish = finish_settings(allowance, feed);
        }
        let rejected = OperationPlan::plan_job(
            &job(CIRCLE_SVG, op.clone(), (60., 60., 8.), 6.),
            &PlanLimits::default(),
        );
        let diagnostic = rejected.expect_err("document validation rejects it");
        assert_eq!(diagnostic.code, "PROJECT_PARAMETER");
        assert!(
            diagnostic.message.contains(if allowance < 0. {
                "finish.radial_allowance_mm"
            } else {
                "finish.feed_mm_min"
            }),
            "{}",
            diagnostic.message
        );
    }

    // On-contour selections have zero offset and cannot carry an allowance
    // (plan section 10.2); zero stays allowed.
    let mut on = profile_op(
        "cut-0-outer",
        ContourSide::On,
        5.,
        8.,
        None,
        finish_settings(0.5, 200.),
        None,
    );
    let plan = OperationPlan::plan_job(
        &job(CIRCLE_SVG, on.clone(), (60., 60., 8.), 6.),
        &PlanLimits::default(),
    )
    .unwrap();
    assert!(
        plan.generation_diagnostics
            .iter()
            .any(|d| d.code == "PROFILE_FINISH_ALLOWANCE")
    );
    if let OperationSettings::Profile(settings) = &mut on.settings {
        settings.finish = finish_settings(0., 200.);
    }
    let plan = OperationPlan::plan_job(
        &job(CIRCLE_SVG, on, (60., 60., 8.), 6.),
        &PlanLimits::default(),
    )
    .unwrap();
    assert_eq!(
        plan.operation_results[0].generation_status,
        GenerationStatus::Complete,
        "{:?}",
        plan.generation_diagnostics
    );
}
