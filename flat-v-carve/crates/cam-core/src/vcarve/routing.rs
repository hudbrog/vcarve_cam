use super::{Candidate, Context, PathFamily, hash};
use crate::{
    geometry::{
        Point, Result, Segment,
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
}
