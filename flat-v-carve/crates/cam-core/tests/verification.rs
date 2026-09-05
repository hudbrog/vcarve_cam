use cam_core::{
    geometry::{BoundaryQuery, Point},
    job::Job,
    motion::{Motion, MotionKind, Position},
    pocket::PlanStatus,
    vcarve::{CombinedPlan, PathFamily, plan_combined},
    verification::{VerificationOptions, VerificationStatus, verify_motions, verify_plan},
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
fn narrow() -> &'static CombinedPlan {
    planned("narrow-channel")
}
fn planned(name: &str) -> &'static CombinedPlan {
    const NAMES: [&str; 5] = [
        "narrow-channel",
        "wide-floor",
        "island",
        "finite-tip",
        "curved-medial",
    ];
    static PLANS: [OnceLock<CombinedPlan>; 5] = [const { OnceLock::new() }; 5];
    let i = NAMES.iter().position(|n| *n == name).unwrap();
    PLANS[i].get_or_init(|| plan_combined(&fixture(name)).unwrap())
}
#[test]
fn continuous_narrow_channel_is_bounded_without_a_sample_lattice() {
    let p = narrow();
    let r = verify_motions(
        &p.endmill.job,
        &p.endmill.motions,
        &p.vbit_motions,
        &VerificationOptions::default(),
    )
    .unwrap();
    eprintln!(
        "status {:?}, cells {}, errors {:?}, first {:?}",
        r.status,
        r.original.evaluated_cells,
        r.original.bounds,
        r.original.findings.first()
    );
    assert_eq!(r.status, VerificationStatus::Passed);
    assert_eq!(r.original.unresolved_cells, 0);
    assert!(r.original.bounds.other_reachable_residual_mm.upper <= 0.05);
    assert_eq!(r.original.bounds.unreachable_detail_mm.upper, 0.);
}

#[test]
fn representative_combined_stock_has_bounded_finish_errors() {
    for name in ["wide-floor", "island", "finite-tip", "curved-medial"] {
        let p = planned(name);
        let r = verify_motions(
            &p.endmill.job,
            &p.endmill.motions,
            &p.vbit_motions,
            &VerificationOptions {
                decimal_places: Some(6),
                ..Default::default()
            },
        )
        .unwrap();
        eprintln!(
            "{name}: {:?}, cells {}, errors {:?}, first {:?}",
            r.status,
            r.original.evaluated_cells,
            r.original.bounds,
            r.original.findings.first()
        );
        assert_eq!(r.status, VerificationStatus::Passed, "{name}");
        assert!(r.original.maximum_error_uncertainty_mm <= 0.05);
        assert!(r.original.evaluated_cells <= VerificationOptions::default().max_cells);
        if name == "wide-floor" {
            // Long pointed-bit lanes have spacing 1.6 * ridge = 0.24 mm.
            // Their analytic maximum ridge is spacing / (2*tan(45 degrees)).
            assert!(r.original.bounds.floor_ridge_mm.lower <= 0.12);
            assert!(r.original.bounds.floor_ridge_mm.upper >= 0.12);
        }
    }
}

#[test]
fn cell_depth_and_slice_budgets_cannot_create_a_coarse_false_pass() {
    let p = narrow();
    for options in [
        VerificationOptions {
            max_cells: 1,
            ..Default::default()
        },
        VerificationOptions {
            max_depth: 1,
            ..Default::default()
        },
        VerificationOptions {
            max_depth_bands: 1,
            ..Default::default()
        },
    ] {
        let r = verify_motions(
            &p.endmill.job,
            &p.endmill.motions,
            &p.vbit_motions,
            &options,
        )
        .unwrap();
        assert_eq!(
            r.status,
            VerificationStatus::Inconclusive,
            "{:?}",
            r.original.findings
        );
        assert!(r.original.evaluated_cells <= options.max_cells);
        assert!(r.original.depth_bands.len() <= options.max_depth_bands);
        assert!(!r.original.findings.is_empty());
    }
    let r = verify_motions(
        &fixture("disconnected"),
        &[],
        &[],
        &VerificationOptions {
            max_cells: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_ne!(
        r.status,
        VerificationStatus::Passed,
        "empty middle of a coarse box is not coverage of disconnected details"
    );
}

#[test]
fn decreasing_uncertainty_refines_the_analytic_channel_and_changes_identity() {
    let p = narrow();
    let mut previous = f64::INFINITY;
    let mut identities = std::collections::BTreeSet::new();
    for tolerance in [0.1, 0.05, 0.02] {
        let mut job = p.endmill.job.clone();
        job.tolerances.verification_tolerance_mm = Some(tolerance);
        let r = verify_motions(
            &job,
            &p.endmill.motions,
            &p.vbit_motions,
            &VerificationOptions::default(),
        )
        .unwrap();
        assert_eq!(r.status, VerificationStatus::Passed);
        let b = r.original.bounds.other_reachable_residual_mm;
        assert!(b.lower <= 0.01 && b.upper >= 0.01);
        assert!(b.upper <= tolerance && b.upper <= previous);
        assert!(r.original.maximum_error_uncertainty_mm <= tolerance);
        previous = b.upper;
        identities.insert(r.verification_fingerprint);
    }
    assert_eq!(identities.len(), 3);
}

#[test]
fn missed_reachable_stock_is_not_hidden_by_a_generous_detail_allowance() {
    let p = narrow();
    let mut job = p.endmill.job.clone();
    job.operation.max_detail_residual_mm = Some(100.);
    let mut motions = p.vbit_motions.clone();
    for m in &mut motions {
        if m.start.z < 0. {
            m.start.z *= 0.6;
        }
        if m.end.z < 0. {
            m.end.z *= 0.6;
        }
    }
    let r = verify_motions(&job, &[], &motions, &VerificationOptions::default()).unwrap();
    assert_eq!(r.status, VerificationStatus::Failed);
    assert_eq!(r.original.bounds.unreachable_detail_mm.upper, 0.);
    assert!(r.original.bounds.other_reachable_residual_mm.lower > 0.05);
    assert!(
        r.original
            .findings
            .iter()
            .any(|f| f.code == "M5_REACHABLE_RESIDUAL" && f.cell.is_some())
    );
}

#[test]
fn finite_tip_detail_limit_requires_geometric_evidence() {
    let p = planned("finite-tip");
    let mut job = p.endmill.job.clone();
    job.operation.max_detail_residual_mm = Some(0.05);
    let r = verify_motions(
        &job,
        &p.endmill.motions,
        &p.vbit_motions,
        &VerificationOptions::default(),
    )
    .unwrap();
    assert_eq!(r.status, VerificationStatus::Failed);
    assert!(r.original.bounds.unreachable_detail_mm.lower > 0.05);
    assert!(
        r.original
            .findings
            .iter()
            .any(|f| f.code == "M5_UNREACHABLE_DETAIL")
    );
    job.operation.max_detail_residual_mm = Some(100.);
    let missing = verify_motions(&job, &[], &[], &VerificationOptions::default()).unwrap();
    assert_eq!(missing.status, VerificationStatus::Failed);
    assert!(
        missing
            .original
            .findings
            .iter()
            .any(|f| matches!(f.code.as_str(), "M5_FLOOR_RIDGE" | "M5_REACHABLE_RESIDUAL"))
    );
}

fn excursion(job: &Job, a: Point, b: Point, depth: f64) -> Vec<Motion> {
    let settings = job.endmill_planning.as_ref().unwrap();
    let tool = job
        .tools
        .iter()
        .find(|t| t.id == job.operation.vbit_id)
        .unwrap();
    let mut previous = Position::new(settings.start_xy_mm, settings.clearance_z_mm);
    [
        (
            MotionKind::RapidXY,
            Position::new(a, settings.clearance_z_mm),
        ),
        (MotionKind::Approach, Position::new(a, 0.)),
        (MotionKind::Plunge, Position::new(a, -depth)),
        (MotionKind::Cut, Position::new(b, -depth)),
        (
            MotionKind::RapidRetract,
            Position::new(b, settings.clearance_z_mm),
        ),
    ]
    .into_iter()
    .enumerate()
    .map(|(id, (kind, end))| {
        let start = previous;
        previous = end;
        Motion {
            id,
            tool_id: tool.id.clone(),
            operation_id: job.operation.id.clone(),
            layer: 0,
            kind,
            start,
            end,
            feed_mm_min: if kind.rapid() {
                None
            } else if kind == MotionKind::Cut {
                tool.cutting_feed_mm_min
            } else {
                tool.plunge_feed_mm_min
            },
        }
    })
    .collect()
}

#[test]
fn continuous_island_crossing_is_a_located_overcut_with_valid_endpoints() {
    let job = fixture("island");
    let q = BoundaryQuery::new(&job.inspect().unwrap().geometry.selected);
    let a = Point::new(10., 15.);
    let b = Point::new(30., 15.);
    assert!(q.sample(a).unwrap().signed_distance_mm > 1.);
    assert!(q.sample(b).unwrap().signed_distance_mm > 1.);
    assert!(q.sample(a.lerp(b, 0.5)).unwrap().signed_distance_mm < 0.);
    let r = verify_motions(
        &job,
        &[],
        &excursion(&job, a, b, 1.),
        &VerificationOptions::default(),
    )
    .unwrap();
    assert_eq!(r.status, VerificationStatus::Failed);
    assert!(r.original.bounds.overcut_mm.lower > 0.9);
    assert!(
        r.original
            .findings
            .iter()
            .any(|f| f.code == "M5_OVERCUT" && f.location.x == 20.)
    );
}

#[test]
fn in_stock_rapid_links_are_rejected_independently_of_cutting_coverage() {
    let p = narrow();
    let mut moves = p.vbit_motions.clone();
    moves
        .iter_mut()
        .find(|m| m.kind == MotionKind::Cut)
        .unwrap()
        .kind = MotionKind::RapidXY;
    let r = verify_motions(
        &p.endmill.job,
        &p.endmill.motions,
        &moves,
        &VerificationOptions::default(),
    )
    .unwrap();
    assert_eq!(r.status, VerificationStatus::Failed);
    assert!(
        r.original
            .findings
            .iter()
            .any(|f| f.code == "M5_INVALID_MOTION" && f.motion_id.is_some())
    );
}

#[test]
fn formatted_coordinates_are_rechecked_and_collapsed_moves_are_retained_as_errors() {
    let p = narrow();
    for (places, expected) in [
        (6, VerificationStatus::Passed),
        (0, VerificationStatus::Failed),
    ] {
        let options = VerificationOptions {
            decimal_places: Some(places),
            ..Default::default()
        };
        let r = verify_motions(
            &p.endmill.job,
            &p.endmill.motions,
            &p.vbit_motions,
            &options,
        )
        .unwrap();
        assert_eq!(r.original.status, VerificationStatus::Passed);
        assert_eq!(
            r.status,
            expected,
            "{:?}",
            r.rounded.as_ref().unwrap().verification.findings
        );
        let rounded = r.rounded.unwrap();
        assert!(rounded.maximum_coordinate_change_mm <= rounded.coordinate_quantum_mm / 2. + 1e-12);
        assert_eq!(
            rounded.verification.checked_motion_count,
            p.vbit_motions.len() + p.endmill.motions.len()
        );
        if places == 0 {
            assert!(
                rounded
                    .verification
                    .findings
                    .iter()
                    .any(|f| f.code.starts_with("ROUNDING_"))
            );
        }
    }
}

#[test]
fn rounding_can_gouge_a_translated_boundary_without_reversing_or_collapsing_a_move() {
    let mut job = fixture("narrow-channel");
    job.import.placement.origin_mm.x = -0.06;
    let moves = excursion(&job, Point::new(0.25, 11.), Point::new(0.25, 29.), 0.18);
    let r = verify_motions(
        &job,
        &[],
        &moves,
        &VerificationOptions {
            decimal_places: Some(1),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(r.original.bounds.overcut_mm.upper < 0.05);
    let rounded = r.rounded.unwrap();
    assert_eq!(rounded.verification.status, VerificationStatus::Failed);
    assert!(rounded.verification.bounds.overcut_mm.lower > 0.05);
    assert!(
        rounded
            .verification
            .findings
            .iter()
            .any(|f| f.code == "M5_OVERCUT")
    );
    assert!(
        rounded
            .verification
            .findings
            .iter()
            .all(|f| !f.code.starts_with("ROUNDING_"))
    );
}

#[test]
fn rectangle_distance_enclosures_include_boundary_crossings_and_contained_islands() {
    let job = fixture("island");
    let q = BoundaryQuery::new(&job.inspect().unwrap().geometry.selected);
    for (min, max) in [
        (Point::new(0., 0.), Point::new(40., 30.)),
        (Point::new(1., 10.), Point::new(2., 20.)),
        (Point::new(15., 10.), Point::new(25., 20.)),
        (Point::new(-2., -1.), Point::new(2., 2.)),
    ] {
        let (lo, hi) = q.box_signed_distance_bounds(min, max).unwrap();
        for y in 0..=20 {
            for x in 0..=20 {
                let p = Point::new(
                    min.x + (max.x - min.x) * x as f64 / 20.,
                    min.y + (max.y - min.y) * y as f64 / 20.,
                );
                let d = q.sample(p).unwrap().signed_distance_mm;
                assert!(lo <= d && d <= hi, "{lo} <= {d} <= {hi} at {p:?}");
            }
        }
    }
}

#[test]
fn depth_band_sweeps_enclose_analytic_stationary_cone_sections_and_volume() {
    let mut job = fixture("wide-floor");
    job.operation.max_depth_mm = Some(1.);
    let mut moves = excursion(&job, Point::new(10., 15.), Point::new(10.01, 15.), 1.);
    // A stationary plunge + retract gives disks of radius (1-t) at each depth.
    moves.remove(3);
    moves.last_mut().unwrap().start = Position::new(Point::new(10., 15.), -1.);
    moves.last_mut().unwrap().end = Position::new(Point::new(10., 15.), 5.);
    moves.last_mut().unwrap().id = 3;
    let r = verify_motions(&job, &[], &moves, &VerificationOptions::default()).unwrap();
    for band in &r.original.depth_bands {
        for t in [band.from_depth_mm, band.to_depth_mm] {
            let area = std::f64::consts::PI * (1. - t).max(0.).powi(2);
            assert!(band.removed_area_mm2.lower <= area + 1e-10);
            assert!(band.removed_area_mm2.upper >= area - 1e-10);
        }
    }
    assert!(
        r.original
            .depth_bands
            .windows(2)
            .all(|w| w[0].to_depth_mm == w[1].from_depth_mm)
    );
    assert!(
        r.original.bounds.residual_volume_mm3.lower <= r.original.bounds.residual_volume_mm3.upper
    );
}

#[test]
fn reports_are_deterministic_and_cached_analysis_cannot_authorize_changed_tools() {
    let mut p = narrow().clone();
    let options = VerificationOptions::default();
    let first = verify_plan(&p, &options).unwrap();
    p.analysis.status = PlanStatus::Incomplete;
    p.analysis.samples.clear();
    let second = verify_plan(&p, &options).unwrap();
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    p.endmill.job.operation.max_depth_mm = Some(1.9);
    assert_eq!(verify_plan(&p, &options).unwrap_err().code, "STALE_PLAN");
}

#[test]
fn invalid_resource_and_rounding_options_are_rejected_before_work() {
    let p = narrow();
    for options in [
        VerificationOptions {
            max_cells: 0,
            ..Default::default()
        },
        VerificationOptions {
            decimal_places: Some(10),
            ..Default::default()
        },
        VerificationOptions {
            reachability_max_cells: 0,
            ..Default::default()
        },
        VerificationOptions {
            max_findings: 0,
            ..Default::default()
        },
    ] {
        assert_eq!(
            verify_motions(&p.endmill.job, &[], &[], &options)
                .unwrap_err()
                .code,
            "VERIFICATION_OPTIONS"
        );
    }
}

#[test]
fn deleting_one_floor_lane_exposes_a_missed_strip_with_valid_motion_continuity() {
    let p = planned("wide-floor");
    let mut ys: Vec<_> = p
        .executions
        .iter()
        .filter(|e| e.candidate.family == PathFamily::Floor && !e.pruned_air)
        .map(|e| e.candidate.points[0].y)
        .collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    let y = ys[2];
    let removed: std::collections::BTreeSet<_> = p
        .executions
        .iter()
        .filter(|e| e.candidate.family == PathFamily::Floor && e.candidate.points[0].y == y)
        .flat_map(|e| e.first_motion_id..e.end_motion_id)
        .collect();
    assert!(!removed.is_empty());
    let mut moves: Vec<Motion> = vec![];
    let mut previous = p.transition.position;
    for original in p.vbit_motions.iter().filter(|m| !removed.contains(&m.id)) {
        let mut m = original.clone();
        if m.start != previous {
            assert_eq!(previous.z, 5.);
            assert_eq!(m.start.z, 5.);
            if m.kind == MotionKind::RapidXY {
                m.start = previous;
            } else {
                let mut link = m.clone();
                link.start = previous;
                link.end = m.start;
                link.kind = MotionKind::RapidXY;
                link.feed_mm_min = None;
                link.id = p.endmill.motions.len() + moves.len();
                moves.push(link);
            }
        }
        if m.start == m.end {
            continue;
        }
        m.id = p.endmill.motions.len() + moves.len();
        previous = m.end;
        moves.push(m);
    }
    let r = verify_motions(
        &p.endmill.job,
        &p.endmill.motions,
        &moves,
        &VerificationOptions::default(),
    )
    .unwrap();
    assert_eq!(r.status, VerificationStatus::Failed);
    assert!(
        r.original
            .findings
            .iter()
            .all(|f| f.code != "M5_INVALID_MOTION")
    );
    assert!(r.original.bounds.floor_ridge_mm.lower > 0.15);
    assert_eq!(r.original.bounds.unreachable_detail_mm.upper, 0.);
}

#[test]
fn strict_zero_ridge_does_not_hide_guarded_depth_residue_at_cap_contacts() {
    for name in ["contact-line", "contact-point"] {
        let p = plan_combined(&fixture(name)).unwrap();
        assert_eq!(p.analysis.status, PlanStatus::Complete);
        let r = verify_motions(
            &p.endmill.job,
            &p.endmill.motions,
            &p.vbit_motions,
            &VerificationOptions {
                decimal_places: Some(6),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.status, VerificationStatus::Failed);
        assert!(r.rounded.unwrap().verification.bounds.floor_ridge_mm.lower > 0.009);
    }
}
