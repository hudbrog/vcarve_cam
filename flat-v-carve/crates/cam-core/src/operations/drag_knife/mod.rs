//! Passive drag-knife planner (plan section 12, slice F2): known-heading
//! alignment, tangent holder compensation on straight/smooth sections,
//! explicit corner swivels with contact depth, depth passes, closure and
//! overlap handling, and the independent no-slip replay as the tip-path
//! error gate.
//!
//! Programmed XY is the blade holder's pivot (plan section 12.1). For an
//! ideal no-slip blade on a tip path `p(s)` with forward tangent `t(s)` and
//! blade offset `d`, the holder rides `q(s) = p(s) + d*t(s)` on straights and
//! pivots on a radius-`d` arc centered on the tip corner during turns. The
//! modeled blade heading (pivot toward tip, degrees CCW from +X) therefore
//! trails travel by 180 degrees and is carried — not reset — across lifts
//! and disconnected chains (plan section 12.3).
pub mod evidence;
pub mod replay;

use crate::{
    contours::ResolvedAnchor,
    geometry::{Diagnostic, Point, Result},
    model::VBit,
    motion::Position,
    operations::{LocatedDiagnostic, PlanContext, PlannerGeometry},
    project::{DragKnifeSettings, StartSelection, ToolGeometry},
    sequence::{
        CoolantIntent, GenerationStatus, LocalStage, PathControlIntent, PlanIssue,
        PlannedOperation, ProcessIntent, ProcessSpindle, StageRole,
    },
    setup::resolve_heights_values,
    toolpath::{Interpolation, MotionEffect, MotionPurpose, PlannedMotion},
};
use replay::{IntendedTip, ReplayStatus, replay};
use std::collections::BTreeMap;

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("drag_knife")
}

/// Numerical reserve for in-stock contact checks.
const RESERVE_MM: f64 = 1e-6;
/// Turns within this many degrees of a pure reversal have no trustworthy
/// swivel direction; the first release rejects them instead of guessing
/// (plan section 12.1).
const AMBIGUOUS_REVERSAL_DEG: f64 = 170.;
/// An alignment/contact swivel whose sweep is this close to 180 degrees is
/// equally ambiguous and is rejected rather than arbitrarily sided.
const AMBIGUOUS_ALIGNMENT_DEG: f64 = 0.5;
/// Total adaptive-integration steps the replay may spend before it reports
/// budget exhaustion (inconclusive, never success; plan section 12.5).
const REPLAY_STEP_BUDGET: usize = 2_000_000;

fn incomplete(issues: Vec<PlanIssue>) -> PlannedOperation {
    PlannedOperation {
        status: GenerationStatus::Incomplete,
        stages: vec![],
        motions: vec![],
        stage_evidence: vec![],
        pass_evidence: vec![],
        issues,
        preparation: vec![],
        named_outputs: vec![],
    }
}

fn issue(code: &str, message: impl Into<String>, operation_id: &str) -> PlanIssue {
    PlanIssue {
        code: code.into(),
        message: message.into(),
        operation_id: Some(operation_id.into()),
        stage_id: None,
    }
}

/// Required-but-unset fields for planning this knife operation. The artwork
/// field names the collection entry ("source" in schema 4, "artwork" in
/// schema 5).
pub fn missing_fields(
    job: &crate::project::CamJob,
    operation_id: &str,
    settings: &DragKnifeSettings,
) -> Vec<LocatedDiagnostic> {
    missing_fields_ctx(&PlanContext::from_v4(job), operation_id, settings, "source")
}

pub(crate) fn missing_fields_ctx(
    ctx: &PlanContext,
    operation_id: &str,
    settings: &DragKnifeSettings,
    artwork_field: &str,
) -> Vec<LocatedDiagnostic> {
    let mut missing = vec![];
    let mut push = |path: String, what: &str| {
        missing.push(LocatedDiagnostic::missing(
            operation_id,
            &path,
            format!("set {what} before planning operation '{operation_id}'"),
        ));
    };
    if !ctx.has_artwork {
        push(
            artwork_field.into(),
            "an SVG source (knife operations select centerline chains)",
        );
    }
    if ctx.setup.stock.thickness_mm.is_none() {
        push("setup.stock.thickness_mm".into(), "the stock thickness");
    }
    if ctx.setup.stock.xy.is_none() {
        push(
            "setup.stock.xy".into(),
            "physical stock XY dimensions (alignment contact)",
        );
    }
    if ctx.setup.clearance_above_stock_mm.is_none() {
        push(
            "setup.clearance_above_stock_mm".into(),
            "the clearance plane",
        );
    }
    if ctx.tolerances.motion_tolerance_mm.is_none() {
        push(
            "tolerances.motion_tolerance_mm".into(),
            "the motion tolerance",
        );
    }
    if settings.chains.is_empty() {
        push(
            format!("operations[{operation_id}].chains"),
            "at least one imported chain",
        );
    }
    for (value, name) in [
        (settings.stepdown_mm, "stepdown_mm"),
        (settings.swivel_depth_mm, "swivel_depth_mm"),
        (settings.corner_threshold_deg, "corner_threshold_deg"),
    ] {
        if value.is_none() {
            push(format!("operations[{operation_id}].{name}"), name);
        }
    }
    let tool = ctx.tool(&settings.assignment.tool_id);
    if tool.is_some_and(|t| t.geometry.is_none()) {
        push(
            format!("operations[{operation_id}].assignment.tool"),
            "the knife tool geometry",
        );
    }
    for (value, name) in [
        (
            settings.assignment.cutting_feed_mm_min,
            "cutting_feed_mm_min",
        ),
        (settings.assignment.plunge_feed_mm_min, "plunge_feed_mm_min"),
        (settings.assignment.swivel_feed_mm_min, "swivel_feed_mm_min"),
        (settings.assignment.max_stepdown_mm, "max_stepdown_mm"),
    ] {
        if value.is_none() {
            push(
                format!("operations[{operation_id}].assignment.{name}"),
                name,
            );
        }
    }
    if settings.alignment.initial_heading_deg.is_none() {
        push(
            format!("operations[{operation_id}].alignment.initial_heading_deg"),
            "the initial blade heading (never defaulted)",
        );
    }
    missing
}

fn knife_geometry(
    ctx: &PlanContext,
    settings: &DragKnifeSettings,
) -> Result<crate::project::DragKnifeSpec> {
    let tool = ctx
        .tool(&settings.assignment.tool_id)
        .ok_or_else(|| error("PROJECT_TOOL_REFERENCE", "knife tool not found"))?;
    match &tool.geometry {
        Some(ToolGeometry::DragKnife(spec)) => Ok(spec.clone()),
        Some(other) => Err(error(
            "PROJECT_TOOL_KIND",
            format!(
                "knife operations need a drag_knife tool, found {}",
                match other {
                    ToolGeometry::Endmill(_) => "endmill",
                    ToolGeometry::Vbit(_) => "vbit",
                    ToolGeometry::DragKnife(_) => "drag_knife",
                }
            ),
        )),
        None => Err(error(
            "MISSING_MACHINING_SETTING",
            "knife tool geometry is required",
        )),
    }
}

/// Normalize an angle to (-pi, pi].
fn normalize_angle(radians: f64) -> f64 {
    let mut value = radians.rem_euclid(2. * std::f64::consts::PI);
    if value > std::f64::consts::PI {
        value -= 2. * std::f64::consts::PI;
    }
    value
}

fn heading_deg(radians: f64) -> f64 {
    radians.to_degrees().rem_euclid(360.)
}

fn angle_of(v: Point) -> f64 {
    v.y.atan2(v.x)
}

/// Signed turn between two unit tangents, normalized to (-180, 180] degrees.
fn signed_turn_deg(t_in: Point, t_out: Point) -> f64 {
    let cross = t_in.x * t_out.y - t_in.y * t_out.x;
    let dot = t_in.x * t_out.x + t_in.y * t_out.y;
    let deg = cross.atan2(dot).to_degrees();
    if deg <= -180. + 1e-9 { 180. } else { deg }
}

/// One element of the ideal holder path.
#[derive(Clone, Copy, Debug)]
enum PathElement {
    /// The tip travels `from` -> `to` along unit tangent `t`; the holder
    /// rides the parallel line `d` ahead of the tip.
    Line { from: Point, to: Point, t: Point },
    /// The tip stays planted at `center` while the holder pivots on the
    /// radius-`d` arc around it, starting at angle `psi_start` (the incoming
    /// travel direction) by the signed `sweep_rad` (the corner's turn).
    Arc {
        center: Point,
        psi_start: f64,
        sweep_rad: f64,
    },
}

/// The resolved tip path of one chain in travel order.
struct KnifePath {
    chain_id: String,
    elements: Vec<PathElement>,
    /// Holder pivot where the first cut begins (`first tip vertex + d*t`).
    start_pivot: Point,
    /// Display-convention heading of the first cut (trailing the tangent).
    first_heading: f64,
}

/// Build the tip polyline in travel order: open chains as drawn, closed
/// rings from the start vertex around once and back, extended by the
/// closure overlap in the travel direction (plan section 12.4 step 6).
fn tip_polyline(
    vertices: &[Point],
    closed: bool,
    overlap_mm: f64,
    chain_id: &str,
) -> Result<Vec<Point>> {
    let mut path = vertices.to_vec();
    if !closed {
        return Ok(path);
    }
    path.push(vertices[0]);
    if overlap_mm <= RESERVE_MM {
        return Ok(path);
    }
    let perimeter: f64 = vertices
        .iter()
        .zip(vertices.iter().cycle().skip(1))
        .map(|(a, b)| a.distance(*b))
        .sum();
    if overlap_mm >= perimeter - RESERVE_MM {
        return Err(error(
            "KNIFE_CLOSURE_RANGE",
            format!(
                "chain '{chain_id}': closure overlap {overlap_mm} mm must stay below one loop ({perimeter:.4} mm); a second uncontrolled loop is not generated"
            ),
        ));
    }
    let mut remaining = overlap_mm;
    let count = vertices.len();
    let mut index = 0usize;
    while remaining > RESERVE_MM {
        let a = vertices[index % count];
        let b = vertices[(index + 1) % count];
        let length = a.distance(b);
        if length <= RESERVE_MM {
            index += 1;
            continue;
        }
        if remaining >= length - RESERVE_MM {
            path.push(b);
            remaining -= length;
        } else {
            path.push(Point::new(
                a.x + (b.x - a.x) * remaining / length,
                a.y + (b.y - a.y) * remaining / length,
            ));
            remaining = 0.;
        }
        index += 1;
    }
    Ok(path)
}

/// Analyze one chain into compensated path elements. Every turn becomes a
/// swivel arc in the geometry; whether it executes as an explicit
/// depth-lifted swivel or a continuous cutting arc is decided at emission
/// from the corner threshold. Near-reversal turns are rejected here.
fn build_path(
    chain_id: &str,
    vertices: &[Point],
    closed: bool,
    overlap_mm: f64,
) -> Result<KnifePath> {
    let tip = tip_polyline(vertices, closed, overlap_mm, chain_id)?;
    let mut elements = vec![];
    let mut previous_tangent: Option<Point> = None;
    for pair in tip.windows(2) {
        let from = pair[0];
        let to = pair[1];
        let length = from.distance(to);
        if length <= 1e-12 {
            continue;
        }
        let t = Point::new((to.x - from.x) / length, (to.y - from.y) / length);
        if let Some(t_in) = previous_tangent {
            let turn = signed_turn_deg(t_in, t);
            if turn.abs() > AMBIGUOUS_REVERSAL_DEG {
                return Err(error(
                    "KNIFE_CORNER_AMBIGUOUS",
                    format!(
                        "chain '{chain_id}': the {:.1}-degree reversal at ({:.3}, {:.3}) has no trustworthy swivel direction; split or simplify the corner",
                        turn.abs(),
                        from.x,
                        from.y
                    ),
                ));
            }
            elements.push(PathElement::Arc {
                center: from,
                psi_start: angle_of(t_in),
                sweep_rad: turn.to_radians(),
            });
        }
        elements.push(PathElement::Line { from, to, t });
        previous_tangent = Some(t);
    }
    // Closure (plan section 12.4 step 6): with an overlap the seam corner is
    // an interior vertex of the extended polyline and was handled above;
    // without one the closing corner is appended explicitly so every pass
    // ends realigned at the seam — handled exactly once, never doubled.
    if closed
        && overlap_mm <= RESERVE_MM
        && let Some(t_last) = previous_tangent
        && let Some(first_line @ PathElement::Line { t: t_first, .. }) = elements.first()
    {
        let turn = signed_turn_deg(t_last, *t_first);
        if turn.abs() > AMBIGUOUS_REVERSAL_DEG {
            return Err(error(
                "KNIFE_CORNER_AMBIGUOUS",
                format!(
                    "chain '{chain_id}': the closing corner reverses {:.1} degrees; its swivel direction is not trustworthy",
                    turn.abs()
                ),
            ));
        }
        let _ = first_line;
        elements.push(PathElement::Arc {
            center: tip[0],
            psi_start: angle_of(t_last),
            sweep_rad: turn.to_radians(),
        });
    }
    let (first_from, first_t) = match elements.first() {
        Some(PathElement::Line { from, t, .. }) => (*from, *t),
        _ => {
            return Err(error(
                "KNIFE_CHAIN_DEGENERATE",
                format!("chain '{chain_id}' has no positive-length cutting segment"),
            ));
        }
    };
    Ok(KnifePath {
        chain_id: chain_id.into(),
        elements,
        start_pivot: Point::new(first_from.x + first_t.x, first_from.y + first_t.y),
        first_heading: angle_of(Point::new(-first_t.x, -first_t.y)),
    })
}

impl KnifePath {
    /// The heading carried out of the traversal: the last element's leaving
    /// direction (a trailing line or a closing arc's exit).
    fn last_heading(&self) -> f64 {
        match self.elements.last() {
            Some(PathElement::Line { t, .. }) => angle_of(Point::new(-t.x, -t.y)),
            Some(PathElement::Arc {
                psi_start,
                sweep_rad,
                ..
            }) => psi_start + sweep_rad + std::f64::consts::PI,
            None => self.first_heading,
        }
    }
}

/// Rotate a closed ring so the boundary point nearest `p` becomes the start
/// vertex (a mid-edge nearest point is inserted). Orientation is kept.
fn rotate_ring_to(vertices: &mut Vec<Point>, p: Point) {
    let n = vertices.len();
    let mut best: Option<(f64, usize, f64)> = None;
    for edge in 0..n {
        let a = vertices[edge];
        let b = vertices[(edge + 1) % n];
        let len_sq = a.distance(b).powi(2);
        if len_sq <= 1e-24 {
            continue;
        }
        let t = (((p.x - a.x) * (b.x - a.x) + (p.y - a.y) * (b.y - a.y)) / len_sq).clamp(0., 1.);
        let projected = Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
        let distance = projected.distance(p);
        if best.is_none_or(|(best_distance, _, _)| distance < best_distance) {
            best = Some((distance, edge, t));
        }
    }
    let Some((_, edge, t)) = best else {
        return;
    };
    let mut rotated = vertices.clone();
    rotated.rotate_left(edge + 1);
    if t > 1e-9 && t < 1. - 1e-9 {
        let a = vertices[edge];
        let b = vertices[(edge + 1) % n];
        rotated.insert(0, Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t));
    }
    *vertices = rotated;
}

fn point_in_stock(ctx: &PlanContext, p: Point) -> bool {
    match ctx.setup.stock.xy {
        Some(rect) => {
            p.x > rect.min_x_mm - RESERVE_MM
                && p.y > rect.min_y_mm - RESERVE_MM
                && p.x < rect.min_x_mm + rect.width_mm + RESERVE_MM
                && p.y < rect.min_y_mm + rect.length_mm + RESERVE_MM
        }
        None => false,
    }
}

/// One prior milling sweep this knife path must still find material
/// beneath (plan section 22.10 F3c). The floor is the sweep's deepest
/// commanded Z; the corridor radius is exact for flat endmills and the
/// V-bit's full cutting radius otherwise (conservative: a V-carved surface
/// is treated as removed out to the cone's widest reach until an exact
/// prefix proof exists).
struct PriorSweep {
    operation_id: String,
    a: Point,
    b: Point,
    radius_mm: f64,
    floor_z: f64,
}

fn prior_sweeps(ctx: &PlanContext, prior_motions: &[PlannedMotion]) -> Result<Vec<PriorSweep>> {
    let mut sweeps = vec![];
    for motion in prior_motions
        .iter()
        .filter(|motion| motion.effect == MotionEffect::MillingSweep)
    {
        let tool = ctx.tool(&motion.tool_id).ok_or_else(|| {
            error(
                "STOCK_HISTORY_TOOL",
                format!("prior stage uses unknown tool '{}'", motion.tool_id),
            )
        })?;
        let radius = match &tool.geometry {
            Some(ToolGeometry::Endmill(spec)) => spec.diameter_mm / 2.,
            Some(ToolGeometry::Vbit(spec)) => {
                VBit::try_from(spec.clone())?.max_cutting_radius().mm()
            }
            _ => {
                return Err(error(
                    "STOCK_HISTORY_TOOL",
                    format!(
                        "prior milling stage '{}' uses a tool without milling geometry",
                        motion.stage_id
                    ),
                ));
            }
        };
        sweeps.push(PriorSweep {
            operation_id: motion.operation_id.clone(),
            a: motion.start.xy(),
            b: motion.end.xy(),
            radius_mm: radius,
            floor_z: motion.start.z.min(motion.end.z),
        });
    }
    Ok(sweeps)
}

fn point_segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = Point::new(b.x - a.x, b.y - a.y);
    let len_sq = ab.x * ab.x + ab.y * ab.y;
    if len_sq <= 1e-24 {
        return p.distance(a);
    }
    let t = (((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / len_sq).clamp(0., 1.);
    p.distance(Point::new(a.x + ab.x * t, a.y + ab.y * t))
}

/// Distance between two closed segments (0 when they touch or cross).
fn segment_segment_distance(a1: Point, a2: Point, b1: Point, b2: Point) -> f64 {
    let cross =
        |o: Point, p: Point, q: Point| (p.x - o.x) * (q.y - o.y) - (p.y - o.y) * (q.x - o.x);
    let sign = |o: Point, p: Point, q: Point| {
        let value = cross(o, p, q);
        if value > 1e-15 {
            1
        } else if value < -1e-15 {
            -1
        } else {
            0
        }
    };
    let touching = sign(a1, a2, b1) != sign(a1, a2, b2) && sign(b1, b2, a1) != sign(b1, b2, a2);
    let collinear_overlap = sign(a1, a2, b1) == 0
        && sign(a1, a2, b2) == 0
        && (point_segment_distance(a1, b1, b2) <= 1e-12
            || point_segment_distance(a2, b1, b2) <= 1e-12
            || point_segment_distance(b1, a1, a2) <= 1e-12
            || point_segment_distance(b2, a1, a2) <= 1e-12);
    if touching || collinear_overlap {
        return 0.;
    }
    point_segment_distance(a1, b1, b2)
        .min(point_segment_distance(a2, b1, b2))
        .min(point_segment_distance(b1, a1, a2))
        .min(point_segment_distance(b2, a1, a2))
}

/// Whether a required knife contact corridor still has material. A corridor
/// point/segment at contact depth `z` is lost when a prior sweep passed
/// within its cutter radius of it and removed material below `z`.
fn contact_lost(
    sweeps: &[PriorSweep],
    a: Point,
    b: Point,
    required_z: f64,
    margin_mm: f64,
) -> Option<(&PriorSweep, Point)> {
    let mut nearest: Option<(&PriorSweep, f64)> = None;
    for sweep in sweeps {
        if sweep.floor_z >= required_z - RESERVE_MM {
            // The sweep never reached this contact depth; material at z
            // beneath its corridor is intact.
            continue;
        }
        let distance = segment_segment_distance(a, b, sweep.a, sweep.b);
        if distance <= sweep.radius_mm + margin_mm
            && nearest.is_none_or(|(_, best)| distance < best)
        {
            nearest = Some((sweep, distance));
        }
    }
    nearest.map(|(sweep, _)| {
        let middle = Point::new((a.x + b.x) / 2., (a.y + b.y) / 2.);
        (sweep, middle)
    })
}

/// Resolved knife geometry/engagement values the prefix-contact gate needs.
struct ContactParams {
    swivel_configured: f64,
    corner_threshold: f64,
    linearization_error: f64,
    /// Blade offset (pivot-to-tip distance).
    d: f64,
}

/// The prefix-contact gate (plan section 22.10 F3c): every required
/// contact, alignment and swivel region of every chain must still contain
/// material at its resolved contact depth in the preceding stock prefix.
/// This conservative corridor test is the supported proof; a knife path
/// whose corridor intersects prior milling removal is rejected with
/// `KNIFE_CONTACT_UNSUPPORTED` instead of cutting a planted swivel in
/// cleared air. Knife traces never count as removal (F2 semantics kept).
fn check_prefix_contact(
    paths: &[KnifePath],
    sweeps: &[PriorSweep],
    layers: &[f64],
    heights: &crate::setup::ResolvedHeights,
    params: &ContactParams,
) -> Option<(String, String, Point)> {
    let ContactParams {
        swivel_configured,
        corner_threshold,
        linearization_error,
        d,
    } = *params;
    if sweeps.is_empty() || layers.is_empty() {
        return None;
    }
    // The shallowest layer binds: its cut is the highest contact depth the
    // blade must find material at, and its swivel is the shallowest
    // engagement. Deeper layers only require material further down, which
    // the shallowest requirement implies at the same location.
    let cut_z = layers[0];
    let pass_depth = heights.top_z - cut_z;
    let swivel_z = heights.top_z - swivel_configured.min(pass_depth);
    for path in paths {
        for element in &path.elements {
            match *element {
                PathElement::Line { from, to, .. } => {
                    if let Some((sweep, at)) = contact_lost(sweeps, from, to, cut_z, RESERVE_MM) {
                        return Some((path.chain_id.clone(), sweep.operation_id.clone(), at));
                    }
                }
                PathElement::Arc {
                    center,
                    psi_start,
                    sweep_rad,
                } => {
                    if sweep_rad.to_degrees().abs() >= corner_threshold {
                        // Explicit corner swivel: the tip stays planted at
                        // the corner through the swivel depth.
                        if let Some((sweep, _)) =
                            contact_lost(sweeps, center, center, swivel_z, RESERVE_MM)
                        {
                            return Some((
                                path.chain_id.clone(),
                                sweep.operation_id.clone(),
                                center,
                            ));
                        }
                    } else {
                        // A small turn is cut continuously at pass depth;
                        // check its emitted chords with the linearization
                        // margin so the arc bulge cannot hide a crossing.
                        let sweep = normalize_angle(sweep_rad);
                        let chords = arc_chords(sweep, d, linearization_error);
                        let mut previous = Point::new(
                            center.x + d * psi_start.cos(),
                            center.y + d * psi_start.sin(),
                        );
                        for chord in 1..=chords {
                            let psi = psi_start + sweep * chord as f64 / chords as f64;
                            let next =
                                Point::new(center.x + d * psi.cos(), center.y + d * psi.sin());
                            if let Some((crossing, at)) =
                                contact_lost(sweeps, previous, next, cut_z, linearization_error)
                            {
                                return Some((
                                    path.chain_id.clone(),
                                    crossing.operation_id.clone(),
                                    at,
                                ));
                            }
                            previous = next;
                        }
                    }
                }
            }
        }
    }
    None
}

/// Chord count for linearizing a swivel arc of radius `d` within the error
/// share `e` (plan section 8.4: bound the subdivision angle by
/// `2*acos(1-e/r)`).
fn arc_chords(sweep_rad: f64, radius: f64, e: f64) -> usize {
    if sweep_rad.abs() < 1e-12 {
        return 0;
    }
    let max_step = if e < radius {
        2. * (1. - e / radius).acos()
    } else {
        std::f64::consts::PI
    };
    ((sweep_rad.abs() / max_step).ceil() as usize).max(1)
}

/// Emission context: motions plus the intended tip each motion is compared
/// against by the replay (plan section 12.5).
struct Emitter<'a> {
    operation_id: &'a str,
    stage_id: String,
    tool_id: String,
    motions: Vec<PlannedMotion>,
    intended: Vec<IntendedTip>,
    next_id: usize,
}

impl<'a> Emitter<'a> {
    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        purpose: MotionPurpose,
        interpolation: Interpolation,
        effect: MotionEffect,
        start: Position,
        end: Position,
        feed: Option<f64>,
        heading: Option<(f64, f64)>,
        contour_id: Option<String>,
        pass: usize,
        layer: usize,
        intended: IntendedTip,
    ) {
        self.motions.push(PlannedMotion {
            id: self.next_id,
            operation_id: self.operation_id.into(),
            stage_id: self.stage_id.clone(),
            tool_id: self.tool_id.clone(),
            contour_id,
            pass_id: pass,
            layer,
            interpolation,
            purpose,
            effect,
            start,
            end,
            feed_mm_min: feed,
            blade_heading_deg: heading,
        });
        self.intended.push(intended);
        self.next_id += 1;
    }
}

/// Resolved per-pass values shared by emission. `pass` identifies the
/// chain-and-depth pass within the operation; `layer` is the depth index.
struct PassContext {
    swivel_z: f64,
    pass: usize,
    layer: usize,
}

/// Emit the swivel arc chords around a planted tip at `ctx.swivel_z` with
/// the swivel feed; the caller owns the vertical transitions around it.
#[allow(clippy::too_many_arguments)]
fn emit_swivel_chords(
    emitter: &mut Emitter,
    chain_id: &str,
    center: Point,
    heading_in: f64,
    heading_out: f64,
    ctx: &PassContext,
    swivel_feed: f64,
    linearization_error: f64,
    d: f64,
    alignment: bool,
) {
    let purpose = if alignment {
        MotionPurpose::KnifeAlign
    } else {
        MotionPurpose::KnifeSwivel
    };
    let psi_in = heading_in - std::f64::consts::PI;
    let sweep = normalize_angle(heading_out - heading_in);
    let chords = arc_chords(sweep, d, linearization_error);
    let mut previous = Position {
        x: center.x + d * psi_in.cos(),
        y: center.y + d * psi_in.sin(),
        z: ctx.swivel_z,
    };
    for chord in 1..=chords {
        let psi = psi_in + sweep * chord as f64 / chords as f64;
        let next = Position {
            x: center.x + d * psi.cos(),
            y: center.y + d * psi.sin(),
            z: ctx.swivel_z,
        };
        let fraction = (chord as f64 - 1.) / chords as f64;
        emitter.push(
            purpose,
            Interpolation::LinearFeed,
            MotionEffect::KnifeTrace,
            previous,
            next,
            Some(swivel_feed),
            Some((
                heading_deg(psi_in + sweep * fraction + std::f64::consts::PI),
                heading_deg(psi + std::f64::consts::PI),
            )),
            Some(chain_id.to_string()),
            ctx.pass,
            ctx.layer,
            IntendedTip::Planted(center),
        );
        previous = next;
    }
}

/// Plan one drag-knife operation against the stock prefix the preceding
/// operations actually left behind (plan section 22.10 F3c).
pub(crate) fn plan(
    ctx: &PlanContext,
    operation_id: &str,
    settings: &DragKnifeSettings,
    published_faces: &BTreeMap<String, crate::operations::PublishedFace>,
    prior_motions: &[PlannedMotion],
    geometry: &PlannerGeometry,
) -> Result<PlannedOperation> {
    let missing = missing_fields_ctx(
        ctx,
        operation_id,
        settings,
        crate::operations::profile::artwork_field(geometry),
    );
    if !missing.is_empty() {
        return Ok(incomplete(
            missing
                .iter()
                .map(|d| {
                    issue(
                        "MISSING_MACHINING_SETTING",
                        format!("{} ({})", d.message, d.field_path.as_deref().unwrap_or("")),
                        operation_id,
                    )
                })
                .collect(),
        ));
    }
    let spec = knife_geometry(ctx, settings)?;
    let d = spec.blade_offset_mm;
    let tolerance = ctx.tolerances.motion_tolerance_mm.expect("checked above");
    // Error allocation: import flattening spent its share upstream; swivel
    // linearization spends a quarter of the motion tolerance so the replay
    // budget (the full tolerance, measured against the intended polyline)
    // keeps headroom.
    let linearization_error = tolerance / 4.;
    let catalogue = geometry.catalogue()?;
    let selected = match catalogue.select_chains(&settings.chains) {
        Ok(selected) => selected,
        Err(diag) => {
            return Ok(incomplete(vec![issue(
                &diag.code,
                diag.message,
                operation_id,
            )]));
        }
    };
    // Anchor starts: the anchor must address a selected chain. Closed chains
    // rotate their seam to it; open chains may only start at an endpoint.
    let mut anchored: Option<ResolvedAnchor> = None;
    if let StartSelection::Anchor(anchor) = &settings.start {
        if !settings.chains.iter().any(|id| id == &anchor.contour_id) {
            return Ok(incomplete(vec![issue(
                "CONTOUR_REFERENCE",
                format!(
                    "start anchor references chain '{}' which this knife operation does not select",
                    anchor.contour_id
                ),
                operation_id,
            )]));
        }
        match catalogue.resolve_anchor(anchor) {
            Ok(resolved) => anchored = Some(resolved),
            Err(diag) => {
                return Ok(incomplete(vec![issue(
                    &diag.code,
                    format!("start anchor: {}", diag.message),
                    operation_id,
                )]));
            }
        }
    }
    let published_planes: BTreeMap<String, f64> = published_faces
        .iter()
        .map(|(id, face)| (id.clone(), face.z_mm))
        .collect();
    let heights = match resolve_heights_values(
        ctx.setup.stock.thickness_mm,
        &settings.top,
        &settings.bottom,
        &published_planes,
    ) {
        Ok(heights) => heights,
        Err(diag) => {
            return Ok(incomplete(vec![issue(
                &diag.code,
                diag.message,
                operation_id,
            )]));
        }
    };
    // Through cutting is permission, not a moved bottom (plan section 6.4).
    let allowance = settings.through_cut_allowance_mm.unwrap_or(0.);
    let thickness = ctx.setup.stock.thickness_mm.expect("checked above");
    let below = -thickness - heights.bottom_z;
    if below > allowance + 1e-9 {
        return Ok(incomplete(vec![issue(
            "KNIFE_THROUGH_ALLOWANCE",
            format!(
                "bottom {:.4} is {:.4} mm below the stock bottom; the through-cut allowance is {:.4} mm",
                heights.bottom_z, below, allowance
            ),
            operation_id,
        )]));
    }
    // Knife cut depth must not exceed the declared capability.
    if heights.top_z - heights.bottom_z > spec.max_cut_depth_mm + 1e-9 {
        return Ok(incomplete(vec![issue(
            "KNIFE_DEPTH_CAPABILITY",
            format!(
                "cut depth {:.4} mm exceeds the knife's declared maximum cut depth {:.4} mm",
                heights.top_z - heights.bottom_z,
                spec.max_cut_depth_mm
            ),
            operation_id,
        )]));
    }
    let stepdown = settings
        .stepdown_mm
        .expect("checked above")
        .min(settings.assignment.max_stepdown_mm.expect("checked above"));
    if stepdown <= 0. {
        return Ok(incomplete(vec![issue(
            "KNIFE_PASS_RANGE",
            "the resolved stepdown must be positive",
            operation_id,
        )]));
    }
    let layers = crate::operations::face::depth_layers(&heights, stepdown);
    let clearance = ctx.setup.clearance_above_stock_mm.expect("checked above");
    let cutting_feed = settings
        .assignment
        .cutting_feed_mm_min
        .expect("checked above");
    let plunge_feed = settings
        .assignment
        .plunge_feed_mm_min
        .expect("checked above");
    let swivel_feed = settings
        .assignment
        .swivel_feed_mm_min
        .expect("checked above");
    let swivel_configured = settings.swivel_depth_mm.expect("checked above");
    let corner_threshold = settings.corner_threshold_deg.expect("checked above");
    let overlap = settings.closure_overlap_mm.unwrap_or(0.);
    // KnifeState (plan section 12.3): a tool load begins unknown; the
    // explicit initial-heading setting is the only thing that establishes it.
    let mut heading = settings
        .alignment
        .initial_heading_deg
        .expect("checked above")
        .to_radians();

    // Resolve every chain's travel order and compensated path up front so a
    // later chain's diagnostic cannot leave earlier geometry half-emitted.
    let mut paths = vec![];
    for chain in &selected {
        let mut vertices = chain.vertices.clone();
        if let Some(anchor) = &anchored
            && anchor.contour_id == chain.id
        {
            if chain.closed {
                rotate_ring_to(&mut vertices, anchor.point);
            } else {
                let last = vertices[vertices.len() - 1];
                let near_start = anchor.point.distance(vertices[0]) <= tolerance;
                let near_end = anchor.point.distance(last) <= tolerance;
                if !near_start && !near_end {
                    return Ok(incomplete(vec![issue(
                        "KNIFE_START_RANGE",
                        format!(
                            "chain '{}' is open; its start must be an endpoint, and the anchor at ({:.3}, {:.3}) is mid-chain",
                            chain.id, anchor.point.x, anchor.point.y
                        ),
                        operation_id,
                    )]));
                }
                if near_end && !near_start {
                    vertices.reverse();
                }
            }
        }
        // Alignment/contact swivels plant the tip at the chain start inside
        // actual stock; a start outside stock cannot keep the blade engaged
        // (plan section 12.3).
        if !point_in_stock(ctx, vertices[0]) {
            return Ok(incomplete(vec![issue(
                "KNIFE_ALIGNMENT_UNAVAILABLE",
                format!(
                    "chain '{}' starts at ({:.3}, {:.3}) outside the stock; the blade cannot align in contact there",
                    chain.id, vertices[0].x, vertices[0].y
                ),
                operation_id,
            )]));
        }
        match build_path(&chain.id, &vertices, chain.closed, overlap) {
            Ok(path) => paths.push(path),
            Err(diag) => {
                return Ok(incomplete(vec![issue(
                    &diag.code,
                    diag.message,
                    operation_id,
                )]));
            }
        }
    }

    // Prefix contact (plan section 22.10 F3c): required cutting and swivel
    // regions must still hold material after the preceding milling. With no
    // prior milling sweeps the intact stock trivially satisfies this and a
    // knife-only job stays usable.
    if !prior_motions.is_empty() {
        let sweeps = match prior_sweeps(ctx, prior_motions) {
            Ok(sweeps) => sweeps,
            Err(diag) => {
                return Ok(incomplete(vec![issue(
                    &diag.code,
                    diag.message,
                    operation_id,
                )]));
            }
        };
        if let Some((chain_id, prior_operation, at)) = check_prefix_contact(
            &paths,
            &sweeps,
            &layers,
            &heights,
            &ContactParams {
                swivel_configured,
                corner_threshold,
                linearization_error,
                d,
            },
        ) {
            return Ok(incomplete(vec![issue(
                "KNIFE_CONTACT_UNSUPPORTED",
                format!(
                    "chain '{chain_id}': required knife contact near ({:.3}, {:.3}) intersects material already removed by operation '{prior_operation}'; knife contact over prior milling is not supported in this release",
                    at.x, at.y
                ),
                operation_id,
            )]));
        }
    }

    let stage = format!("{operation_id}-knife");
    let mut emitter = Emitter {
        operation_id,
        stage_id: stage.clone(),
        tool_id: settings.assignment.tool_id.clone(),
        motions: vec![],
        intended: vec![],
        next_id: 0,
    };
    let mut lifted_at: Option<Position> = None;

    for (chain_index, path) in paths.iter().enumerate() {
        let chain_id = path.chain_id.clone();
        for (layer_index, &cut_z) in layers.iter().enumerate() {
            // Resolved swivel depth for this pass: positive and never deeper
            // than the pass (plan section 12.2); shallow passes swivel at
            // full pass depth. The motions carry the resolved Z.
            let pass_depth = heights.top_z - cut_z;
            let ctx = PassContext {
                swivel_z: heights.top_z - swivel_configured.min(pass_depth),
                pass: chain_index * layers.len() + layer_index,
                layer: layer_index,
            };
            let target_heading = path.first_heading;
            let align_turn = signed_turn_deg(
                Point::new(heading.cos(), heading.sin()),
                Point::new(target_heading.cos(), target_heading.sin()),
            );
            if (align_turn.abs() - 180.).abs() < AMBIGUOUS_ALIGNMENT_DEG {
                return Ok(incomplete(vec![issue(
                    "KNIFE_ALIGNMENT_UNAVAILABLE",
                    format!(
                        "chain '{chain_id}': the carried heading ({:.1} deg) opposes the first cut ({:.1} deg) within {AMBIGUOUS_ALIGNMENT_DEG} deg; the alignment swivel direction is ambiguous",
                        heading_deg(heading),
                        heading_deg(target_heading),
                    ),
                    operation_id,
                )]));
            }
            let needs_align = align_turn.abs() > 1e-7;
            // The planted tip is the chain start; the holder hovers above it
            // at the carried heading — never a teleport to the next tangent
            // (plan section 12.3).
            let start_tip = Point::new(
                path.start_pivot.x + d * target_heading.cos(),
                path.start_pivot.y + d * target_heading.sin(),
            );
            let pivot_in = Point::new(
                start_tip.x - d * heading.cos(),
                start_tip.y - d * heading.sin(),
            );
            let pivot_target = path.start_pivot;
            let above = Position {
                x: pivot_in.x,
                y: pivot_in.y,
                z: clearance,
            };
            if let Some(from) = lifted_at
                && ((from.x - above.x).abs() > 1e-9 || (from.y - above.y).abs() > 1e-9)
            {
                emitter.push(
                    MotionPurpose::Clearance,
                    Interpolation::Rapid,
                    MotionEffect::None,
                    from,
                    above,
                    None,
                    Some((heading_deg(heading), heading_deg(heading))),
                    None,
                    ctx.pass,
                    ctx.layer,
                    IntendedTip::Lifted,
                );
            }
            // Approach in air to the material top, then a fed descent with
            // the blade at the carried heading; the tip plants at the start.
            emitter.push(
                MotionPurpose::Approach,
                Interpolation::Rapid,
                MotionEffect::None,
                above,
                Position {
                    x: pivot_in.x,
                    y: pivot_in.y,
                    z: heights.top_z,
                },
                None,
                Some((heading_deg(heading), heading_deg(heading))),
                Some(chain_id.clone()),
                ctx.pass,
                ctx.layer,
                IntendedTip::Lifted,
            );
            emitter.push(
                MotionPurpose::Entry,
                Interpolation::LinearFeed,
                MotionEffect::KnifeTrace,
                Position {
                    x: pivot_in.x,
                    y: pivot_in.y,
                    z: heights.top_z,
                },
                Position {
                    x: pivot_in.x,
                    y: pivot_in.y,
                    z: if needs_align { ctx.swivel_z } else { cut_z },
                },
                Some(plunge_feed),
                Some((heading_deg(heading), heading_deg(heading))),
                Some(chain_id.clone()),
                ctx.pass,
                ctx.layer,
                IntendedTip::Planted(start_tip),
            );
            if needs_align {
                // Contact alignment swivel around the planted tip, then the
                // fed descent to pass depth at the cutting pivot.
                emit_swivel_chords(
                    &mut emitter,
                    &chain_id,
                    start_tip,
                    heading,
                    target_heading,
                    &ctx,
                    swivel_feed,
                    linearization_error,
                    d,
                    true,
                );
                if (cut_z - ctx.swivel_z).abs() > 1e-12 {
                    emitter.push(
                        MotionPurpose::Entry,
                        Interpolation::LinearFeed,
                        MotionEffect::KnifeTrace,
                        Position {
                            x: pivot_target.x,
                            y: pivot_target.y,
                            z: ctx.swivel_z,
                        },
                        Position {
                            x: pivot_target.x,
                            y: pivot_target.y,
                            z: cut_z,
                        },
                        Some(plunge_feed),
                        Some((heading_deg(target_heading), heading_deg(target_heading))),
                        Some(chain_id.clone()),
                        ctx.pass,
                        ctx.layer,
                        IntendedTip::Planted(start_tip),
                    );
                }
            }
            // Traverse the compensated path at pass depth.
            let mut position = Position {
                x: pivot_target.x,
                y: pivot_target.y,
                z: cut_z,
            };
            for element in &path.elements {
                match *element {
                    PathElement::Line { from, to, t } => {
                        let end = Position {
                            x: to.x + d * t.x,
                            y: to.y + d * t.y,
                            z: cut_z,
                        };
                        if (end.x - position.x).abs() <= 1e-12
                            && (end.y - position.y).abs() <= 1e-12
                        {
                            continue;
                        }
                        let travel_heading = angle_of(Point::new(-t.x, -t.y));
                        emitter.push(
                            MotionPurpose::KnifeCut,
                            Interpolation::LinearFeed,
                            MotionEffect::KnifeTrace,
                            position,
                            end,
                            Some(cutting_feed),
                            Some((heading_deg(travel_heading), heading_deg(travel_heading))),
                            Some(chain_id.clone()),
                            ctx.pass,
                            ctx.layer,
                            IntendedTip::Segment(from, to),
                        );
                        position = end;
                    }
                    PathElement::Arc {
                        center,
                        psi_start,
                        sweep_rad,
                    } => {
                        let turn_deg = sweep_rad.to_degrees();
                        let heading_in = psi_start + std::f64::consts::PI;
                        let heading_out = psi_start + sweep_rad + std::f64::consts::PI;
                        if turn_deg.abs() >= corner_threshold {
                            // Explicit corner swivel (plan section 12.1): the
                            // holder rises to the resolved swivel depth —
                            // engaged, never lifted clear — pivots around the
                            // planted tip, and re-enters pass depth.
                            let lift = ctx.swivel_z - cut_z > 1e-12;
                            if lift {
                                emitter.push(
                                    MotionPurpose::KnifeSwivel,
                                    Interpolation::LinearFeed,
                                    MotionEffect::KnifeTrace,
                                    position,
                                    Position {
                                        x: position.x,
                                        y: position.y,
                                        z: ctx.swivel_z,
                                    },
                                    Some(plunge_feed),
                                    Some((heading_deg(heading_in), heading_deg(heading_in))),
                                    Some(chain_id.clone()),
                                    ctx.pass,
                                    ctx.layer,
                                    IntendedTip::Planted(center),
                                );
                            }
                            emit_swivel_chords(
                                &mut emitter,
                                &chain_id,
                                center,
                                heading_in,
                                heading_out,
                                &ctx,
                                swivel_feed,
                                linearization_error,
                                d,
                                false,
                            );
                            let pivot_out = Position {
                                x: center.x + d * (psi_start + sweep_rad).cos(),
                                y: center.y + d * (psi_start + sweep_rad).sin(),
                                z: cut_z,
                            };
                            if lift {
                                emitter.push(
                                    MotionPurpose::KnifeSwivel,
                                    Interpolation::LinearFeed,
                                    MotionEffect::KnifeTrace,
                                    Position {
                                        x: pivot_out.x,
                                        y: pivot_out.y,
                                        z: ctx.swivel_z,
                                    },
                                    pivot_out,
                                    Some(plunge_feed),
                                    Some((heading_deg(heading_out), heading_deg(heading_out))),
                                    Some(chain_id.clone()),
                                    ctx.pass,
                                    ctx.layer,
                                    IntendedTip::Planted(center),
                                );
                            }
                            position = pivot_out;
                        } else {
                            // A small turn below the threshold stays part of
                            // the continuous compensated cut; compensation is
                            // never discarded at small vertices, so many
                            // small turns cannot accumulate tip error (plan
                            // section 12.4).
                            let sweep = normalize_angle(sweep_rad);
                            let chords = arc_chords(sweep, d, linearization_error);
                            let mut previous = Position {
                                x: center.x + d * psi_start.cos(),
                                y: center.y + d * psi_start.sin(),
                                z: cut_z,
                            };
                            for chord in 1..=chords {
                                let psi = psi_start + sweep * chord as f64 / chords as f64;
                                let next = Position {
                                    x: center.x + d * psi.cos(),
                                    y: center.y + d * psi.sin(),
                                    z: cut_z,
                                };
                                let fraction = (chord as f64 - 1.) / chords as f64;
                                emitter.push(
                                    MotionPurpose::KnifeCut,
                                    Interpolation::LinearFeed,
                                    MotionEffect::KnifeTrace,
                                    previous,
                                    next,
                                    Some(cutting_feed),
                                    Some((
                                        heading_deg(
                                            psi_start + sweep * fraction + std::f64::consts::PI,
                                        ),
                                        heading_deg(psi + std::f64::consts::PI),
                                    )),
                                    Some(chain_id.clone()),
                                    ctx.pass,
                                    ctx.layer,
                                    IntendedTip::Planted(center),
                                );
                                previous = next;
                            }
                            position = previous;
                        }
                    }
                }
            }
            // Retract to clearance carrying the modeled outgoing heading; a
            // lifted move never rotates the blade (plan section 12.3).
            heading = path.last_heading();
            let retracted = Position {
                x: position.x,
                y: position.y,
                z: clearance,
            };
            emitter.push(
                MotionPurpose::Clearance,
                Interpolation::Rapid,
                MotionEffect::None,
                position,
                retracted,
                None,
                Some((heading_deg(heading), heading_deg(heading))),
                None,
                ctx.pass,
                ctx.layer,
                IntendedTip::Lifted,
            );
            lifted_at = Some(retracted);
        }
    }

    // Independent kinematic replay (plan section 12.5): an engineering gate
    // on the emitted holder polyline, not a display derivation from the
    // modeled headings.
    let outcome = replay(
        &emitter.motions,
        d,
        &emitter.intended,
        tolerance,
        REPLAY_STEP_BUDGET,
    );
    match outcome.status {
        ReplayStatus::BudgetExhausted => {
            let mut result = incomplete(vec![issue(
                "KNIFE_REPLAY_BUDGET",
                format!(
                    "the independent knife replay exhausted its {}-step integration budget; the result is inconclusive, not truncated",
                    REPLAY_STEP_BUDGET
                ),
                operation_id,
            )]);
            result.status = GenerationStatus::Inconclusive;
            Ok(result)
        }
        ReplayStatus::Exceeded => Ok(incomplete(vec![issue(
            "KNIFE_TIP_ERROR",
            format!(
                "the replayed blade tip deviates {:.4} mm from the intended tip path (budget {:.4} mm, heading error {:.3} deg)",
                outcome.max_tip_deviation_mm, tolerance, outcome.max_heading_error_deg
            ),
            operation_id,
        )])),
        ReplayStatus::Within => {
            let motion_count = emitter.motions.len();
            Ok(PlannedOperation {
                named_outputs: vec![],
                status: GenerationStatus::Complete,
                stages: vec![LocalStage {
                    stage_id: stage,
                    role: StageRole::Knife,
                    tool_id: settings.assignment.tool_id.clone(),
                    motion_range: (0, motion_count),
                    // Knife intent resolves to spindle off and exact path in
                    // this release (plan sections 8.2 and 12.5); the writer
                    // re-establishes the full group after every tool change.
                    intent: ProcessIntent {
                        spindle: ProcessSpindle::Off,
                        coolant: CoolantIntent::Off,
                        path_control: PathControlIntent::ExactPath,
                    },
                }],
                motions: emitter.motions,
                stage_evidence: vec![],
                pass_evidence: vec![],
                issues: vec![],
                preparation: vec![],
            })
        }
    }
}
