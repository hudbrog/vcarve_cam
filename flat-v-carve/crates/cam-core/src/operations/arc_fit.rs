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
    checks::ARC_RESERVE_MM,
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
fn same_run(first: &PlannedMotion, next: &PlannedMotion) -> bool {
    fittable(next)
        && first.stage_id == next.stage_id
        && first.tool_id == next.tool_id
        && first.operation_id == next.operation_id
        && first.purpose == next.purpose
        && first.effect == next.effect
        && first.feed_mm_min == next.feed_mm_min
        && first.layer == next.layer
        && first.pass_id == next.pass_id
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
/// best is emitted. A tie prefers the straight move, which every controller
/// handles natively.
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
            (Some(line), Some((arc, arc_error))) => Some(if arc_error < line {
                Primitive::Arc { end: to, arc }
            } else {
                Primitive::Line { end: to }
            }),
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
    role != StageRole::Knife
}
