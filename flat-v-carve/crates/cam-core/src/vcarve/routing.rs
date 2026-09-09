use super::{Candidate, Context, PathFamily, hash};
use crate::{
    geometry::{
        Grid, GridPoint, Point, Region, Result, Segment, precision,
        spatial::{Aabb, SpatialIndex},
    },
    motion::Position,
};

/// Voronoi branches reconstruct shared vertices independently. Put their entry
/// and exit points on the existing construction grid, checking changed chords
/// with the same reserve, to prevent microscopic connecting motions.
pub(super) fn weld_endpoints(ctx: &Context, paths: &mut [Candidate]) -> Result<()> {
    let grid = ctx.target.region().grid();
    if 1. / grid.scale() > ctx.motion_tolerance / 16. {
        return Ok(());
    }
    let radius = |p: Position| {
        ctx.tool.tip_radius().mm() + p.depth() * ctx.tool.angle().slope() + ctx.guard / 2.
    };
    for c in paths {
        if c.points.is_empty() {
            continue;
        }
        let mut points = c.points.clone();
        let last = points.len() - 1;
        for i in [0, last] {
            let p = points[i];
            points[i] = Position::new(
                grid.point(grid.quantize(p.xy())?),
                (p.z * grid.scale()).round() / grid.scale(),
            );
            points[i].z = points[i].z.clamp(-ctx.target.depth_cap().mm(), 0.);
        }
        points.dedup();
        let valid = (0..points.len()).all(|i| {
            let a = points[i];
            let b = points[(i + 1).min(points.len() - 1)];
            (a == b || a.xy() != b.xy())
                && ctx
                    .target
                    .boundary()
                    .variable_radius_margin_mm(
                        Segment {
                            start: a.xy(),
                            end: b.xy(),
                        },
                        radius(a),
                        radius(b),
                    )
                    .is_ok_and(|margin| margin >= 0.)
        });
        if valid {
            c.points = points;
        }
    }
    Ok(())
}

/// Join independently reconstructed medial endpoints to immutable boundary
/// vertices before identities/executions are recorded. Anchors never move, so
/// endpoint reconciliation cannot accumulate through a chain of near neighbors.
pub(super) fn reconcile_endpoints(ctx: &Context, paths: &mut [Candidate]) {
    let anchors: Vec<_> = paths
        .iter()
        .filter(|p| p.family == PathFamily::Boundary)
        .flat_map(|p| p.points.iter().copied())
        .collect();
    let index = SpatialIndex::new(anchors.iter().map(|p| Aabb::new(p.xy(), p.xy())).collect());
    let budget = ctx.motion_tolerance.min(ctx.tolerance) / 32.;
    let slope = ctx.tool.angle().slope();
    let distance_limit = (2. / ctx.target.region().grid().scale()).min(budget * slope.min(1.));
    let radius = |p: Position| ctx.tool.tip_radius().mm() + p.depth() * slope + ctx.guard / 2.;
    for c in paths
        .iter_mut()
        .filter(|p| p.family == PathFamily::Medial && p.points.len() >= 2)
    {
        let mut points = c.points.clone();
        let last = points.len() - 1;
        for i in [0, last] {
            let p = c.points[i];
            let mut best: Option<(f64, usize)> = None;
            index.minimum(Aabb::new(p.xy(), p.xy()), |j| {
                let distance = p.xy().distance(anchors[j].xy());
                if best.is_none_or(|b| distance.total_cmp(&b.0).then(j.cmp(&b.1)).is_lt()) {
                    best = Some((distance, j));
                }
                distance
            });
            if let Some((distance, j)) = best {
                let q = anchors[j];
                let reserve =
                    128. * f64::EPSILON * p.x.abs().max(p.y.abs()).max(1.) * (1. + 1. / slope);
                // Preserve every endpoint depth, including cap intersections.
                // Parameterwise XY displacement bounds the entire changed
                // segment and its cone-height error by displacement / slope.
                if p.z == q.z
                    && distance <= distance_limit
                    && distance / slope.min(1.) + reserve <= budget
                {
                    points[i] = q;
                }
            }
        }
        // Do not collapse point features or turn a valid cut into a vertical
        // segment. Both changed endpoint chords must retain reserved clearance.
        if points != c.points
            && points.windows(2).all(|w| {
                w[0].xy() != w[1].xy()
                    && ctx
                        .target
                        .boundary()
                        .variable_radius_margin_mm(
                            Segment {
                                start: w[0].xy(),
                                end: w[1].xy(),
                            },
                            radius(w[0]),
                            radius(w[1]),
                        )
                        .is_ok_and(|m| m >= 0.)
            })
        {
            c.points = points;
        }
    }
}

/// Compare required cuts independently of traversal direction and contour start.
/// Multiplicity, family and source association remain part of the identity.
pub(super) fn candidate_key(candidate: &Candidate) -> Result<String> {
    let key = |p: Position| [p.x, p.y, p.z].map(|v| if v == 0. { 0 } else { v.to_bits() });
    let mut edges: Vec<_> = candidate
        .points
        .windows(2)
        .map(|pair| {
            let a = key(pair[0]);
            let b = key(pair[1]);
            if a <= b { [a, b] } else { [b, a] }
        })
        .collect();
    edges.sort_unstable();
    hash(&(
        candidate.family,
        candidate.source_branch,
        edges,
        if candidate.points.len() <= 1 {
            candidate.points.first().copied().map(key)
        } else {
            None
        },
    ))
}

pub(super) fn order(paths: &[Candidate], start: Point) -> Vec<Candidate> {
    let mut entries = vec![];
    for (i, c) in paths.iter().enumerate() {
        let Some(a) = c.points.first() else { continue };
        let b = c.points.last().unwrap();
        if c.points.len() > 2 && a == b {
            for (v, p) in c.points[..c.points.len() - 1].iter().enumerate() {
                entries.push((i, v, p.xy(), p.xy()));
            }
        } else {
            entries.push((i, 0, a.xy(), b.xy()));
            if a != b {
                entries.push((i, c.points.len() - 1, b.xy(), a.xy()));
            }
        }
    }
    crate::routing::nearest_order(entries, paths.len(), start)
        .into_iter()
        .map(|(i, v)| {
            let mut c = paths[i].clone();
            if c.points.len() > 2 && c.points.first() == c.points.last() {
                c.points.pop();
                c.points.rotate_left(v);
                c.points.push(c.points[0]);
            } else if v > 0 {
                c.points.reverse();
            }
            c
        })
        .collect()
}

/// Artwork features (disjoint selected components such as flower leaves) used
/// to sequence V-bit work: one feature is completed before traveling to the
/// next, so a greedy nearest walk can no longer ping-pong between components
/// whose entries converge near shared detail.
pub(super) struct FeatureIndex {
    grid: Grid,
    /// Non-hole rings as `(min_x, min_y, max_x, max_y, points)` in grid units.
    rings: Vec<(i64, i64, i64, i64, Vec<GridPoint>)>,
}
impl FeatureIndex {
    pub(super) fn new(region: &Region) -> Self {
        Self {
            grid: region.grid(),
            rings: region
                .rings()
                .iter()
                .filter(|r| !r.is_hole())
                .map(|r| {
                    let (min_x, min_y, max_x, max_y) = r.points().iter().fold(
                        (i64::MAX, i64::MAX, i64::MIN, i64::MIN),
                        |(x0, y0, x1, y1), p| (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)),
                    );
                    (min_x, min_y, max_x, max_y, r.points().to_vec())
                })
                .collect(),
        }
    }
    /// Feature id of the component containing `p`. Candidates lie strictly
    /// inside their component, so the exact parity cast decides; a point in no
    /// component (a hole, or off-region rounding) falls back to the nearest
    /// ring so the assignment stays total and deterministic.
    pub(super) fn feature(&self, p: Point) -> usize {
        let Ok(q) = self.grid.quantize(p) else {
            return 0;
        };
        let mut nearest: Option<(f64, usize)> = None;
        for (i, (x0, y0, x1, y1, ring)) in self.rings.iter().enumerate() {
            if q.x >= *x0 && q.x <= *x1 && q.y >= *y0 && q.y <= *y1 && precision::inside(q, ring) {
                return i;
            }
            let dx = if q.x < *x0 {
                *x0 - q.x
            } else if q.x > *x1 {
                q.x - *x1
            } else {
                0
            };
            let dy = if q.y < *y0 {
                *y0 - q.y
            } else if q.y > *y1 {
                q.y - *y1
            } else {
                0
            };
            let distance = (dx as f64).hypot(dy as f64) / self.grid.scale();
            if nearest.is_none_or(|(d, _)| distance < d) {
                nearest = Some((distance, i));
            }
        }
        nearest.map_or(0, |(_, i)| i)
    }
}
fn feature_of(features: &FeatureIndex, c: &Candidate) -> usize {
    // A central vertex names the component the whole path belongs to; holes
    // never contain candidates, and touching components union into one ring.
    c.points
        .get(c.points.len() / 2)
        .or_else(|| c.points.first())
        .map_or(0, |p| features.feature(p.xy()))
}

/// Order paths feature by feature, completing each component's roughing and
/// finishing locally before the next travel. Both the feature sequence and
/// the route inside a feature stay greedy-nearest; ties resolve to the lowest
/// feature id for determinism.
pub(super) fn feature_order(
    paths: &[Candidate],
    start: Point,
    features: &FeatureIndex,
) -> Vec<Candidate> {
    let mut groups: std::collections::BTreeMap<usize, Vec<Candidate>> =
        std::collections::BTreeMap::new();
    for c in paths {
        groups
            .entry(feature_of(features, c))
            .or_default()
            .push(c.clone());
    }
    let mut current = start;
    let mut ordered = vec![];
    while !groups.is_empty() {
        // Endpoint distance bounds the travel to enter a group; the full
        // per-vertex entry choice happens inside `order`.
        let mut chosen: Option<(usize, f64)> = None;
        for (&id, group) in &groups {
            let nearest = group
                .iter()
                .flat_map(|c| [c.points.first(), c.points.last()])
                .flatten()
                .map(|p| p.xy().distance(current))
                .fold(f64::INFINITY, f64::min);
            if chosen.is_none_or(|(_, d)| nearest < d) {
                chosen = Some((id, nearest));
            }
        }
        let group = groups.remove(&chosen.unwrap().0).unwrap();
        let sub = order(&group, current);
        if let Some(end) = sub.last().and_then(|c| c.points.last()) {
            current = end.xy();
        }
        ordered.extend(sub);
    }
    ordered
}

/// Order isolated cleanup plunges feature-first, then nearest, so residual
/// points do not drag the bit across the whole artwork in scan order.
pub(super) fn order_points(
    points: Vec<Point>,
    start: Point,
    features: &FeatureIndex,
) -> Vec<Point> {
    let mut groups: std::collections::BTreeMap<usize, Vec<Point>> =
        std::collections::BTreeMap::new();
    for p in points {
        groups.entry(features.feature(p)).or_default().push(p);
    }
    let mut current = start;
    let mut ordered = vec![];
    while !groups.is_empty() {
        let mut chosen: Option<(usize, f64)> = None;
        for (&id, group) in &groups {
            let nearest = group
                .iter()
                .map(|p| p.distance(current))
                .fold(f64::INFINITY, f64::min);
            if chosen.is_none_or(|(_, d)| nearest < d) {
                chosen = Some((id, nearest));
            }
        }
        let mut group = groups.remove(&chosen.unwrap().0).unwrap();
        while !group.is_empty() {
            let mut best = 0;
            for (i, p) in group.iter().enumerate() {
                if p.distance(current) < group[best].distance(current) {
                    best = i;
                }
            }
            current = group.remove(best);
            ordered.push(current);
        }
    }
    ordered
}

/// New links are confined to one stepover and one stepdown from stock top.
/// Deeper passes only join exactly coincident XYZ endpoints: they cannot assume
/// that material between independently roughed paths has already been cleared.
pub(super) fn can_link(ctx: &Context, a: Position, b: Position) -> bool {
    if a.z > 0. || b.z > 0. {
        return false;
    }
    if a == b {
        return true;
    }
    let distance = a.xy().distance(b.xy());
    let quantum = 1. / ctx.target.region().grid().scale();
    let reserve = (64.
        * f64::EPSILON
        * [a.x, a.y, b.x, b.y]
            .into_iter()
            .map(f64::abs)
            .fold(1., f64::max))
    .min(quantum / 8.);
    if a.xy() == b.xy()
        || distance + reserve < quantum
        || distance > ctx.stepover
        || a.depth().max(b.depth()) > ctx.stepdown
    {
        return false;
    }
    let r = |p: Position| {
        ctx.tool.tip_radius().mm() + p.depth() * ctx.tool.angle().slope() + ctx.guard / 2.
    };
    ctx.target
        .boundary()
        .variable_radius_margin_mm(
            Segment {
                start: a.xy(),
                end: b.xy(),
            },
            r(a),
            r(b),
        )
        .is_ok_and(|margin| margin >= 0.)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{job::Job, vcarve::PathFamily};

    #[test]
    fn adjacent_grid_points_link_on_either_side_of_binary_roundoff() {
        let job = Job::from_json(include_str!("../../../../fixtures/m4/wide-floor.json")).unwrap();
        let ctx = Context::new(&job).unwrap();
        let q = 1. / ctx.target.region().grid().scale();
        let mut below = false;
        let mut above = false;
        for x in [5., 6., 7., 10., 20.] {
            let a = Position::new(Point::new(x, 20.), -0.5);
            let b = Position::new(Point::new(x + q, 20.), -0.5);
            below |= a.xy().distance(b.xy()) < q;
            above |= a.xy().distance(b.xy()) >= q;
            assert!(can_link(&ctx, a, b));
            assert!(can_link(&ctx, b, a));
            assert!(!can_link(&ctx, a, Position::new(a.xy(), -0.6)));
            assert!(!can_link(
                &ctx,
                a,
                Position::new(Point::new(x + q / 4., 20.), -0.5)
            ));
        }
        assert!(
            below && above,
            "test must exercise both threshold roundoff directions"
        );
    }

    #[test]
    fn endpoint_reconciliation_is_bounded_immutable_and_preserves_depth_and_points() {
        let job = Job::from_json(include_str!("../../../../fixtures/m4/wide-floor.json")).unwrap();
        let ctx = Context::new(&job).unwrap();
        let q = 1. / ctx.target.region().grid().scale();
        let p = |x, y, z| Position::new(Point::new(x, y), z);
        let medial = |points| Candidate {
            family: PathFamily::Medial,
            source_branch: Some(0),
            points,
        };
        let boundary = Candidate {
            family: PathFamily::Boundary,
            source_branch: None,
            points: vec![
                p(10., 20., -1.),
                p(10., 22., -1.),
                p(12., 22., -1.),
                p(10., 20., -1.),
            ],
        };
        let mut paths = vec![
            boundary.clone(),
            medial(vec![p(8., 20., -0.5), p(10. + q, 20., -1.)]),
            medial(vec![p(8., 20., -0.5), p(10. + 2.5 * q, 20., -1.)]),
            medial(vec![p(8., 20., -0.5), p(10. + q, 20., -1. + q)]),
            medial(vec![p(10., 20., -1.), p(10. + q, 20., -1.)]),
            medial(vec![p(10. + q, 20., -1.)]),
        ];
        let original = paths.clone();
        reconcile_endpoints(&ctx, &mut paths);
        assert_eq!(paths[0].points, boundary.points);
        assert_eq!(paths[1].points[1], boundary.points[0]);
        for i in 2..paths.len() {
            assert_eq!(paths[i].points, original[i].points);
        }
        // An unchanged anchor is the only witness, even on repeated calls.
        let once = serde_json::to_value(&paths).unwrap();
        reconcile_endpoints(&ctx, &mut paths);
        assert_eq!(serde_json::to_value(&paths).unwrap(), once);
        for (&a, &b) in original[1].points.iter().zip(&paths[1].points) {
            assert_eq!(a.z, b.z);
            assert!(
                a.xy().distance(b.xy()) / ctx.tool.angle().slope().min(1.)
                    <= ctx.motion_tolerance.min(ctx.tolerance) / 32.
            );
        }
    }

    #[test]
    fn routing_preserves_required_cuts_with_rotated_loops_and_reversed_profiles() {
        let p = |x, y, z| Position::new(Point::new(x, y), z);
        let paths = vec![
            Candidate {
                family: PathFamily::Boundary,
                source_branch: None,
                points: vec![
                    p(2., 2., -1.),
                    p(2., 4., -1.),
                    p(4., 4., -1.),
                    p(4., 2., -1.),
                    p(2., 2., -1.),
                ],
            },
            Candidate {
                family: PathFamily::Medial,
                source_branch: Some(1),
                points: vec![p(1., 4., -1.), p(1., 0., -0.5)],
            },
        ];
        let ordered = order(&paths, Point::new(1., 0.));
        assert_eq!(ordered[0].points[0], paths[1].points[1]);
        assert_eq!(ordered[1].points[0], paths[0].points[1]);
        assert_eq!(
            candidate_key(&ordered[0]).unwrap(),
            candidate_key(&paths[1]).unwrap()
        );
        assert_eq!(
            candidate_key(&ordered[1]).unwrap(),
            candidate_key(&paths[0]).unwrap()
        );
        let mut changed = paths[1].clone();
        changed.points.push(changed.points[0]);
        assert_ne!(
            candidate_key(&changed).unwrap(),
            candidate_key(&paths[1]).unwrap()
        );
        changed = paths[1].clone();
        changed.points[1].z -= 0.01;
        assert_ne!(
            candidate_key(&changed).unwrap(),
            candidate_key(&paths[1]).unwrap()
        );
    }

    #[test]
    fn links_check_whole_sweep_stock_stepdown_and_representable_length() {
        let job = Job::from_json(include_str!("../../../../fixtures/m4/island.json")).unwrap();
        let mut ctx = Context::new(&job).unwrap();
        ctx.stepover = 40.;
        let p = |x, y, d: f64| Position::new(Point::new(x, y), -d);
        assert!(can_link(&ctx, p(5., 5., 0.5), p(6., 5., 0.5)));
        assert!(
            !can_link(&ctx, p(8., 15., 0.5), p(32., 15., 0.5)),
            "island crossing"
        );
        assert!(
            !can_link(
                &ctx,
                p(5., 5., ctx.stepdown + 0.1),
                p(6., 5., ctx.stepdown + 0.1)
            ),
            "uncleared stock on a deeper pass"
        );
        assert!(
            !can_link(&ctx, p(5., 5., 0.5), p(5. + 1e-12, 5., 0.5)),
            "microscopic connector"
        );
        assert!(
            can_link(&ctx, p(5., 5., 0.5), p(5., 5., 0.5)),
            "coincident paths need no connector"
        );
    }

    #[test]
    fn linked_paths_preserve_limits_and_coincident_execution_ranges() {
        use crate::{
            motion::MotionKind,
            pocket::plan_endmill,
            vcarve::{execute, verify_vbit_motions},
        };
        let job = Job::from_json(include_str!("../../../../fixtures/m4/island.json")).unwrap();
        let endmill = plan_endmill(&job).unwrap();
        let mut ctx = Context::new(&job).unwrap();
        let mut moves = vec![];
        let stock = super::super::EndmillStock::new(&endmill, ctx.mill.radius().mm()).unwrap();
        let mut executions = vec![];
        let candidate = |x| Candidate {
            family: PathFamily::Medial,
            source_branch: None,
            points: vec![
                Position::new(Point::new(x, 5.), -0.5),
                Position::new(Point::new(x + 1., 5.), -0.5),
            ],
        };
        let cap = ctx.target.depth_cap().mm();
        execute(
            &ctx,
            &stock,
            &candidate(5.),
            cap,
            true,
            &mut moves,
            &mut executions,
        )
        .unwrap();
        let original = moves.clone();
        let records = serde_json::to_value(&executions).unwrap();
        ctx.settings.max_motions = moves.len();
        let error = execute(
            &ctx,
            &stock,
            &candidate(6.1),
            cap,
            true,
            &mut moves,
            &mut executions,
        )
        .unwrap_err();
        assert_eq!(error.code, "VBIT_MOTION_LIMIT");
        assert_eq!(moves, original);
        assert_eq!(serde_json::to_value(&executions).unwrap(), records);
        assert_eq!(moves.last().unwrap().kind, MotionKind::RapidRetract);
        verify_vbit_motions(&job, &endmill.motions, &moves).unwrap();
        let point = Candidate {
            family: PathFamily::Contact,
            source_branch: None,
            points: vec![moves.last().unwrap().start],
        };
        for _ in 0..2 {
            execute(&ctx, &stock, &point, cap, true, &mut moves, &mut executions).unwrap();
        }
        assert!(
            executions
                .iter()
                .any(|e| e.first_motion_id == e.end_motion_id && !e.pruned_air)
        );
        let transition = crate::vcarve::StageTransition {
            after_motion_count: endmill.motions.len(),
            position: endmill.motions.last().unwrap().end,
            from_tool_id: job.operation.endmill_id.clone(),
            to_tool_id: job.operation.vbit_id.clone(),
        };
        super::super::verify::executions(&ctx, &endmill, &transition, &moves, &executions).unwrap();
        ctx.settings.max_motions = 1000;
        execute(
            &ctx,
            &stock,
            &candidate(6.1),
            cap,
            true,
            &mut moves,
            &mut executions,
        )
        .unwrap();
        assert_eq!(
            moves
                .iter()
                .filter(|m| m.kind == MotionKind::Approach)
                .count(),
            1
        );
        assert_eq!(
            moves[executions.last().unwrap().first_motion_id - endmill.motions.len()].kind,
            MotionKind::Cut
        );
        verify_vbit_motions(&job, &endmill.motions, &moves).unwrap();
    }

    fn two_component_job() -> (Context, FeatureIndex) {
        let mut job =
            Job::from_json(include_str!("../../../../fixtures/m4/narrow-channel.json")).unwrap();
        job.source.svg = job
            .source
            .svg
            .replace("M0 0h3v20h-3z", "M0 0h3v20h-3zM20 0h3v20h-3z");
        job.selected_region_ids = vec!["pocket::0".into(), "pocket::1".into()];
        let ctx = Context::new(&job).unwrap();
        assert_eq!(ctx.target.region().component_count(), 2);
        let features = FeatureIndex::new(ctx.target.region());
        (ctx, features)
    }

    #[test]
    fn feature_order_completes_each_disjoint_component_before_moving_on() {
        let (_ctx, features) = two_component_job();
        let left = features.feature(Point::new(1.5, 10.));
        let right = features.feature(Point::new(21.5, 10.));
        assert_ne!(left, right);
        // Outside every component resolves to the nearest ring, deterministically.
        assert_eq!(features.feature(Point::new(19., 10.)), right);
        let p = |x, y| Position::new(Point::new(x, y), -0.5);
        let medial = |a: (f64, f64), b: (f64, f64)| Candidate {
            family: PathFamily::Medial,
            source_branch: None,
            points: vec![p(a.0, a.1), p(b.0, b.1)],
        };
        // Interleave candidates from both components; feature_order must
        // untangle them into contiguous per-component blocks.
        let paths = vec![
            medial((21., 5.), (22., 6.)),
            medial((1., 15.), (2., 16.)),
            medial((21., 15.), (22., 14.)),
            medial((1., 5.), (2., 6.)),
        ];
        let ids = |ordered: &[Candidate]| {
            ordered
                .iter()
                .map(|c| features.feature(c.points[0].xy()))
                .collect::<Vec<_>>()
        };
        let ordered = feature_order(&paths, Point::new(0., 0.), &features);
        assert_eq!(ids(&ordered), vec![left, left, right, right]);
        // Starting inside the second component flips which block comes first.
        let flipped = feature_order(&paths, Point::new(21.5, 10.), &features);
        assert_eq!(ids(&flipped), vec![right, right, left, left]);
        // The route is deterministic across repeated runs.
        assert_eq!(
            ids(&feature_order(&paths, Point::new(0., 0.), &features)),
            ids(&ordered)
        );
    }

    #[test]
    fn order_points_groups_features_and_walks_nearest_within_each() {
        let (_ctx, features) = two_component_job();
        let left = features.feature(Point::new(1.5, 10.));
        let right = features.feature(Point::new(21.5, 10.));
        let points = vec![
            Point::new(21.5, 15.),
            Point::new(1.5, 15.),
            Point::new(21.5, 5.),
            Point::new(1.5, 5.),
        ];
        let ordered = order_points(points, Point::new(0., 0.), &features);
        let ids: Vec<usize> = ordered.iter().map(|p| features.feature(*p)).collect();
        assert_eq!(ids, vec![left, left, right, right]);
        assert!(ordered[0].distance(Point::new(1.5, 5.)) < f64::EPSILON);
    }

    #[test]
    #[ignore = "real flower planning locality regression"]
    fn flower_vbit_executions_complete_one_leaf_at_a_time() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../real_data");
        let job = Job::from_json(
            &std::fs::read_to_string(data.join("flower_box-svg.job-real.json")).unwrap(),
        )
        .unwrap();
        let plan = crate::vcarve::plan_combined(&job).unwrap();
        let geometry =
            crate::svg::import_svg(&job.source.svg, &job.import, Some(&job.selected_region_ids))
                .unwrap();
        let features = FeatureIndex::new(&geometry.selected);
        let sequence: Vec<usize> = plan
            .executions
            .iter()
            .filter(|e| !matches!(e.candidate.family, PathFamily::Floor))
            .filter_map(|e| e.candidate.points.first())
            .map(|p| features.feature(p.xy()))
            .collect();
        let mut runs = 0;
        let mut prev = usize::MAX;
        for &f in &sequence {
            if f != prev {
                runs += 1;
                prev = f;
            }
        }
        assert_eq!(
            runs,
            geometry.selected.component_count(),
            "each artwork component (leaf) must finish before the next one starts"
        );
    }
}
