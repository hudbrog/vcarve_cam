use cam_core::{
    geometry::{BoundaryQuery, Grid, Point, Region},
    job::{Job, ToolGeometry},
    motion::MotionKind,
    vcarve::{PathFamily, plan_combined},
    verification::{VerificationOptions, VerificationStatus, verify_plan},
};

#[test]
fn polyline_clipping_splits_at_holes_without_connecting_cleared_intervals() {
    let grid = Grid::new(0.005, 30.).unwrap();
    let region = Region::from_rings(
        grid,
        &[
            vec![
                Point::new(0., 0.),
                Point::new(20., 0.),
                Point::new(20., 10.),
                Point::new(0., 10.),
            ],
            vec![
                Point::new(8., 2.),
                Point::new(12., 2.),
                Point::new(12., 8.),
                Point::new(8., 8.),
            ],
        ],
    )
    .unwrap();
    let line = [Point::new(-1., 5.), Point::new(21., 5.)];
    let paths = region.clip_polyline(&line).unwrap();
    assert_eq!(
        paths,
        vec![
            vec![Point::new(0., 5.), Point::new(8., 5.)],
            vec![Point::new(12., 5.), Point::new(20., 5.)]
        ]
    );
    assert_eq!(paths, region.clip_polyline(&[line[1], line[0]]).unwrap());
    let loop_path = [
        Point::new(1., 1.),
        Point::new(5., 1.),
        Point::new(5., 9.),
        Point::new(1., 9.),
        Point::new(1., 1.),
    ];
    let clipped = region.clip_polyline(&loop_path).unwrap();
    assert_eq!(clipped.len(), 1);
    assert_eq!(clipped[0].first(), clipped[0].last());
    assert_eq!(clipped[0].len(), 5);
    assert!(
        region
            .clip_polyline(&[Point::new(f64::NAN, 0.), Point::new(1., 1.)])
            .is_err()
    );
}

#[test]
fn island_floor_cleanup_stays_near_residual_boundaries_and_preserves_finish() {
    let job = Job::from_json(include_str!("../../../fixtures/m4/island.json")).unwrap();
    let plan = plan_combined(&job).unwrap();
    let query = BoundaryQuery::new(&job.inspect().unwrap().geometry.selected);
    let floor: Vec<_> = plan
        .executions
        .iter()
        .filter(|e| e.candidate.family == PathFamily::Floor && !e.pruned_air)
        .collect();
    assert!(
        !floor.is_empty(),
        "wall allowance still needs floor cleanup"
    );
    for e in &floor {
        for p in &e.candidate.points {
            assert!(
                query.sample(p.xy()).unwrap().distance_mm < 3.6,
                "floor cleanup strayed into the already-cleared interior: {p:?}"
            );
        }
    }
    let cuts: f64 = plan
        .vbit_motions
        .iter()
        .filter(|m| m.kind == MotionKind::Cut)
        .map(|m| m.start.xy().distance(m.end.xy()))
        .sum();
    let entries = plan
        .vbit_motions
        .iter()
        .filter(|m| m.kind == MotionKind::Approach)
        .count();
    eprintln!(
        "island: {} V-bit motions, {entries} entries, {cuts:.2} mm cutting",
        plan.vbit_motions.len()
    );
    assert!(
        entries < 100,
        "the old full-floor raster required 1000 excursions"
    );
    assert!(
        cuts < 3000.,
        "the old raster required 6477 mm of V-bit cuts"
    );
    assert!(
        plan.executions
            .iter()
            .any(|e| e.final_finish && e.candidate.family == PathFamily::Boundary)
    );
    assert!(
        plan.executions
            .iter()
            .any(|e| e.final_finish && e.candidate.family == PathFamily::Medial)
    );
    let report = verify_plan(
        &plan,
        &VerificationOptions {
            decimal_places: Some(6),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        report.status,
        VerificationStatus::Passed,
        "{:?}",
        (
            report.original.findings,
            report.rounded.map(|r| r.verification.findings)
        )
    );
}

#[test]
fn acute_floor_with_no_endmill_access_still_gets_complete_vbit_clearing() {
    let mut job = Job::from_json(include_str!("../../../fixtures/m4/narrow-channel.json")).unwrap();
    job.source.svg = job
        .source
        .svg
        .replace("M0 0h3v20h-3z", "M0 0 L12 0 L6 10 z");
    if let Some(ToolGeometry::Endmill(spec)) = &mut job.tools[0].geometry {
        spec.diameter_mm = 16.;
    }
    let plan = plan_combined(&job).unwrap();
    assert!(plan.endmill.motions.is_empty());
    assert!(
        plan.executions
            .iter()
            .any(|e| e.candidate.family == PathFamily::Floor)
    );
    let report = verify_plan(
        &plan,
        &VerificationOptions {
            decimal_places: Some(6),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        report.status,
        VerificationStatus::Passed,
        "{:?}",
        report.original.findings
    );
}
