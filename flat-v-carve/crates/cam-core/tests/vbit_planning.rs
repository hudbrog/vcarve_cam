use cam_core::{
    geometry::{BoundaryQuery, Grid, Point, PointLocation, Region, Segment},
    job::Job,
    model::{VBit, VBitSpec},
    motion::{Motion, MotionKind, Position},
    pocket::PlanStatus,
    stock::{
        removed_depth_at, variable_capsule_bounds, vbit_air_in_endmill, vbit_removal_at_slice,
        vbit_removed_depth_at,
    },
    vcarve::{CombinedPlan, PathFamily, plan_combined, verify_combined_plan, verify_vbit_motions},
};
use std::sync::OnceLock;
fn fixture(name: &str) -> Job {
    Job::from_json(
        &std::fs::read_to_string(format!(
            "{}/../../fixtures/m4/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap()
}
fn planned(name: &str) -> &'static CombinedPlan {
    const NAMES: [&str; 7] = [
        "wide-floor",
        "narrow-channel",
        "finite-tip",
        "exact-fit",
        "island",
        "disconnected",
        "curved-medial",
    ];
    static CACHE: [OnceLock<CombinedPlan>; 7] = [const { OnceLock::new() }; 7];
    let i = NAMES.iter().position(|n| *n == name).unwrap();
    CACHE[i].get_or_init(|| plan_combined(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}")))
}
fn tool(tip: f64) -> VBit {
    VBit::try_from(VBitSpec {
        included_angle_deg: 90.,
        tip_diameter_mm: 2. * tip,
        max_cutting_diameter_mm: 12.,
        cutting_height_mm: 5.,
    })
    .unwrap()
}
fn motion(a: Position, b: Position, kind: MotionKind) -> Motion {
    Motion {
        id: 0,
        tool_id: "vbit".into(),
        operation_id: "flat-v-carve".into(),
        layer: 0,
        kind,
        start: a,
        end: b,
        feed_mm_min: Some(250.),
    }
}
fn rect() -> Region {
    Region::from_rings(
        Grid::new(0.005, 50.).unwrap(),
        &[vec![
            Point::new(0., 0.),
            Point::new(20., 0.),
            Point::new(20., 20.),
            Point::new(0., 20.),
        ]],
    )
    .unwrap()
}

#[test]
fn combined_fixture_matrix_clears_floors_and_preserves_islands() {
    for name in [
        "wide-floor",
        "narrow-channel",
        "finite-tip",
        "exact-fit",
        "island",
        "disconnected",
        "curved-medial",
    ] {
        let p = planned(name);
        assert_eq!(
            p.analysis.status,
            PlanStatus::Complete,
            "{name}: {:?}",
            p.analysis.diagnostics
        );
        assert!(p.analysis.missing_floor_beyond_tolerance.rings().is_empty());
        assert!(
            p.analysis
                .slices
                .iter()
                .all(|s| s.possible_overcut.rings().is_empty())
        );
        assert_eq!(
            p.analysis.finish_paths_expected,
            p.analysis.finish_paths_executed
        );
        assert!(p.analysis.finish_paths_expected > 0);
    }
    let island = planned("island");
    assert_eq!(
        vbit_removed_depth_at(&island.vbit_motions, &tool(0.), Point::new(20., 15.)).unwrap(),
        0.
    );
}
#[test]
fn variable_radius_check_accepts_a_safe_rising_path_and_rejects_interior_gouges() {
    let q = BoundaryQuery::new(&rect());
    let s = Segment {
        start: Point::new(1., 5.),
        end: Point::new(4., 5.),
    };
    assert!(q.variable_radius_margin_mm(s, 0.99, 3.99).unwrap() > 0.);
    assert!(q.segment_distance_mm(s).unwrap() < 3.99); // A deepest-depth-only check would wrongly reject it.
    let job = fixture("island");
    let q = BoundaryQuery::new(&job.inspect().unwrap().geometry.selected);
    let s = Segment {
        start: Point::new(8., 15.),
        end: Point::new(32., 15.),
    };
    assert!(
        q.sample(s.start).unwrap().distance_mm > 3. && q.sample(s.end).unwrap().distance_mm > 3.
    );
    assert!(q.variable_radius_margin_mm(s, 1., 3.).unwrap() < 0.);
}
#[test]
fn tapered_sweep_brackets_analytic_cone_hull_and_retains_its_apex() {
    let g = Grid::new(0.005, 100.).unwrap();
    let c = variable_capsule_bounds(g, Point::new(0., 0.), 0., Point::new(10., 0.), 2.).unwrap();
    let exact = 2. * 96f64.sqrt() + 4. * (-0.2f64).acos();
    assert!(c.lower.area_mm2() < exact && c.upper.area_mm2() > exact);
    assert!(exact - c.lower.area_mm2() < 0.08);
    assert!(c.radial_error_mm < 0.005);
    assert_eq!(
        BoundaryQuery::new(&c.lower)
            .sample(Point::new(1., 0.))
            .unwrap()
            .location,
        PointLocation::Inside
    );
    let m = motion(
        Position::new(Point::new(0., 0.), 0.),
        Position::new(Point::new(10., 0.), -2.),
        MotionKind::Cut,
    );
    let s = vbit_removal_at_slice(g, &[m], &tool(0.), 1.).unwrap();
    let q = BoundaryQuery::new(&s.lower);
    assert_eq!(
        q.sample(Point::new(4., 0.)).unwrap().location,
        PointLocation::Outside
    );
    assert_eq!(
        q.sample(Point::new(6., 0.)).unwrap().location,
        PointLocation::Inside
    );
    let tiny = variable_capsule_bounds(g, Point::new(0., 0.), 0., Point::new(2., 0.), 0.).unwrap();
    assert!(tiny.lower.rings().is_empty());
}
#[test]
fn actual_vbit_point_query_matches_stationary_solution_and_dense_reference() {
    let ramp = motion(
        Position::new(Point::new(0., 0.), 0.),
        Position::new(Point::new(10., 0.), -2.),
        MotionKind::Cut,
    );
    assert!(
        (vbit_removed_depth_at(std::slice::from_ref(&ramp), &tool(0.), Point::new(5., 1.))
            .unwrap()
            - (1. - 0.96f64.sqrt()))
        .abs()
            < 1e-12
    );
    assert!(
        (vbit_removed_depth_at(std::slice::from_ref(&ramp), &tool(1.), Point::new(5., 0.))
            .unwrap()
            - 1.2)
            .abs()
            < 1e-12
    );
    for tip in [0., 0.4] {
        for p in [
            Point::new(3., 0.2),
            Point::new(5., 0.7),
            Point::new(9., 1.5),
        ] {
            let tool = tool(tip);
            let actual = vbit_removed_depth_at(std::slice::from_ref(&ramp), &tool, p).unwrap();
            let reference = (0..=20_000)
                .map(|i| {
                    let pose = ramp.start.lerp(ramp.end, i as f64 / 20_000.);
                    tool.removal_depth(
                        cam_core::model::Depth::new(pose.depth()).unwrap(),
                        cam_core::model::Length::new(pose.xy().distance(p)).unwrap(),
                    )
                    .unwrap()
                    .mm()
                })
                .fold(0., f64::max);
            assert!(actual + 1e-12 >= reference && actual - reference < 0.001);
        }
    }
}
#[test]
fn floor_ridge_matches_parallel_lane_formula_for_pointed_and_flat_tips() {
    for tip in [0., 0.1] {
        let paths = [
            motion(
                Position::new(Point::new(0., 0.), -2.),
                Position::new(Point::new(10., 0.), -2.),
                MotionKind::Cut,
            ),
            motion(
                Position::new(Point::new(0., 0.6), -2.),
                Position::new(Point::new(10., 0.6), -2.),
                MotionKind::Cut,
            ),
        ];
        let removed = vbit_removed_depth_at(&paths, &tool(tip), Point::new(5., 0.3)).unwrap();
        assert!((2. - removed - (0.3 - tip)).abs() < 1e-12);
    }
}
#[test]
fn air_proof_checks_flanks_in_addition_to_cleared_centers() {
    let prior = [motion(
        Position::new(Point::new(0., 0.), -2.),
        Position::new(Point::new(10., 0.), -2.),
        MotionKind::Cut,
    )];
    let centered = motion(
        Position::new(Point::new(4., 0.), -1.),
        Position::new(Point::new(6., 0.), -1.),
        MotionKind::Cut,
    );
    assert!(vbit_air_in_endmill(&centered, &tool(0.), &prior, 3., 0.01));
    let flank = motion(
        Position::new(Point::new(4., 2.5), -1.),
        Position::new(Point::new(6., 2.5), -1.),
        MotionKind::Cut,
    );
    assert_eq!(removed_depth_at(&prior, 3., flank.start.xy()).unwrap(), 2.);
    assert!(!vbit_air_in_endmill(&flank, &tool(0.), &prior, 3., 0.01));
    assert!(planned("wide-floor").analysis.pruned_air_paths > 0);
}
#[test]
fn narrowed_regions_get_vbit_depth_passes_when_the_endmill_stage_is_empty() {
    let p = planned("narrow-channel");
    assert_eq!(p.endmill.analysis.status, PlanStatus::Empty);
    assert!(p.endmill.motions.is_empty());
    assert!(!p.vbit_motions.is_empty());
    assert!(
        p.executions
            .iter()
            .any(|e| !e.final_finish && e.pass_depth_mm == 1.)
    );
    assert!(p.executions.iter().any(|e| e.pass_depth_mm == 2.));
    assert!(
        p.vbit_motions
            .iter()
            .any(|m| m.kind == MotionKind::Cut && m.start.z != m.end.z)
    );
    assert!(
        p.vbit_motions
            .iter()
            .all(|m| m.kind != MotionKind::Plunge || m.start.z - m.end.z <= 1. + 1e-12)
    );
}
#[test]
fn medial_branches_preserve_curves_radii_and_join_full_depth_families() {
    let curved = planned("curved-medial");
    assert!(curved.analysis.medial_axis.curved_branches > 0);
    let query = BoundaryQuery::new(&curved.endmill.job.inspect().unwrap().geometry.selected);
    for b in &curved.analysis.medial_axis.branches {
        for (p, r) in b.points.iter().zip(&b.clearance_radii_mm) {
            assert!((query.sample(p.xy()).unwrap().distance_mm - r).abs() < 1e-10);
        }
    }
    let p = planned("wide-floor");
    let boundaries: Vec<_> = p
        .executions
        .iter()
        .filter(|e| e.final_finish && e.candidate.family == PathFamily::Boundary)
        .flat_map(|e| {
            e.candidate.points.windows(2).map(|w| Segment {
                start: w[0].xy(),
                end: w[1].xy(),
            })
        })
        .collect();
    let joins: Vec<_> = p
        .executions
        .iter()
        .filter(|e| e.final_finish && e.candidate.family == PathFamily::Medial)
        .flat_map(|e| &e.candidate.points)
        .filter(|q| q.depth() > 1.999)
        .collect();
    assert!(!joins.is_empty());
    for q in joins {
        assert!(
            boundaries
                .iter()
                .map(|s| s.distance(q.xy()))
                .fold(f64::INFINITY, f64::min)
                < 0.01
        );
    }
}
#[test]
fn medial_chords_retain_clearance_for_conservative_stock_brackets() {
    for name in ["curved-medial", "finite-tip", "island"] {
        let plan = planned(name);
        let region = plan.endmill.job.inspect().unwrap().geometry.selected;
        let query = BoundaryQuery::new(&region);
        let bit = if name == "finite-tip" {
            tool(0.5)
        } else {
            tool(0.)
        };
        let reserve = region.grid().tolerance_mm();
        let mut checked = 0;
        for execution in &plan.executions {
            if execution.candidate.family != PathFamily::Medial {
                continue;
            }
            for pair in execution.candidate.points.windows(2) {
                let radius =
                    |p: Position| bit.tip_radius().mm() + p.depth() * bit.angle().slope() + reserve;
                let margin = query
                    .variable_radius_margin_mm(
                        Segment {
                            start: pair[0].xy(),
                            end: pair[1].xy(),
                        },
                        radius(pair[0]),
                        radius(pair[1]),
                    )
                    .unwrap();
                assert!(
                    margin >= 0.,
                    "{name}: medial chord fails the reserved-clearance check: {margin}"
                );
                checked += 1;
            }
        }
        assert!(checked > 0);
    }
}
#[test]
fn zero_ridge_is_rejected_only_when_pointed_area_clearing_is_needed() {
    assert_eq!(
        plan_combined(&fixture("zero-ridge")).unwrap_err().code,
        "ZERO_RIDGE_AREA_CLEARING"
    );
    let mut narrow = fixture("narrow-channel");
    narrow.operation.max_floor_ridge_mm = Some(0.);
    assert_eq!(
        plan_combined(&narrow).unwrap().analysis.status,
        PlanStatus::Complete
    );
    let mut flat = fixture("finite-tip");
    flat.operation.max_floor_ridge_mm = Some(0.);
    assert_eq!(
        plan_combined(&flat).unwrap().analysis.status,
        PlanStatus::Complete
    );
}
#[test]
fn cutter_limited_detail_is_measured_separately_from_missed_reachable_stock() {
    let accepted = planned("finite-tip");
    assert!(accepted.analysis.max_sampled_unavoidable_residual_mm > 0.1);
    assert!(accepted.analysis.max_sampled_missed_reachable_mm < 0.05);
    let mut strict = fixture("finite-tip");
    strict.operation.max_detail_residual_mm = Some(0.);
    let p = plan_combined(&strict).unwrap();
    assert_eq!(p.analysis.status, PlanStatus::Incomplete);
    assert!(
        p.analysis
            .diagnostics
            .iter()
            .any(|d| d.code == "DETAIL_RESIDUAL_LIMIT")
    );
}
#[test]
fn saved_combined_reports_recompute_and_reject_stale_motion_or_job_edits() {
    let p = planned("narrow-channel");
    let saved = p.to_json().unwrap();
    let artifact: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert!(artifact.get("analysis").is_none());
    assert!(artifact["endmill"].get("analysis").is_none());
    assert_eq!(
        CombinedPlan::from_json(&saved).unwrap().to_json().unwrap(),
        saved
    );
    let mut value: serde_json::Value = serde_json::from_str(&saved).unwrap();
    value["analysis"] = serde_json::json!({"status":"empty","samples":[]});
    value["vbit_spindle_rpm"] = serde_json::json!(1);
    let loaded = CombinedPlan::from_json(&value.to_string()).unwrap();
    let streamed = CombinedPlan::from_reader(value.to_string().as_bytes()).unwrap();
    assert_eq!(streamed.to_json().unwrap(), loaded.to_json().unwrap());
    assert_eq!(loaded.analysis.status, PlanStatus::Complete);
    assert_eq!(loaded.vbit_spindle_rpm, 12000.);
    value["vbit_motions"][0]["end"]["x"] = serde_json::json!(999.);
    assert_eq!(
        CombinedPlan::from_reader(value.to_string().as_bytes())
            .unwrap_err()
            .code,
        "STALE_PLAN"
    );
    assert_eq!(
        CombinedPlan::from_json(&value.to_string())
            .unwrap_err()
            .code,
        "STALE_PLAN"
    );
    value = serde_json::from_str(&saved).unwrap();
    value["endmill"]["job"]["operation"]["max_floor_ridge_mm"] = serde_json::json!(0.9);
    assert_eq!(
        CombinedPlan::from_reader(value.to_string().as_bytes())
            .unwrap_err()
            .code,
        "STALE_PLAN"
    );
    assert_eq!(
        CombinedPlan::from_json(&value.to_string())
            .unwrap_err()
            .code,
        "STALE_PLAN"
    );
    value = serde_json::from_str(&saved).unwrap();
    value["engine_version"] = serde_json::json!("old");
    assert_eq!(
        CombinedPlan::from_json(&value.to_string())
            .unwrap_err()
            .code,
        "PLAN_VERSION"
    );
}
#[test]
fn streamed_plan_loading_is_independent_of_the_string_api_size_limit() {
    use std::io::{BufReader, Read};
    let saved = planned("narrow-channel").to_json().unwrap();
    // Valid trailing whitespace crosses 128 MB without creating a huge fixture
    // or allocating a huge string. Identity and analysis must still be rebuilt.
    let reader = saved
        .as_bytes()
        .chain(std::io::repeat(b' ').take(128_000_001));
    let loaded = CombinedPlan::from_reader(BufReader::new(reader)).unwrap();
    assert_eq!(loaded.to_json().unwrap(), saved);
    let truncated = &saved.as_bytes()[..saved.len() / 2];
    assert_eq!(
        CombinedPlan::from_reader(truncated).unwrap_err().code,
        "PLAN_JSON"
    );
}

#[test]
fn authenticated_plans_rebuild_cached_reports_and_bind_an_immutable_snapshot() {
    use cam_core::{
        vcarve::AuthenticatedPlan,
        verification::{VerificationOptions, verify_authenticated_plan, verify_plan},
    };
    let mut source = planned("narrow-channel").clone();
    source.analysis.finish_paths_executed = 0;
    let authenticated = AuthenticatedPlan::from_plan(&source).unwrap();
    assert_eq!(
        authenticated.plan().analysis.finish_paths_executed,
        planned("narrow-channel").analysis.finish_paths_executed
    );
    let options = VerificationOptions {
        max_cells: 1,
        ..Default::default()
    };
    let report = verify_authenticated_plan(&authenticated, &options).unwrap();
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        serde_json::to_value(verify_plan(&source, &options).unwrap()).unwrap()
    );
    source.vbit_motions[0].end.x += 1.;
    assert_eq!(
        verify_plan(&source, &options).unwrap_err().code,
        "STALE_PLAN"
    );
    assert!(AuthenticatedPlan::from_plan(&source).is_err());
    assert_eq!(
        serde_json::to_value(verify_authenticated_plan(&authenticated, &options).unwrap()).unwrap(),
        serde_json::to_value(report).unwrap()
    );
}

#[test]
fn missing_final_finish_cannot_be_hidden_by_an_already_clean_stock_preview() {
    let mut p = planned("narrow-channel").clone();
    let at = p.executions.iter().position(|e| e.final_finish).unwrap();
    let id = p.executions[at].first_motion_id;
    p.executions.truncate(at);
    p.vbit_motions.truncate(id - p.endmill.motions.len());
    let a = verify_combined_plan(&p).unwrap();
    assert_eq!(a.status, PlanStatus::Incomplete);
    assert!(
        a.diagnostics
            .iter()
            .any(|d| d.code == "INCOMPLETE_BOUNDARY_FINISH")
    );
    assert!(a.max_sampled_missed_reachable_mm < 0.05);
}
// Reconnect independently retained excursions, keeping their actual cuts and profiles.
fn retain_executions(p: &mut CombinedPlan, keep: impl Fn(&cam_core::vcarve::Execution) -> bool) {
    let old = std::mem::take(&mut p.vbit_motions);
    let records = std::mem::take(&mut p.executions);
    let base = p.endmill.motions.len();
    let mut previous = p.transition.position;
    for mut e in records.into_iter().filter(keep) {
        let mut chunk = old[e.first_motion_id - base..e.end_motion_id - base].to_vec();
        if let Some(first) = chunk.first_mut() {
            if first.kind == MotionKind::RapidXY {
                first.start = previous;
                if first.start == first.end {
                    chunk.remove(0);
                }
            } else if first.start != previous {
                let mut rapid = first.clone();
                rapid.kind = MotionKind::RapidXY;
                rapid.feed_mm_min = None;
                rapid.end = first.start;
                rapid.start = previous;
                chunk.insert(0, rapid);
            }
        }
        e.first_motion_id = base + p.vbit_motions.len();
        for m in &mut chunk {
            m.id = base + p.vbit_motions.len();
            previous = m.end;
            p.vbit_motions.push(m.clone());
        }
        e.end_motion_id = base + p.vbit_motions.len();
        p.executions.push(e);
    }
}
#[test]
fn removing_floor_lanes_reports_missed_floor_instead_of_unreachable_detail() {
    let mut p = planned("wide-floor").clone();
    retain_executions(&mut p, |e| e.candidate.family != PathFamily::Floor);
    let a = verify_combined_plan(&p).unwrap();
    assert_eq!(a.status, PlanStatus::Incomplete);
    assert!(a.missing_floor_beyond_tolerance.area_mm2() > 1.);
    assert!(
        a.diagnostics
            .iter()
            .any(|d| d.code == "INCOMPLETE_VBIT_FLOOR")
    );
    assert_eq!(a.max_sampled_unavoidable_residual_mm, 0.);
}
#[test]
fn deleting_a_depth_pass_requires_actual_prior_stock_proof() {
    let mut p = planned("narrow-channel").clone();
    retain_executions(&mut p, |e| e.pass_depth_mm > 1.);
    assert_eq!(
        verify_combined_plan(&p).unwrap_err().code,
        "VBIT_STOCK_STEPDOWN"
    );
}
#[test]
fn motion_verifier_rejects_in_stock_rapids_wrong_feed_and_wrong_tool_order() {
    let p = planned("narrow-channel");
    let i = p
        .vbit_motions
        .iter()
        .position(|m| m.kind == MotionKind::Cut)
        .unwrap();
    let mut moves = p.vbit_motions.clone();
    moves[i].kind = MotionKind::RapidXY;
    moves[i].feed_mm_min = None;
    assert_eq!(
        verify_vbit_motions(&p.endmill.job, &p.endmill.motions, &moves)
            .unwrap_err()
            .code,
        "INVALID_VBIT_MOTION"
    );
    moves = p.vbit_motions.clone();
    moves[i].feed_mm_min = Some(1.);
    assert_eq!(
        verify_vbit_motions(&p.endmill.job, &p.endmill.motions, &moves)
            .unwrap_err()
            .code,
        "INVALID_VBIT_MOTION"
    );
    let mut altered = p.clone();
    altered.transition.to_tool_id = "endmill".into();
    assert_eq!(
        verify_combined_plan(&altered).unwrap_err().code,
        "TOOL_STAGE_ORDER"
    );
}
#[test]
fn unsupported_entries_and_resource_budgets_never_claim_completion() {
    for (name, status, code) in [
        (
            "unsupported-entry",
            PlanStatus::Incomplete,
            "UNSUPPORTED_VBIT_ENTRY",
        ),
        (
            "resource-limit",
            PlanStatus::Inconclusive,
            "VBIT_MOTION_LIMIT",
        ),
    ] {
        let p = plan_combined(&fixture(name)).unwrap();
        assert_eq!(p.analysis.status, status);
        assert!(p.analysis.diagnostics.iter().any(|d| d.code == code));
        assert_eq!(
            CombinedPlan::from_json(&p.to_json().unwrap())
                .unwrap()
                .analysis
                .status,
            status
        );
    }
    let mut j = fixture("narrow-channel");
    j.vbit_planning.as_mut().unwrap().max_quality_samples = 1;
    let p = plan_combined(&j).unwrap();
    assert_eq!(p.analysis.status, PlanStatus::Inconclusive);
    assert!(
        p.analysis
            .diagnostics
            .iter()
            .any(|d| d.code == "QUALITY_SAMPLE_LIMIT")
    );
}
#[test]
fn vbit_entry_and_quality_fields_remain_explicit_during_job_migration() {
    let mut v = serde_json::to_value(fixture("wide-floor")).unwrap();
    v["schema_version"] = serde_json::json!(2);
    v.as_object_mut().unwrap().remove("vbit_planning");
    for t in v["tools"].as_array_mut().unwrap() {
        t.as_object_mut().unwrap().remove("plunge_capable");
    }
    let j = Job::from_json(&v.to_string()).unwrap();
    assert_eq!(j.schema_version, 3);
    assert!(j.vbit_planning.is_none());
    assert!(j.tools[1].plunge_capable.is_none());
    assert_eq!(plan_combined(&j).unwrap_err().code, "MISSING_VBIT_SETTINGS");
}

#[test]
fn exact_cap_center_lines_and_isolated_points_are_retained_with_guarded_depth() {
    for name in ["contact-line", "contact-point"] {
        let p = plan_combined(&fixture(name)).unwrap();
        assert_eq!(
            p.analysis.status,
            PlanStatus::Complete,
            "{name}: {:?}",
            p.analysis.diagnostics
        );
        let deepest = p
            .vbit_motions
            .iter()
            .map(|m| m.end.depth())
            .fold(0., f64::max);
        assert!((1.99 - 1e-10..=2.).contains(&deepest));
        if name == "contact-point" {
            let contact = p
                .executions
                .iter()
                .find(|e| e.final_finish && e.candidate.family == PathFamily::Contact)
                .unwrap();
            assert_eq!(contact.candidate.points.len(), 1);
            assert!(
                p.vbit_motions
                    .iter()
                    .any(|m| m.id >= contact.first_motion_id
                        && m.id < contact.end_motion_id
                        && m.kind == MotionKind::Plunge)
            );
        }
    }
}

#[test]
fn cutting_height_and_conflicting_capabilities_are_rejected() {
    let m = motion(
        Position::new(Point::new(1., 1.), 0.),
        Position::new(Point::new(1., 1.), -6.),
        MotionKind::Plunge,
    );
    assert_eq!(
        vbit_removed_depth_at(std::slice::from_ref(&m), &tool(0.), Point::new(1., 1.))
            .unwrap_err()
            .code,
        "STOCK_CUTTING_HEIGHT"
    );
    assert_eq!(
        vbit_removal_at_slice(Grid::new(0.005, 100.).unwrap(), &[m], &tool(0.), 1.)
            .unwrap_err()
            .code,
        "STOCK_CUTTING_HEIGHT"
    );
    let mut j = fixture("wide-floor");
    j.tools[0].plunge_capable = Some(false);
    assert_eq!(
        j.validate_settings().unwrap_err().code,
        "JOB_TOOL_CAPABILITY"
    );
}

#[test]
fn curved_medial_xyz_linearization_stays_within_motion_tolerance() {
    let p = planned("curved-medial");
    let query = BoundaryQuery::new(&p.endmill.job.inspect().unwrap().geometry.selected);
    let guard = 2. * p.endmill.job.import.geometry_tolerance_mm;
    let cap = p.endmill.job.operation.max_depth_mm.unwrap();
    let tolerance = p.endmill.job.tolerances.motion_tolerance_mm.unwrap();
    for b in p
        .analysis
        .medial_axis
        .branches
        .iter()
        .filter(|b| b.curve.is_curved())
    {
        let segments: Vec<_> = p
            .executions
            .iter()
            .filter(|e| {
                e.final_finish
                    && e.candidate.family == PathFamily::Medial
                    && e.candidate.source_branch == Some(b.id)
            })
            .flat_map(|e| e.candidate.points.windows(2))
            .collect();
        for i in 0..=500 {
            let q = b.curve.evaluate(i as f64 / 500.).unwrap();
            let depth = (query.sample(q).unwrap().distance_mm - guard).clamp(0., cap);
            if depth == 0. || depth == cap {
                continue;
            }
            let v = Position::new(q, -depth);
            let distance = segments
                .iter()
                .map(|w| {
                    let a = w[0];
                    let b = w[1];
                    let dx = b.x - a.x;
                    let dy = b.y - a.y;
                    let dz = b.z - a.z;
                    let t = (((v.x - a.x) * dx + (v.y - a.y) * dy + (v.z - a.z) * dz)
                        / (dx * dx + dy * dy + dz * dz))
                        .clamp(0., 1.);
                    let q = a.lerp(b, t);
                    ((v.x - q.x).powi(2) + (v.y - q.y).powi(2) + (v.z - q.z).powi(2)).sqrt()
                })
                .fold(f64::INFINITY, f64::min);
            assert!(
                distance <= tolerance,
                "branch {}: XYZ distance {distance}",
                b.id
            );
        }
    }
}
#[test]
fn obtuse_vbit_and_placed_artwork_preserve_continuous_clearance() {
    let mut j = fixture("narrow-channel");
    if let Some(cam_core::job::ToolGeometry::Vbit(s)) = &mut j.tools[1].geometry {
        s.included_angle_deg = 120.;
        s.max_cutting_diameter_mm = 20.;
    }
    j.import.placement.rotation_deg = 31.;
    j.import.placement.origin_mm = Point::new(80., 27.);
    let p = plan_combined(&j).unwrap();
    assert_eq!(
        p.analysis.status,
        PlanStatus::Complete,
        "{:?}",
        p.analysis.diagnostics
    );
    assert!(p.analysis.max_sampled_missed_reachable_mm < 0.05);
}
