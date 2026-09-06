use super::{Candidate, Context, hash};
use crate::{
    geometry::{Point, Result, Segment},
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
    if distance < 1. / ctx.target.region().grid().scale()
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
            &endmill,
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
            &endmill,
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
            execute(
                &ctx,
                &endmill,
                &point,
                cap,
                true,
                &mut moves,
                &mut executions,
            )
            .unwrap();
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
            &endmill,
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
