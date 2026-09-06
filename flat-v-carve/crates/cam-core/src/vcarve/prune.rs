//! Remove tessellation spokes only with a bound on the entire cutter envelope.
use super::{Candidate, Context, MedialAxis, PathFamily};
use crate::{
    geometry::{
        Point,
        spatial::{Aabb, SpatialIndex},
    },
    motion::Position,
};
use std::collections::HashSet;

/// Approximate floor/wall polylines within the existing motion budget. Every
/// replaced vertex is bounded against the replacement chord; linear
/// interpolation extends that bound to every point of each original segment.
/// The reverse coverage follows because the projected parameters start at 0
/// and end at 1. Independently check the complete new cutter sweep for safety.
pub(super) fn simplify_contours(
    ctx: &Context,
    paths: &mut [Candidate],
) -> crate::geometry::Result<()> {
    use crate::geometry::Segment;
    let slope = ctx.tool.angle().slope();
    let budget = ctx.motion_tolerance.min(ctx.tolerance) / 8.;
    for c in paths
        .iter_mut()
        .filter(|c| matches!(c.family, PathFamily::Floor | PathFamily::Boundary))
    {
        if c.points.len() <= 2 {
            continue;
        }
        let mut keep = vec![false; c.points.len()];
        keep[0] = true;
        keep[c.points.len() - 1] = true;
        let mut pending = vec![(0, c.points.len() - 1)];
        let mut remaining_checks = c.points.len().saturating_mul(32);
        while let Some((first, last)) = pending.pop() {
            if last <= first + 1 {
                continue;
            }
            // Pathological polylines can make recursive simplification
            // quadratic. Retaining an unchecked range preserves its geometry.
            let cost = last - first - 1;
            if cost > remaining_checks {
                keep[first + 1..last].fill(true);
                continue;
            }
            remaining_checks -= cost;
            let a = c.points[first];
            let b = c.points[last];
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let length2 = dx * dx + dy * dy;
            let mut worst = 0.;
            let mut split = (first + last) / 2;
            for (i, &p) in c.points.iter().enumerate().take(last).skip(first + 1) {
                let t = if length2 == 0. {
                    0.
                } else {
                    (((p.x - a.x) * dx + (p.y - a.y) * dy) / length2).clamp(0., 1.)
                };
                let q = a.lerp(b, t);
                // Bound both envelope depth error and XYZ motion error.
                let loss = (p.depth() - q.depth()).abs() + p.xy().distance(q.xy()) / slope.min(1.);
                if loss > worst {
                    worst = loss;
                    split = i;
                }
            }
            let reserve = 128.
                * f64::EPSILON
                * [a.x, a.y, a.z, b.x, b.y, b.z]
                    .into_iter()
                    .map(f64::abs)
                    .fold(1., f64::max)
                * (1. + 1. / slope);
            let radius =
                |p: Position| ctx.tool.tip_radius().mm() + p.depth() * slope + ctx.guard / 2.;
            if a.xy() != b.xy()
                && worst + reserve <= budget
                && ctx.target.boundary().variable_radius_margin_mm(
                    Segment {
                        start: a.xy(),
                        end: b.xy(),
                    },
                    radius(a),
                    radius(b),
                )? >= 0.
            {
                continue;
            }
            keep[split] = true;
            pending.push((split, last));
            pending.push((first, split));
        }
        let mut index = 0;
        c.points.retain(|_| {
            let retained = keep[index];
            index += 1;
            retained
        });
    }
    Ok(())
}

fn key(p: Position) -> [u64; 3] {
    [p.x, p.y, p.z].map(|v| if v == 0. { 0 } else { v.to_bits() })
}

/// For a fixed tool, its cone-height function is 1/slope Lipschitz in XY,
/// including a finite flat tip. At ANY stock point the cut made at p exceeds
/// the cut made at q by at most d(p)-d(q)+|p.xy-q.xy|/slope. Along a linear XYZ
/// chord that bound is convex, so checking both endpoints bounds the complete
/// sweep, not just sampled positions. Keep q as an obligatory finish witness.
fn dominated_endpoint(points: &[Position], slope: f64, budget: f64) -> Option<Position> {
    if points.len() != 2 {
        return None;
    }
    let q = if points[0].depth() >= points[1].depth() {
        points[0]
    } else {
        points[1]
    };
    envelope_dominated(points, q, slope, budget).then_some(q)
}

fn envelope_dominated(points: &[Position], q: Position, slope: f64, budget: f64) -> bool {
    points.iter().all(|p| {
        let distance = p.xy().distance(q.xy()) / slope;
        let magnitude = [p.x, p.y, p.z, q.x, q.y, q.z, distance]
            .into_iter()
            .map(f64::abs)
            .fold(1., f64::max);
        let reserve = 128. * f64::EPSILON * magnitude * (1. + 1. / slope);
        p.depth() - q.depth() + distance + reserve <= budget
    })
}

pub(super) fn redundant_spokes(ctx: &Context, axis: &MedialAxis, paths: &mut Vec<Candidate>) {
    let straight: HashSet<_> = axis
        .branches
        .iter()
        .filter(|b| !b.curve.is_curved())
        .map(|b| b.id)
        .collect();
    // Spend only one eighth of the existing accuracy budget; neither input
    // tolerance nor boundary geometry changes. Curved medial chords retain
    // their existing XYZ linearization contract.
    let budget = ctx.motion_tolerance.min(ctx.tolerance) / 8.;
    let witnesses: Vec<_> = paths
        .iter()
        .map(|c| {
            if c.family == PathFamily::Medial
                && c.source_branch.is_some_and(|id| straight.contains(&id))
            {
                dominated_endpoint(&c.points, ctx.tool.angle().slope(), budget)
            } else {
                None
            }
        })
        .collect();
    let mut retained = HashSet::new();
    for (c, witness) in paths.iter().zip(&witnesses) {
        // A floor path may be air-pruned and does not belong to final finishing.
        if witness.is_none() && c.family != PathFamily::Floor {
            retained.extend(c.points.iter().copied().map(key));
        }
    }
    // Cap intersections and polygon offsets need not reconstruct the same
    // vertex. A point on a retained final chord is also a valid witness, but
    // charge its ENTIRE displacement against the original envelope budget.
    // Only unchanged final chords enter the index, preventing error chaining.
    let segments: Vec<_> = paths
        .iter()
        .zip(&witnesses)
        .filter(|(c, w)| w.is_none() && c.family != PathFamily::Floor)
        .flat_map(|(c, _)| c.points.windows(2).map(|w| [w[0], w[1]]))
        .collect();
    let index = SpatialIndex::new(
        segments
            .iter()
            .map(|w| Aabb::new(w[0].xy(), w[1].xy()))
            .collect(),
    );
    let mut isolated = HashSet::new();
    let mut i = 0;
    paths.retain_mut(|c| {
        let path_index = i;
        i += 1;
        let Some(q) = witnesses[path_index] else {
            return true;
        };
        if retained.contains(&key(q)) {
            return false;
        }
        let slope = ctx.tool.angle().slope();
        let radius = budget * slope;
        let mut covered = false;
        // visit cannot fail: the visitor uses only finite, already validated
        // candidate positions and returns Ok unconditionally.
        index
            .visit(
                Aabb::new(
                    Point::new(q.x - radius, q.y - radius),
                    Point::new(q.x + radius, q.y + radius),
                ),
                |id| {
                    if !covered {
                        let [a, b] = segments[id];
                        let dx = b.x - a.x;
                        let dy = b.y - a.y;
                        let length2 = dx * dx + dy * dy;
                        let t = if length2 == 0. {
                            0.
                        } else {
                            (((q.x - a.x) * dx + (q.y - a.y) * dy) / length2).clamp(0., 1.)
                        };
                        covered = envelope_dominated(&c.points, a.lerp(b, t), slope, budget);
                    }
                    Ok(())
                },
            )
            .expect("infallible envelope visitor");
        if covered || !isolated.insert(key(q)) {
            return false;
        }
        // If no retained contour supplies a bounded witness, keep a point
        // execution. Never chain approximate domination through spokes:
        // that would accumulate error or delete all witnesses in a cycle.
        c.points = vec![q];
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        geometry::Point,
        job::Job,
        motion::{Motion, MotionKind},
        stock::vbit_removed_depth_at,
    };

    #[test]
    fn endpoint_bound_covers_the_whole_sweep_and_finite_tip_at_all_test_points() {
        let job = Job::from_json(include_str!("../../../../fixtures/m4/finite-tip.json")).unwrap();
        let ctx = Context::new(&job).unwrap();
        let slope = ctx.tool.angle().slope();
        let q = Position::new(Point::new(3., 0.), -2.);
        let p = Position::new(Point::new(3. - 1.5 * slope - 0.0002, 0.), -0.5);
        let budget = 0.001;
        assert_eq!(dominated_endpoint(&[p, q], slope, budget), Some(q));
        let cut = |start, end| Motion {
            id: 0,
            tool_id: "vbit".into(),
            operation_id: "test".into(),
            layer: 0,
            kind: MotionKind::Cut,
            start,
            end,
            feed_mm_min: Some(100.),
        };
        for i in -20..=50 {
            for j in -20..=20 {
                let point = Point::new(i as f64 / 10., j as f64 / 10.);
                let before = vbit_removed_depth_at(&[cut(p, q)], &ctx.tool, point).unwrap();
                let after = vbit_removed_depth_at(&[cut(q, q)], &ctx.tool, point).unwrap();
                assert!(before - after <= budget);
            }
        }
        let sharp = Position::new(Point::new(0., 0.), -1.);
        assert_eq!(dominated_endpoint(&[sharp, q], slope, budget), None);
        assert_eq!(dominated_endpoint(&[p, q], slope, 0.00001), None);
    }

    #[test]
    fn curved_band_retains_contour_and_all_discarded_sweep_coverage() {
        use crate::{
            stock::StockQuery,
            vcarve::{medial, routing},
        };
        let mut job =
            Job::from_json(include_str!("../../../../fixtures/m4/narrow-channel.json")).unwrap();
        job.import.geometry_tolerance_mm = 0.001;
        job.source.svg = job.source.svg.replace(
            "M0 0h3v20h-3z",
            "M18 10 A8 8 0 1 1 2 10 A8 8 0 1 1 18 10 Z M15 10 A5 5 0 1 0 5 10 A5 5 0 1 0 15 10 Z",
        );
        let ctx = Context::new(&job).unwrap();
        let (axis, mut paths) = medial::build(&ctx).unwrap();
        routing::weld_endpoints(&ctx, &mut paths).unwrap();
        let original = paths.clone();
        redundant_spokes(&ctx, &axis, &mut paths);
        let length = |paths: &[Candidate]| {
            paths
                .iter()
                .flat_map(|c| c.points.windows(2))
                .map(|w| w[0].xy().distance(w[1].xy()))
                .sum::<f64>()
        };
        assert!(length(&paths) < length(&original) * 0.7);
        let moves: Vec<_> = paths
            .iter()
            .flat_map(|c| {
                (0..c.points.len()).map(|i| Motion {
                    id: i,
                    tool_id: "vbit".into(),
                    operation_id: "test".into(),
                    layer: 0,
                    kind: MotionKind::Cut,
                    start: c.points[i],
                    end: c.points[(i + 1).min(c.points.len() - 1)],
                    feed_mm_min: Some(100.),
                })
            })
            .collect();
        let stock = StockQuery::vbit(&moves, &ctx.tool).unwrap();
        // Original spokes remain independent witnesses, including positions
        // absent from the new motion-dependent quality sample set.
        for c in &original {
            for w in c.points.windows(2) {
                for i in 0..=10 {
                    let p = w[0].lerp(w[1], i as f64 / 10.);
                    assert!(
                        p.depth() - stock.removed_depth(p.xy()).unwrap()
                            <= ctx.motion_tolerance / 8. + 1e-10
                    );
                }
            }
        }
        let mut twice = paths.clone();
        redundant_spokes(&ctx, &axis, &mut twice);
        assert_eq!(
            serde_json::to_value(&paths).unwrap(),
            serde_json::to_value(twice).unwrap()
        );
    }

    #[test]
    fn contour_simplification_bounds_flank_coverage_and_keeps_closed_loops() {
        use crate::stock::StockQuery;
        let job = Job::from_json(include_str!("../../../../fixtures/m4/wide-floor.json")).unwrap();
        let ctx = Context::new(&job).unwrap();
        let original = Candidate {
            family: PathFamily::Boundary,
            source_branch: None,
            points: (0..=512)
                .map(|i| {
                    let angle = (i % 512) as f64 * std::f64::consts::TAU / 512.;
                    Position::new(
                        Point::new(15. + 4. * angle.cos(), 20. + 4. * angle.sin()),
                        -1.,
                    )
                })
                .collect(),
        };
        let mut paths = vec![original.clone()];
        simplify_contours(&ctx, &mut paths).unwrap();
        assert!(paths[0].points.len() * 3 < original.points.len() * 2);
        assert_eq!(paths[0].points.first(), paths[0].points.last());
        let moves = |c: &Candidate| {
            c.points
                .windows(2)
                .enumerate()
                .map(|(id, w)| Motion {
                    id,
                    tool_id: "vbit".into(),
                    operation_id: "test".into(),
                    layer: 0,
                    kind: MotionKind::Cut,
                    start: w[0],
                    end: w[1],
                    feed_mm_min: Some(100.),
                })
                .collect::<Vec<_>>()
        };
        let before = moves(&original);
        let after = moves(&paths[0]);
        let before = StockQuery::vbit(&before, &ctx.tool).unwrap();
        let after = StockQuery::vbit(&after, &ctx.tool).unwrap();
        for i in 0..2048 {
            let angle = i as f64 * std::f64::consts::TAU / 2048.;
            for radius in [3.1, 3.5, 4., 4.5, 4.9] {
                let p = Point::new(15. + radius * angle.cos(), 20. + radius * angle.sin());
                assert!(
                    (before.removed_depth(p).unwrap() - after.removed_depth(p).unwrap()).abs()
                        <= ctx.motion_tolerance.min(ctx.tolerance) / 8. + 1e-10
                );
            }
        }
    }

    #[test]
    fn isolated_witnesses_survive_and_approximation_does_not_chain() {
        use crate::vcarve::medial;
        let job =
            Job::from_json(include_str!("../../../../fixtures/m4/narrow-channel.json")).unwrap();
        let ctx = Context::new(&job).unwrap();
        let (axis, _) = medial::build(&ctx).unwrap();
        let id = axis
            .branches
            .iter()
            .find(|b| !b.curve.is_curved())
            .unwrap()
            .id;
        let q = Position::new(Point::new(1.5, 15.), -1.);
        let spoke = |dy| Candidate {
            family: PathFamily::Medial,
            source_branch: Some(id),
            points: vec![
                Position::new(Point::new(1., 15. + dy), -0.5),
                Position::new(Point::new(q.x, q.y + dy), q.z),
            ],
        };
        let mut paths = vec![spoke(0.), spoke(0.), spoke(0.0001)];
        redundant_spokes(&ctx, &axis, &mut paths);
        assert_eq!(
            paths.len(),
            2,
            "distinct unsupported witnesses must remain even when close"
        );
        assert!(paths.iter().all(|c| c.points.len() == 1));
        assert_eq!(paths[0].points, vec![q]);
    }
}
