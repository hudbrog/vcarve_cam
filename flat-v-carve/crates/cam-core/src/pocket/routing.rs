//! Checked links with optional retracing along the last fully cut contour.
use super::{settings::Context, verify};
use crate::geometry::{BoundaryQuery, Point, Segment};

fn project(ctx: &Context, p: Point, a: Point, b: Point) -> (Point, f64) {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let length = a.distance(b);
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / (dx * dx + dy * dy)).clamp(0., 1.);
    // Preserve exact vertices when a partial-edge move would be microscopic.
    if t * length <= ctx.cleanup_budget {
        (a, 0.)
    } else if (1. - t) * length <= ctx.cleanup_budget {
        (b, 1.)
    } else {
        (Point::new(a.x + dx * t, a.y + dy * t), t)
    }
}

pub(super) fn link(
    ctx: &Context,
    contour: &[Point],
    next: &mut Vec<Point>,
    depth: f64,
    prior: Option<&BoundaryQuery>,
) -> Option<Vec<Point>> {
    let &start = contour.first()?;
    // Prefer feasible starts on this contour; bound alternate-start searches.
    let mut vertices: Vec<_> = (0..next.len()).collect();
    vertices.sort_by(|&a, &b| {
        start
            .distance(next[a])
            .total_cmp(&start.distance(next[b]))
            .then(a.cmp(&b))
    });
    for v in vertices.into_iter().take(16) {
        if let Some(path) = departure(ctx, contour, next[v], depth, prior) {
            next.rotate_left(v);
            return Some(path);
        }
    }
    // When clearing from an inner contour outward, its nearest exit can still
    // be farther than a stepover from every outer vertex. Split an existing
    // outer edge at a nearer entry, preserving its complete original geometry.
    let mut entries: Vec<_> = (0..next.len())
        .map(|i| {
            let (q, _) = project(ctx, start, next[i], next[(i + 1) % next.len()]);
            (start.distance(q), i, q)
        })
        .collect();
    entries.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    for (_, i, q) in entries.into_iter().take(16) {
        if let Some(path) = departure(ctx, contour, q, depth, prior) {
            let j = (i + 1) % next.len();
            if q == next[i] {
                next.rotate_left(i);
            } else if q == next[j] {
                next.rotate_left(j);
            } else {
                next.rotate_left(j);
                next.insert(0, q);
            }
            return Some(path);
        }
    }
    None
}

fn fresh_link(
    ctx: &Context,
    a: Point,
    b: Point,
    depth: f64,
    prior: Option<&BoundaryQuery>,
) -> bool {
    if !verify::center_margin(ctx, a, b, depth).is_ok_and(|m| m >= ctx.guard / 2.) {
        return false;
    }
    if depth <= ctx.stepdown {
        return true;
    }
    // The whole cutter disk must fit in a lower-bound sweep at depth-stepdown.
    // A target-center check alone says nothing about the stock above this cut.
    let r = ctx.mill.radius().mm() + ctx.guard / 2.;
    prior.is_some_and(|q| {
        q.variable_radius_margin_mm(Segment { start: a, end: b }, r, r)
            .is_ok_and(|margin| margin >= 0.)
    })
}

/// Return the already-cut part of the route, excluding the current position.
/// The caller appends the checked fresh link to `destination` at cutting feed.
pub(super) fn departure(
    ctx: &Context,
    contour: &[Point],
    destination: Point,
    depth: f64,
    prior: Option<&BoundaryQuery>,
) -> Option<Vec<Point>> {
    let &start = contour.first()?;
    // Two independently constructed/simplified contours each contribute their
    // offset and cleanup budgets to this heuristic, not to cutter clearance.
    let grid = ctx.target.region().grid();
    let limit =
        ctx.stepover + 2. * (grid.arc_tolerance_mm() + grid.snap_bound_mm() + ctx.cleanup_budget);
    if start.distance(destination) <= limit && fresh_link(ctx, start, destination, depth, prior) {
        return Some(vec![]);
    }
    let lengths: Vec<_> = contour
        .iter()
        .zip(contour.iter().cycle().skip(1))
        .map(|(&a, &b)| a.distance(b))
        .collect();
    let perimeter: f64 = lengths.iter().sum();
    let mut at = 0.;
    let mut best: Option<(f64, usize, Point, bool)> = None;
    for (i, &a) in contour.iter().enumerate() {
        let b = contour[(i + 1) % contour.len()];
        let (q, t) = project(ctx, destination, a, b);
        let forward = at + lengths[i] * t;
        let reverse = perimeter - forward;
        let cost = forward.min(reverse) + q.distance(destination);
        // Bound detours. All departure travel retraces an actual completed
        // contour; only the final crossing may remove fresh material.
        if q.distance(destination) <= limit
            && cost <= 4. * limit
            && best.as_ref().is_none_or(|v| cost < v.0)
            && fresh_link(ctx, q, destination, depth, prior)
        {
            best = Some((cost, i, q, forward <= reverse));
        }
        at += lengths[i];
    }
    best.map(|(_, i, q, forward)| {
        let mut path = if forward {
            contour[1..=i].to_vec()
        } else {
            contour[i + 1..].iter().rev().copied().collect()
        };
        if path.last().copied().unwrap_or(start) != q {
            path.push(q);
        }
        path
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        geometry::{Grid, Region},
        job::Job,
    };
    #[test]
    fn corner_departure_retraces_a_cut_edge_before_one_stepover_crossing() {
        let ctx = Context::new(
            &Job::from_json(include_str!("../../../../fixtures/m3/rectangle.json")).unwrap(),
        )
        .unwrap();
        let p = Point::new;
        let contour = vec![p(5., 15.), p(20., 15.), p(20., 25.), p(5., 25.)];
        let destination = p(6.5, 16.5);
        let path = departure(&ctx, &contour, destination, 1., None).unwrap();
        assert_eq!(path.len(), 1);
        assert!((path[0].distance(destination) - ctx.stepover).abs() < 1e-12);
        assert!(contour[0].distance(destination) > ctx.stepover);
        assert_eq!(
            Segment {
                start: contour[0],
                end: contour[1]
            }
            .distance(path[0]),
            0.
        );
        assert!(departure(&ctx, &contour, destination, 2., None).is_none());
    }
    #[test]
    fn deeper_links_require_the_whole_swept_cutter_in_preceding_stock() {
        let ctx = Context::new(
            &Job::from_json(include_str!("../../../../fixtures/m3/rectangle.json")).unwrap(),
        )
        .unwrap();
        let p = Point::new;
        let region = Region::from_rings(
            Grid::new(0.005, 100.).unwrap(),
            &[vec![p(5., 15.), p(20., 15.), p(20., 25.), p(5., 25.)]],
        )
        .unwrap();
        let prior = BoundaryQuery::new(&region);
        assert!(fresh_link(&ctx, p(10., 20.), p(11., 20.), 2., Some(&prior)));
        assert!(
            !fresh_link(&ctx, p(6., 20.), p(7., 20.), 2., Some(&prior)),
            "centers fit but cutter crosses uncleared stock"
        );
    }

    #[test]
    fn outward_link_splits_an_edge_without_changing_the_contour() {
        let ctx = Context::new(
            &Job::from_json(include_str!("../../../../fixtures/m3/rectangle.json")).unwrap(),
        )
        .unwrap();
        let p = Point::new;
        let inner = vec![p(6.5, 16.5), p(18.5, 16.5), p(18.5, 23.5), p(6.5, 23.5)];
        let original = vec![p(5., 15.), p(20., 15.), p(20., 25.), p(5., 25.)];
        let mut next = original.clone();
        assert!(link(&ctx, &inner, &mut next, 1., None).unwrap().is_empty());
        assert_eq!(next.len(), original.len() + 1);
        assert!(inner[0].distance(next[0]) <= ctx.stepover);
        for &v in &original {
            assert!(next.contains(&v));
        }
        for (i, &a) in next.iter().enumerate() {
            let b = next[(i + 1) % next.len()];
            assert!((0..original.len()).any(|j| {
                let edge = Segment {
                    start: original[j],
                    end: original[(j + 1) % original.len()],
                };
                edge.distance(a) < 1e-12 && edge.distance(b) < 1e-12
            }));
        }
    }
}
