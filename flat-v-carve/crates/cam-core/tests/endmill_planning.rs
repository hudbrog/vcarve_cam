use cam_core::{
    job::Job,
    pocket::{PlanStatus, plan_endmill},
};
fn fixture(name: &str) -> Job {
    Job::from_json(
        &std::fs::read_to_string(format!(
            "{}/../../fixtures/m3/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap(),
    )
    .unwrap()
}
#[test]
fn standard_clearing_fixtures() {
    for name in [
        "rectangle",
        "island",
        "disconnected",
        "ramp",
        "deepest-region",
    ] {
        let plan = plan_endmill(&fixture(name)).unwrap();
        eprintln!(
            "{name}: {:?}, {} moves, {:?}",
            plan.analysis.status,
            plan.motions.len(),
            plan.analysis.diagnostics
        );
        for l in &plan.analysis.layers {
            eprintln!(
                "depth {}: removed {}, missing {}, overcut {}",
                l.depth_mm,
                l.removal.lower.area_mm2(),
                l.missing_floor_beyond_tolerance.area_mm2(),
                l.possible_overcut.area_mm2()
            );
        }
        assert_eq!(plan.analysis.status, PlanStatus::Complete, "{name}");
    }
}

use cam_core::{
    geometry::{BoundaryQuery, Grid, Point, PointLocation, Region},
    job::ToolGeometry,
    motion::{Motion, MotionKind, Position},
    pocket::{EndmillPlan, verify_endmill_motions},
    stock::{capsule_bounds, removal_at_slice, removed_depth_at},
};

#[test]
fn no_access_exact_fit_unsupported_entry_and_limits_are_distinct() {
    for (name, status, code) in [
        ("no-access", PlanStatus::Empty, None),
        (
            "exact-fit",
            PlanStatus::Inconclusive,
            Some("ZERO_MARGIN_ACCESS"),
        ),
        (
            "narrow-margin",
            PlanStatus::Inconclusive,
            Some("NO_NUMERICAL_MARGIN"),
        ),
        (
            "unsupported-entry",
            PlanStatus::Incomplete,
            Some("UNSUPPORTED_ENTRY"),
        ),
        (
            "resource-limit",
            PlanStatus::Inconclusive,
            Some("PLANNING_RESOURCE_LIMIT"),
        ),
    ] {
        let plan = plan_endmill(&fixture(name)).unwrap();
        assert_eq!(
            plan.analysis.status, status,
            "{name}: {:?}",
            plan.analysis.diagnostics
        );
        if let Some(code) = code {
            assert!(
                plan.analysis.diagnostics.iter().any(|d| d.code == code),
                "{name}"
            );
        }
        if matches!(name, "no-access" | "unsupported-entry" | "narrow-margin") {
            assert!(plan.motions.is_empty());
        }
        let loaded = EndmillPlan::from_json(&plan.to_json().unwrap()).unwrap();
        assert_eq!(loaded.analysis.status, status, "reload {name}");
    }
}

#[test]
fn saved_reports_are_recomputed_and_identity_changes_rejected() {
    let plan = plan_endmill(&fixture("rectangle")).unwrap();
    let saved = plan.to_json().unwrap();
    assert_eq!(
        EndmillPlan::from_json(&saved).unwrap().to_json().unwrap(),
        saved
    );
    let mut data: serde_json::Value = serde_json::from_str(&saved).unwrap();
    data["analysis"] = serde_json::json!({"status":"empty","layers":[]});
    data["spindle_rpm"] = serde_json::json!(1);
    let replay = EndmillPlan::from_json(&data.to_string()).unwrap();
    let streamed = EndmillPlan::from_reader(data.to_string().as_bytes()).unwrap();
    assert_eq!(streamed.to_json().unwrap(), replay.to_json().unwrap());
    assert_eq!(replay.analysis.status, PlanStatus::Complete);
    assert_eq!(replay.spindle_rpm, 10000.);
    assert_eq!(replay.analysis.layers.len(), 2);
    data["job"]["operation"]["wall_allowance_mm"] = serde_json::json!(0.8);
    assert_eq!(
        EndmillPlan::from_json(&data.to_string()).unwrap_err().code,
        "STALE_PLAN"
    );
    data = serde_json::from_str(&saved).unwrap();
    data["motions"][0]["end"]["x"] = serde_json::json!(1.);
    assert_eq!(
        EndmillPlan::from_json(&data.to_string()).unwrap_err().code,
        "STALE_PLAN"
    );
    data = serde_json::from_str(&saved).unwrap();
    data["generation_issues"] =
        serde_json::json!([{"code":"PLANNING_RESOURCE_LIMIT","message":"partial"}]);
    assert_eq!(
        EndmillPlan::from_json(&data.to_string()).unwrap_err().code,
        "STALE_PLAN"
    );
    data["engine_version"] = serde_json::json!("0.3.0");
    assert_eq!(
        EndmillPlan::from_json(&data.to_string()).unwrap_err().code,
        "PLAN_VERSION"
    );
}

fn excursion(job: &Job, a: Point, b: Point) -> Vec<Motion> {
    let mut start = Position::new(Point::new(0., 0.), 5.);
    [
        (MotionKind::RapidXY, Position::new(a, 5.), None),
        (MotionKind::Approach, Position::new(a, 0.), Some(100.)),
        (MotionKind::Plunge, Position::new(a, -1.), Some(100.)),
        (MotionKind::Cut, Position::new(b, -1.), Some(300.)),
        (MotionKind::RapidRetract, Position::new(b, 5.), None),
    ]
    .into_iter()
    .enumerate()
    .map(|(id, (kind, end, feed))| {
        let m = Motion {
            id,
            tool_id: job.operation.endmill_id.clone(),
            operation_id: job.operation.id.clone(),
            layer: 0,
            kind,
            start,
            end,
            feed_mm_min: feed,
        };
        start = end;
        m
    })
    .collect()
}

#[test]
fn whole_segment_check_catches_crossing_island_with_valid_endpoints() {
    let mut job = fixture("island");
    job.operation.max_depth_mm = Some(1.);
    let a = Point::new(8., 15.);
    let b = Point::new(32., 15.);
    let query = BoundaryQuery::new(&job.inspect().unwrap().geometry.selected);
    for p in [a, b] {
        assert!(query.sample(p).unwrap().signed_distance_mm > 3.5);
    }
    assert_eq!(
        verify_endmill_motions(&job, &excursion(&job, a, b))
            .unwrap_err()
            .code,
        "MOTION_CLEARANCE"
    );
}

#[test]
fn clearance_checker_rejects_wall_gouges_and_allowance_violations() {
    let mut job = fixture("rectangle");
    job.operation.max_depth_mm = Some(1.);
    // x=3.25 clears the nominal cylinder but violates the 0.5 mm allowance.
    let moves = excursion(&job, Point::new(3.25, 17.), Point::new(3.25, 23.));
    assert_eq!(
        verify_endmill_motions(&job, &moves).unwrap_err().code,
        "MOTION_CLEARANCE"
    );
}

#[test]
fn deleting_reachable_clearing_leaves_measured_missing_floor() {
    let job = fixture("rectangle");
    let plan = plan_endmill(&job).unwrap();
    let end = plan
        .motions
        .iter()
        .position(|m| m.kind == MotionKind::RapidRetract)
        .unwrap();
    let partial = verify_endmill_motions(&job, &plan.motions[..=end]).unwrap();
    assert_eq!(partial.status, PlanStatus::Incomplete);
    assert!(partial.layers[0].missing_floor_beyond_tolerance.area_mm2() > 100.);
    assert!(partial.layers[1].removal.lower.rings().is_empty());
    assert!(
        partial
            .diagnostics
            .iter()
            .any(|d| d.code == "INCOMPLETE_FLOOR_COVERAGE")
    );
}

#[test]
fn motion_contract_rejects_in_stock_rapids_wrong_feeds_and_discontinuity() {
    let job = fixture("rectangle");
    let plan = plan_endmill(&job).unwrap();
    let cut = plan
        .motions
        .iter()
        .position(|m| m.kind == MotionKind::Cut)
        .unwrap();
    let mut bad = plan.motions.clone();
    bad[cut].kind = MotionKind::RapidXY;
    bad[cut].feed_mm_min = None;
    assert_eq!(
        verify_endmill_motions(&job, &bad).unwrap_err().code,
        "INVALID_MOTION"
    );
    bad = plan.motions.clone();
    bad[cut].feed_mm_min = Some(999.);
    assert_eq!(
        verify_endmill_motions(&job, &bad).unwrap_err().code,
        "INVALID_MOTION"
    );
    bad = plan.motions.clone();
    bad[cut].start.x += 1.;
    assert_eq!(
        verify_endmill_motions(&job, &bad).unwrap_err().code,
        "INVALID_MOTION"
    );
    bad = plan.motions.clone();
    bad[cut].end.z = f64::NAN;
    assert_eq!(
        verify_endmill_motions(&job, &bad).unwrap_err().code,
        "INVALID_MOTION"
    );
    assert_eq!(
        verify_endmill_motions(&job, &plan.motions[..plan.motions.len() - 1])
            .unwrap_err()
            .code,
        "INVALID_MOTION"
    );
}

#[test]
fn depth_dependent_layers_clear_more_upper_stock_and_finish_at_depth_cap() {
    let layered = plan_endmill(&fixture("rectangle")).unwrap();
    let deepest = plan_endmill(&fixture("deepest-region")).unwrap();
    assert!(
        layered.analysis.layers[0].removal.lower.area_mm2()
            > deepest.analysis.layers[0].removal.lower.area_mm2() + 80.
    );
    assert!(
        (layered.analysis.layers[1].removal.lower.area_mm2()
            - deepest.analysis.layers[1].removal.lower.area_mm2())
        .abs()
            < 1e-6
    );
    assert_eq!(
        removed_depth_at(&layered.motions, 2., Point::new(15., 20.)).unwrap(),
        2.
    );
    assert_eq!(
        removed_depth_at(&layered.motions, 2., Point::new(0.5, 20.)).unwrap(),
        0.
    );
    assert!(layered.analysis.layers[1].remaining_target.area_mm2() > 40.);
    let mut job = fixture("rectangle");
    job.operation.max_depth_mm = Some(2.3);
    let plan = plan_endmill(&job).unwrap();
    assert_eq!(plan.analysis.layers.last().unwrap().depth_mm, 2.3);
    assert_eq!(
        removed_depth_at(&plan.motions, 2., Point::new(15., 20.)).unwrap(),
        2.3
    );
}

#[test]
fn disconnected_links_stay_at_clearance_and_islands_remain_stock() {
    let plan = plan_endmill(&fixture("disconnected")).unwrap();
    let crossings: Vec<_> = plan
        .motions
        .iter()
        .filter(|m| m.start.x < 20. && m.end.x > 20. || m.start.x > 20. && m.end.x < 20.)
        .collect();
    assert!(!crossings.is_empty());
    assert!(
        crossings
            .iter()
            .all(|m| m.kind == MotionKind::RapidXY && m.start.z == 5. && m.end.z == 5.)
    );
    let island = plan_endmill(&fixture("island")).unwrap();
    assert_eq!(
        removed_depth_at(&island.motions, 2., Point::new(20., 15.)).unwrap(),
        0.
    );
    for l in &island.analysis.layers {
        assert_eq!(
            BoundaryQuery::new(&l.removal.upper)
                .sample(Point::new(20., 15.))
                .unwrap()
                .location,
            PointLocation::Outside
        );
    }
}

#[test]
fn ramp_entries_obey_angle_capability_and_stepdown_and_contribute_stock() {
    let job = fixture("ramp");
    let plan = plan_endmill(&job).unwrap();
    assert!(!plan.motions.iter().any(|m| m.kind == MotionKind::Plunge));
    let ramps: Vec<_> = plan
        .motions
        .iter()
        .filter(|m| m.kind == MotionKind::Ramp)
        .collect();
    assert!(!ramps.is_empty());
    for m in &ramps {
        let dz = m.start.z - m.end.z;
        assert!(dz > 0. && dz <= 0.5 + 1e-12);
        assert!(dz <= m.start.xy().distance(m.end.xy()) * 5f64.to_radians().tan() + 1e-12);
        assert_eq!(m.feed_mm_min, Some(100.));
    }
    assert!(ramps.iter().any(|m| {
        plan.analysis.layers[0]
            .removal
            .contributing_motion_ids
            .contains(&m.id)
    }));
    let mut unsupported = job.clone();
    unsupported.tools[0].ramp_capable = None;
    assert_eq!(
        plan_endmill(&unsupported).unwrap().analysis.status,
        PlanStatus::Incomplete
    );
    assert_eq!(
        verify_endmill_motions(&unsupported, &plan.motions)
            .unwrap_err()
            .code,
        "INVALID_MOTION"
    );
}

#[test]
fn capsule_polygons_bracket_analytic_disk_and_segment_area() {
    let grid = Grid::new(0.005, 100.).unwrap();
    for (a, b) in [
        (Point::new(0., 0.), Point::new(0., 0.)),
        (Point::new(0., 0.), Point::new(7., 3.)),
    ] {
        let c = capsule_bounds(grid, a, b, 2.).unwrap();
        let exact = std::f64::consts::PI * 4. + 4. * a.distance(b);
        assert!(c.lower.area_mm2() < exact && c.upper.area_mm2() > exact);
        assert!(c.radial_error_mm < 0.005);
    }
    let empty = Region::from_rings(grid, &[]).unwrap();
    assert!(empty.dilate(2.).unwrap().rings().is_empty());
}

#[test]
fn actual_ramp_clipping_and_stock_point_query_match_analytic_solution() {
    let m = Motion {
        id: 0,
        tool_id: "mill".into(),
        operation_id: "carve".into(),
        layer: 0,
        kind: MotionKind::Ramp,
        start: Position::new(Point::new(0., 0.), 0.),
        end: Position::new(Point::new(10., 0.), -2.),
        feed_mm_min: Some(100.),
    };
    let (a, b) = m.at_depth(1.).unwrap();
    assert_eq!(a, Point::new(5., 0.));
    assert_eq!(b, Point::new(10., 0.));
    let moves = [m];
    assert!((removed_depth_at(&moves, 1., Point::new(5., 0.)).unwrap() - 1.2).abs() < 1e-12);
    assert_eq!(
        removed_depth_at(&moves, 1., Point::new(5., 1.)).unwrap(),
        1.
    );
    assert_eq!(
        removed_depth_at(&moves, 1., Point::new(5., 1.01)).unwrap(),
        0.
    );
    let slice = removal_at_slice(Grid::new(0.005, 100.).unwrap(), &moves, 1., 1.).unwrap();
    let q = BoundaryQuery::new(&slice.lower);
    assert_eq!(
        q.sample(Point::new(0., 0.)).unwrap().location,
        PointLocation::Outside
    );
    assert_eq!(
        q.sample(Point::new(6., 0.)).unwrap().location,
        PointLocation::Inside
    );
    let mut plunge = moves[0].clone();
    plunge.kind = MotionKind::Plunge;
    plunge.end.x = 0.;
    plunge.end.z = -3.;
    assert_eq!(
        removed_depth_at(&[plunge], 1., Point::new(0., 0.)).unwrap(),
        3.
    );
}

#[test]
fn missing_settings_and_precision_limits_fail_before_any_motion_is_generated() {
    let mut job = fixture("rectangle");
    job.endmill_planning = None;
    assert_eq!(
        plan_endmill(&job).unwrap_err().code,
        "MISSING_PLANNING_SETTINGS"
    );
    job = fixture("rectangle");
    job.tools[0].cutting_feed_mm_min = None;
    assert_eq!(
        plan_endmill(&job).unwrap_err().code,
        "MISSING_MACHINING_SETTING"
    );
    job = fixture("rectangle");
    job.tools[0].stepover_mm = Some(3.);
    assert_eq!(plan_endmill(&job).unwrap_err().code, "STEPOVER_RANGE");
    job = fixture("rectangle");
    job.tolerances.verification_tolerance_mm = Some(0.001);
    assert_eq!(
        plan_endmill(&job).unwrap_err().code,
        "VERIFICATION_PRECISION"
    );
    job = fixture("rectangle");
    job.endmill_planning.as_mut().unwrap().max_layers = 1;
    assert_eq!(
        plan_endmill(&job).unwrap_err().code,
        "PLANNING_RESOURCE_LIMIT"
    );
    job = fixture("rectangle");
    if let Some(ToolGeometry::Endmill(s)) = &mut job.tools[0].geometry {
        s.cutting_length_mm = 1.;
    }
    assert_eq!(
        plan_endmill(&job).unwrap_err().code,
        "ENDMILL_CUTTING_LENGTH"
    );
}

#[test]
fn schema_one_jobs_migrate_without_inventing_entry_capability_or_settings() {
    let mut old = serde_json::to_value(fixture("rectangle")).unwrap();
    old["schema_version"] = serde_json::json!(1);
    old.as_object_mut().unwrap().remove("endmill_planning");
    for tool in old["tools"].as_array_mut().unwrap() {
        tool.as_object_mut().unwrap().remove("ramp_capable");
    }
    let job = Job::from_json(&old.to_string()).unwrap();
    assert_eq!(job.schema_version, 3);
    assert!(job.endmill_planning.is_none());
    assert!(job.tools.iter().all(|t| t.ramp_capable.is_none()));
    assert_eq!(
        plan_endmill(&job).unwrap_err().code,
        "MISSING_PLANNING_SETTINGS"
    );
}

#[test]
fn placement_rotation_and_translation_preserve_clearing_area_within_grid_budget() {
    let mut job = fixture("rectangle");
    let original = plan_endmill(&job).unwrap();
    job.import.placement.rotation_deg = 27.;
    job.import.placement.origin_mm = Point::new(80., -30.);
    let placed = plan_endmill(&job).unwrap();
    assert_eq!(placed.analysis.status, PlanStatus::Complete);
    for (a, b) in original.analysis.layers.iter().zip(&placed.analysis.layers) {
        assert!((a.removal.lower.area_mm2() - b.removal.lower.area_mm2()).abs() < 0.05);
    }
}

#[test]
fn public_stock_queries_reject_nonfinite_moves_and_motion_precision_is_explicit() {
    let mut job = fixture("rectangle");
    let mut moves = excursion(&job, Point::new(5., 15.), Point::new(6., 15.));
    moves[2].start.x = f64::NAN;
    assert_eq!(
        removed_depth_at(&moves, 2., Point::new(5., 15.))
            .unwrap_err()
            .code,
        "STOCK_MOTION"
    );
    assert_eq!(
        removal_at_slice(Grid::new(0.005, 100.).unwrap(), &moves, 2., 1.)
            .unwrap_err()
            .code,
        "STOCK_MOTION"
    );
    job.tolerances.motion_tolerance_mm = Some(0.0001);
    assert_eq!(plan_endmill(&job).unwrap_err().code, "MOTION_PRECISION");
}
