//! Bounded cleanup of microscopic offset edges, before routing or entry planning.
use super::{EntryStrategy, settings::Context, verify};
use crate::geometry::{Point, Segment};
use std::collections::VecDeque;

pub(super) fn tiny_edges(ctx: &Context, points: &mut Vec<Point>, depth: f64) {
    let n = points.len();
    if n <= 3 || ctx.cleanup_budget <= 0. {
        return;
    }
    let mut next: Vec<_> = (0..n).map(|i| (i + 1) % n).collect();
    let mut prev: Vec<_> = (0..n).map(|i| (i + n - 1) % n).collect();
    let mut retained = vec![true; n];
    let mut pending: VecDeque<_> = (0..n).collect();
    let mut count = n;
    let mut checks = n.saturating_mul(32);
    while let Some(i) = pending.pop_front() {
        // The longest edge is already at [0, 1]. Preserve that entire ramp
        // traverse, including its endpoints, and never erase a closed loop.
        if !retained[i]
            || count <= 3
            || (matches!(ctx.settings.entry, EntryStrategy::Ramp { .. }) && i < 2)
        {
            continue;
        }
        let a = prev[i];
        let b = next[i];
        if points[a]
            .distance(points[i])
            .min(points[i].distance(points[b]))
            > ctx.cleanup_budget
            || points[a] == points[b]
        {
            continue;
        }
        let cost = (b + n - a) % n;
        if cost > checks {
            break;
        }
        checks -= cost;
        let chord = Segment {
            start: points[a],
            end: points[b],
        };
        let reserve = 128.
            * f64::EPSILON
            * [points[a], points[b]]
                .iter()
                .map(|p| p.x.abs().max(p.y.abs()))
                .fold(1., f64::max);
        // Always measure against ORIGINAL vertices, including ones removed in
        // earlier merges. Linear interpolation bounds the original edges too.
        if (1..cost).all(|j| chord.distance(points[(a + j) % n]) + reserve <= ctx.cleanup_budget)
            && verify::center_margin(ctx, points[a], points[b], depth)
                .is_ok_and(|margin| margin >= ctx.guard / 2.)
        {
            retained[i] = false;
            next[a] = b;
            prev[b] = a;
            count -= 1;
            pending.extend([a, b]);
        }
    }
    let mut i = 0;
    points.retain(|_| {
        let keep = retained[i];
        i += 1;
        keep
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::Job;
    fn context() -> Context {
        Context::new(
            &Job::from_json(include_str!("../../../../fixtures/m3/rectangle.json")).unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn tiny_closing_edges_are_bounded_and_ramp_edge_is_preserved() {
        let mut ctx = context();
        let q = 1. / ctx.target.region().grid().scale();
        let p = Point::new;
        let original = vec![
            p(5., 15.),
            p(20., 15.),
            p(20., 20.),
            p(5., 20.),
            p(5., 15. + q),
        ];
        let mut points = original.clone();
        tiny_edges(&ctx, &mut points, 1.);
        assert_eq!(points.len(), 4);
        for &v in &original {
            assert!((0..points.len()).any(|i| {
                Segment {
                    start: points[i],
                    end: points[(i + 1) % points.len()],
                }
                .distance(v)
                    <= ctx.cleanup_budget
            }));
        }
        ctx.settings.entry = EntryStrategy::Ramp {
            max_angle_deg: 5.,
            feed_mm_min: 100.,
        };
        let mut points = original;
        let entry = points[..2].to_vec();
        tiny_edges(&ctx, &mut points, 1.);
        assert_eq!(points[..2], entry);
        assert_eq!(points.len(), 4);
    }
    #[test]
    fn chained_merges_cannot_accumulate_error_or_erase_a_loop() {
        let ctx = context();
        let b = ctx.cleanup_budget;
        let mut points: Vec<_> = (0..80)
            .map(|i| {
                let t = i as f64 * 0.12;
                Point::new(10. + b * 10. * t.cos(), 20. + b * 10. * t.sin())
            })
            .collect();
        let original = points.clone();
        tiny_edges(&ctx, &mut points, 1.);
        assert!(points.len() >= 3);
        for &p in &original {
            assert!((0..points.len()).any(|i| {
                Segment {
                    start: points[i],
                    end: points[(i + 1) % points.len()],
                }
                .distance(p)
                    <= b
            }));
        }
        let mut triangle = vec![
            Point::new(10., 20.),
            Point::new(10. + b / 2., 20.),
            Point::new(10., 20. + b / 2.),
        ];
        tiny_edges(&ctx, &mut triangle, 1.);
        assert_eq!(triangle.len(), 3);
    }
}
