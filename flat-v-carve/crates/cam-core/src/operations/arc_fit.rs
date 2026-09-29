//! Bounded arc/line fitting of a milling stage's motion stream (plan section
//! 8.4, option 4 of the arc investigation).
//!
//! The planners emit the resolved polyline: a flattened curve becomes
//! thousands of straight segments of a few tens of microns. This pass rewrites
//! a run of those segments into the fewest primitives — straight moves and
//! circular arcs — that stay inside a declared tolerance of the polyline it
//! replaces, so the program follows the same curve with far fewer blocks and
//! no chord error at all on the arcs it emits.
//!
//! Three rules keep it honest:
//!
//! - **Only within one run.** A primitive never spans a semantic breakpoint:
//!   the same stage, tool, purpose, effect, feed, layer, pass and contour, an
//!   unbroken chain of endpoints, and only the cutting purposes a fit may
//!   touch (a plunge, a lead, a tab transition or a lift is never merged).
//! - **Measured, not trusted.** Every primitive is re-measured against every
//!   polyline vertex it covers, in XY *and* in Z, and emitted only if all of
//!   them stay inside the tolerance.
//! - **Bounded work.** The search for the longest covering primitive stops
//!   after [`MAX_FIT_SPAN`] vertices, so a long straight run costs a bounded
//!   amount and is still collapsed into one block per span.
use crate::{
    checks::{ARC_CENTRE_OFFSET_LIMIT_MM, ARC_RESERVE_MM},
    geometry::Point,
    motion::Position,
    sequence::{ArcFitOutput, StageRole},
    toolpath::{ArcMove, Interpolation, MotionEffect, MotionPurpose, PlannedMotion},
};

/// Most vertices one primitive may cover. A bound, not a target: it caps the
/// quadratic search on a long straight run without changing what the fit can
/// represent, because the next primitive simply continues from there.
pub const MAX_FIT_SPAN: usize = 1024;
/// Numeric reserve on the tolerance comparison.
const RESERVE: f64 = 1e-9;
/// How much better than the straight primitive an arc must measure before the
/// fit prefers it. The two only compete when both already satisfy the
/// tolerance, so this chooses between two legal primitives; the margin keeps
/// floating-point noise on a straight run from minting a multi-kilometre arc
/// that no controller can tell from the line it replaced.
const LINE_TIE_MARGIN_MM: f64 = 1e-6;

/// The purposes a fit may rewrite. Everything else is a semantic breakpoint:
/// an entry, a lift, a lead, a tab transition or a knife motion is left
/// exactly as its planner emitted it.
fn fittable(motion: &PlannedMotion) -> bool {
    motion.effect == MotionEffect::MillingSweep
        && matches!(motion.purpose, MotionPurpose::Rough | MotionPurpose::Finish)
        && motion.interpolation == Interpolation::LinearFeed
        && motion.feed_mm_min.is_some()
}

/// Whether two consecutive motions may share one fitted primitive.
///
/// `pass_id` is deliberately *not* part of this: for the V-carve adapter it is
/// the execution (candidate) index — the identity the inspector groups
/// finish paths by — and a fan of medial branches emits thousands of
/// single-motion executions that are continuous in space with the same tool,
/// purpose, effect, feed and depth. Treating that bookkeeping as a machining
/// boundary is what kept the finish fan at one move per run. The primitive
/// carries the pass of its first motion, so the label sequence stays ordered
/// and the plan's evidence is recomputed from it. `contour_id` stays a hard
/// break: separate contours are separate cuts, whatever the geometry does
/// between them.
fn same_run(first: &PlannedMotion, next: &PlannedMotion) -> bool {
    fittable(next)
        && first.stage_id == next.stage_id
        && first.tool_id == next.tool_id
        && first.operation_id == next.operation_id
        && first.purpose == next.purpose
        && first.effect == next.effect
        && first.feed_mm_min == next.feed_mm_min
        && first.layer == next.layer
        && first.contour_id == next.contour_id
        && first.end == next.start
}

/// One fitted primitive over a run of vertices.
#[derive(Clone, Copy, Debug)]
enum Primitive {
    Line { end: usize },
    Arc { end: usize, arc: ArcMove },
}

impl Primitive {
    fn end(self) -> usize {
        match self {
            Self::Line { end } | Self::Arc { end, .. } => end,
        }
    }
}

/// Circumcentre of three points, or `None` when they are collinear (where the
/// straight primitive is the right answer anyway).
fn circumcentre(a: Point, b: Point, c: Point) -> Option<Point> {
    let d = 2. * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    if d.abs() < 1e-12 {
        return None;
    }
    let (aa, bb, cc) = (
        a.x * a.x + a.y * a.y,
        b.x * b.x + b.y * b.y,
        c.x * c.x + c.y * c.y,
    );
    Some(Point::new(
        (aa * (b.y - c.y) + bb * (c.y - a.y) + cc * (a.y - b.y)) / d,
        (aa * (c.x - b.x) + bb * (a.x - c.x) + cc * (b.x - a.x)) / d,
    ))
}

/// Fraction of the primitive at vertex `index` of a run that spans
/// `from..=to`, used to interpolate Z linearly with the XY parameter.
fn parameter(points: &[Point], from: usize, to: usize, index: usize, arc: Option<ArcMove>) -> f64 {
    if to == from {
        return 0.;
    }
    if let Some(arc) = arc {
        let centre = arc.center;
        let angle = |p: Point| (p.y - centre.y).atan2(p.x - centre.x);
        let (a, b, p) = (angle(points[from]), angle(points[to]), angle(points[index]));
        let sweep = |x: f64, y: f64| {
            let mut s = if arc.clockwise { x - y } else { y - x };
            if s < 0. {
                s += std::f64::consts::TAU;
            }
            s
        };
        let total = sweep(a, b);
        if total <= 1e-12 {
            return 0.;
        }
        return (sweep(a, p) / total).clamp(0., 1.);
    }
    let (a, b) = (points[from], points[to]);
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let length_squared = dx * dx + dy * dy;
    if length_squared <= 1e-24 {
        return 0.;
    }
    (((points[index].x - a.x) * dx + (points[index].y - a.y) * dy) / length_squared).clamp(0., 1.)
}

/// Worst XY and Z deviation of the polyline `from..=to` from one candidate
/// primitive, or `None` when the candidate is degenerate.
///
/// Both the vertices *and* each covered chord's midpoint are measured. A
/// straight primitive only needs its vertices (the polyline lies on it between
/// them), but an arc can bulge away from a chord whose endpoints sit on it, so
/// sampling the chords is what makes the acceptance test about the polyline
/// rather than about its corners.
fn deviation(
    points: &[Point],
    depths: &[f64],
    from: usize,
    to: usize,
    arc: Option<ArcMove>,
) -> Option<f64> {
    let mut worst: f64 = 0.;
    let distance = |p: Point| match arc {
        Some(arc) => arc.distance(points[from], points[to], p),
        None => crate::toolpath::distance_to_segment(p, points[from], points[to]),
    };
    for index in from..=to {
        let xy = distance(points[index]);
        let t = parameter(points, from, to, index, arc);
        let z = depths[from] + (depths[to] - depths[from]) * t;
        worst = worst.max(xy).max((depths[index] - z).abs());
        if index > from {
            let (a, b) = (points[index - 1], points[index]);
            let mid = Point::new((a.x + b.x) / 2., (a.y + b.y) / 2.);
            let t_before = parameter(points, from, to, index - 1, arc);
            let t_mid = (t_before + t) / 2.;
            let z_mid = (depths[index - 1] + depths[index]) / 2.;
            let z_model = depths[from] + (depths[to] - depths[from]) * t_mid;
            worst = worst.max(distance(mid)).max((z_mid - z_model).abs());
        }
        if !worst.is_finite() {
            return None;
        }
    }
    Some(worst)
}

/// The circular candidate through the first, middle and last vertex of a span:
/// a centre, a direction and a radius. Collinear spans have no such circle, and
/// degenerate ones have no swing to program.
fn candidate_arc(points: &[Point], from: usize, to: usize) -> Option<ArcMove> {
    if to < from + 2 || points[from].distance(points[to]) <= 1e-12 {
        return None;
    }
    let middle = from + (to - from) / 2;
    let center = circumcentre(points[from], points[middle], points[to])?;
    // The centre's offset from the span's start is exactly what the program's
    // `I`/`J` words carry, so a centre beyond the word bound is not
    // programmable — and an arc that far away is the straight primitive
    // anyway. Refuse it here and let the line cover the span.
    if (center.x - points[from].x).abs() > ARC_CENTRE_OFFSET_LIMIT_MM
        || (center.y - points[from].y).abs() > ARC_CENTRE_OFFSET_LIMIT_MM
    {
        return None;
    }
    // The sign of the turn between the two chords is the arc's direction.
    let (a, b) = (points[middle], points[to]);
    let cross = (a.x - points[from].x) * (b.y - a.y) - (a.y - points[from].y) * (b.x - a.x);
    let arc = ArcMove {
        center,
        clockwise: cross < 0.,
    };
    let radius = arc.radius(points[from])?;
    if !radius.is_finite() || radius <= ARC_RESERVE_MM {
        return None;
    }
    let sweep = arc.sweep_rad(points[from], points[to])?;
    // At most a half turn: past that the same three points describe the other
    // arc as well, and a primitive that wraps further is not a local fit of
    // the polyline it covers.
    (sweep > 1e-9 && sweep <= std::f64::consts::PI).then_some(arc)
}

/// Longest primitive starting at `from`: extended while either model still
/// covers every vertex inside the tolerance, then the model that covers it
/// best is emitted. An arc has to beat the straight move by more than
/// [`LINE_TIE_MARGIN_MM`]; a tie — or a noise-scale win on a straight run —
/// keeps the line, which every controller handles natively.
fn fit_from(points: &[Point], depths: &[f64], from: usize, tolerance: f64) -> Option<Primitive> {
    let last = points.len() - 1;
    let limit = last.min(from + MAX_FIT_SPAN);
    let mut best: Option<Primitive> = None;
    for to in from + 1..=limit {
        let line =
            deviation(points, depths, from, to, None).filter(|error| *error <= tolerance + RESERVE);
        let arc = candidate_arc(points, from, to).and_then(|arc| {
            deviation(points, depths, from, to, Some(arc))
                .filter(|error| *error <= tolerance + RESERVE)
                .map(|error| (arc, error))
        });
        let chosen = match (line, arc) {
            (Some(line), Some((arc, arc_error))) => {
                Some(if line - arc_error > LINE_TIE_MARGIN_MM {
                    Primitive::Arc { end: to, arc }
                } else {
                    Primitive::Line { end: to }
                })
            }
            (Some(_), None) => Some(Primitive::Line { end: to }),
            (None, Some((arc, _))) => Some(Primitive::Arc { end: to, arc }),
            // Neither model reaches this far: the last vertex that fit is the
            // primitive's end.
            (None, None) => break,
        };
        best = chosen;
    }
    best
}

/// Fit one stage's motions. Returns the rewritten stream and what the fit
/// measured; the caller owns stage ranges and motion ids.
pub(crate) fn fit_motions(
    motions: &[PlannedMotion],
    tolerance_mm: f64,
) -> (Vec<PlannedMotion>, ArcFitOutput) {
    let mut out: Vec<PlannedMotion> = Vec::with_capacity(motions.len());
    let mut before = 0usize;
    let mut arcs = 0usize;
    let mut worst: f64 = 0.;
    let mut index = 0usize;
    while index < motions.len() {
        before += 1;
        if !fittable(&motions[index]) {
            out.push(motions[index].clone());
            index += 1;
            continue;
        }
        // The run this motion can share a primitive with.
        let mut last = index;
        while last + 1 < motions.len() && same_run(&motions[last], &motions[last + 1]) {
            last += 1;
            before += 1;
        }
        let mut points: Vec<Point> = Vec::with_capacity(last - index + 2);
        let mut depths: Vec<f64> = Vec::with_capacity(last - index + 2);
        points.push(motions[index].start.xy());
        depths.push(motions[index].start.z);
        for motion in &motions[index..=last] {
            points.push(motion.end.xy());
            depths.push(motion.end.z);
        }
        let template = &motions[index];
        let mut cursor = 0usize;
        while cursor + 1 < points.len() {
            let primitive = fit_from(&points, &depths, cursor, tolerance_mm).unwrap_or(
                // Two vertices always fit a straight primitive; this arm can
                // only be reached if the tolerance is not finite.
                Primitive::Line { end: cursor + 1 },
            );
            let end = primitive.end();
            let interpolation = match primitive {
                Primitive::Line { .. } => Interpolation::LinearFeed,
                Primitive::Arc { arc, .. } => {
                    arcs += 1;
                    Interpolation::ArcFeed(arc)
                }
            };
            if let Some(error) = deviation(
                &points,
                &depths,
                cursor,
                end,
                match primitive {
                    Primitive::Arc { arc, .. } => Some(arc),
                    Primitive::Line { .. } => None,
                },
            ) {
                worst = worst.max(error);
            }
            out.push(PlannedMotion {
                id: template.id,
                operation_id: template.operation_id.clone(),
                stage_id: template.stage_id.clone(),
                tool_id: template.tool_id.clone(),
                contour_id: template.contour_id.clone(),
                pass_id: template.pass_id,
                layer: template.layer,
                interpolation,
                purpose: template.purpose,
                effect: template.effect,
                start: Position::new(points[cursor], depths[cursor]),
                end: Position::new(points[end], depths[end]),
                feed_mm_min: template.feed_mm_min,
                blade_heading_deg: None,
            });
            cursor = end;
        }
        index = last + 1;
    }
    let output = ArcFitOutput {
        tolerance_mm,
        motions_before: before,
        motions_after: out.len(),
        arcs_emitted: arcs,
        max_deviation_mm: worst,
    };
    (out, output)
}

/// Whether a stage role may be fitted. Knife stages keep their planner's arcs:
/// their tip contract is the planner's, not this pass's.
pub(crate) fn fittable_role(role: StageRole) -> bool {
    !matches!(
        role,
        StageRole::Knife | StageRole::PocketRough | StageRole::PocketFinish
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One fittable linear cutting motion of a single continuous run.
    fn segment(from: Point, to: Point) -> PlannedMotion {
        PlannedMotion {
            id: 0,
            operation_id: "op".into(),
            stage_id: "stage".into(),
            tool_id: "tool".into(),
            contour_id: Some("contour".into()),
            pass_id: 0,
            layer: 0,
            interpolation: Interpolation::LinearFeed,
            purpose: MotionPurpose::Finish,
            effect: MotionEffect::MillingSweep,
            start: Position::new(from, -1.),
            end: Position::new(to, -1.),
            feed_mm_min: Some(400.),
            blade_heading_deg: None,
        }
    }

    /// A near-straight run of `count` segments along +X, every vertex lifted
    /// `wiggle` mm above the chord's line, chaining end to start.
    fn near_straight_run(count: usize, wiggle: f64) -> Vec<PlannedMotion> {
        let span = 1.1;
        let step = span / count as f64;
        let vertex = |i: usize| {
            let x = i as f64 * step;
            Point::new(x, wiggle * (std::f64::consts::PI * x / span).sin())
        };
        (0..count)
            .map(|i| segment(vertex(i), vertex(i + 1)))
            .collect()
    }

    #[test]
    fn a_straight_run_with_noise_scale_curvature_fits_to_lines_not_arcs() {
        // A wiggle below the tie margin is indistinguishable from a straight
        // line at the program's own precision, so the fit must keep emitting
        // the straight primitive rather than a sub-micron "win" that mints an
        // arc with a centre kilometres away (the failing-export shape).
        let motions = near_straight_run(64, 9e-7);
        let (fitted, evidence) = fit_motions(&motions, 0.005);
        assert_eq!(
            evidence.arcs_emitted, 0,
            "noise-scale curvature must not mint arcs"
        );
        assert!(
            fitted.len() < motions.len(),
            "the run still collapses: {} primitives for {} segments",
            fitted.len(),
            motions.len()
        );
        assert!(
            fitted
                .iter()
                .all(|motion| motion.interpolation == Interpolation::LinearFeed)
        );
        assert!(evidence.max_deviation_mm <= 0.005);
    }

    #[test]
    fn a_curved_run_still_fits_arcs() {
        // Real curvature wins by far more than the margin, so the tie-break
        // must not suppress the arcs the fit exists to emit.
        let motions = near_straight_run(64, 0.002);
        let (fitted, evidence) = fit_motions(&motions, 0.005);
        assert!(evidence.arcs_emitted > 0, "{evidence:?}");
        for motion in &fitted {
            if let Interpolation::ArcFeed(arc) = motion.interpolation {
                let from = motion.start.xy();
                assert!(
                    (arc.center.x - from.x).abs() <= ARC_CENTRE_OFFSET_LIMIT_MM
                        && (arc.center.y - from.y).abs() <= ARC_CENTRE_OFFSET_LIMIT_MM,
                    "an emitted arc carries a centre the program cannot word"
                );
            }
        }
        assert!(evidence.max_deviation_mm <= 0.005);
    }

    #[test]
    fn a_span_whose_circumcentre_is_beyond_the_word_bound_has_no_arc() {
        // Endpoints 2 mm apart with a 0.25 µm bulge: the circle through all
        // three vertices has its centre about 2 km away, past the limit the
        // program's I/J words can carry, so no arc candidate exists.
        let points = [
            Point::new(0., 0.),
            Point::new(1., 2.5e-7),
            Point::new(2., 0.),
        ];
        assert!(candidate_arc(&points, 0, 2).is_none());
        // The same span with real curvature keeps its arc candidate.
        let curved = [Point::new(0., 0.), Point::new(1., 0.25), Point::new(2., 0.)];
        assert!(candidate_arc(&curved, 0, 2).is_some());
    }
}
