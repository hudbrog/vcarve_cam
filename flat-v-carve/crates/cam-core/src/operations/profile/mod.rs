//! Profile planner (plan sections 10.1-10.4, slices D2-E3): explicit-side
//! closed contour offsets, climb/conventional traversal from the retained
//! side, depth passes ending exactly at the bottom, through allowance,
//! rectangular tabs with protected footprints applied to every deep pass,
//! radial finishing, anchor starts, contour ramp entries, and tangent
//! line/arc leads checked against retained geometry and tab volumes.
//! Ramped tab shoulders remain a later slice and produce a specific
//! diagnostic, never a silent change.
mod entries;
mod tabs;

use crate::{
    contours::{ContourRole, ResolvedAnchor},
    geometry::{Diagnostic, Grid, Point, Result},
    motion::Position,
    operations::{LocatedDiagnostic, PlanContext, PlannerGeometry, PublishedFace},
    project::{
        ContourOrder, ContourSide, CutDirection, HeightReference, LeadSpec, MillingAssignment,
        ProfileEntry, ProfileSettings, StartSelection, TabPlacement, TabSettings, ToolGeometry,
        TraversalDirection,
    },
    sequence::{
        CoolantIntent, GenerationStatus, LocalStage, NamedOutput, PathControlIntent, PlanIssue,
        PlannedOperation, ProcessIntent, ProcessSpindle, StageRole, TabPlacementOutput,
    },
    setup::resolve_heights_values,
    toolpath::{Interpolation, MotionEffect, MotionPurpose, PlannedMotion},
};
use entries::LeadShape;
use std::collections::BTreeMap;

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("profile")
}

/// Field name of the artwork container in missing-field diagnostics: the
/// attached single source in schema 4, the collection in schema 5.
pub(crate) fn artwork_field(geometry: &PlannerGeometry) -> &'static str {
    match geometry {
        PlannerGeometry::SourceJob(_) => "source",
        PlannerGeometry::Catalogue(_) => "artwork",
    }
}

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

/// One resolved lead: its shape and its own feed (plan section 10.1).
struct ResolvedLead {
    shape: LeadShape,
    feed: f64,
}

/// Resolved depth entry and leads shared by every loop of the operation.
struct EntrySpecs {
    /// Ramp angle (radians) and feed; `None` is a plunge entry.
    ramp: Option<(f64, f64)>,
    lead_in: Option<ResolvedLead>,
    lead_out: Option<ResolvedLead>,
}

/// A concrete lead polyline in travel order (lead-ins end at the seam,
/// lead-outs start at the traversal end) with its feed.
struct LeadGeometry {
    points: Vec<Point>,
    feed: f64,
}

fn polyline_len(points: &[Point]) -> f64 {
    points
        .windows(2)
        .map(|pair| pair[0].distance(pair[1]))
        .sum()
}

/// Arc length the ramp descends along the loop itself this layer: the ramp
/// starts at the lead-in beginning, so only its overshoot beyond the lead
/// wraps onto the compensated contour (plan section 10.4).
fn ramp_wrap(entry: &EntrySpecs, plunge_from_z: f64, cut_z: f64, lead_in_len: f64) -> f64 {
    entry
        .ramp
        .map(|(angle, _)| ((plunge_from_z - cut_z) / angle.tan() - lead_in_len).max(0.))
        .unwrap_or(0.)
}

/// Rotate a closed loop so its seam is the boundary point nearest `p`,
/// inserting a vertex when that point lies mid-edge. Traversal order is
/// preserved; the seam becomes `vertices[0]`.
fn reposition_seam(vertices: &mut Vec<Point>, p: Point) {
    let n = vertices.len();
    let mut best: Option<(f64, usize, f64)> = None; // (distance, edge, t)
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
        // Mid-edge projection becomes the new seam vertex.
        let a = vertices[edge];
        let b = vertices[(edge + 1) % n];
        rotated.insert(0, Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t));
    }
    *vertices = rotated;
}

/// Distance from a point to a closed loop's boundary.
fn loop_distance(vertices: &[Point], p: Point) -> f64 {
    (0..vertices.len())
        .map(|index| {
            entries::point_segment_distance(
                p,
                vertices[index],
                vertices[(index + 1) % vertices.len()],
            )
        })
        .fold(f64::INFINITY, f64::min)
}

/// Required-but-unset fields for planning this profile operation (schema-4
/// service surface; delegates to the shared context form).
pub fn missing_fields(
    job: &crate::project::CamJob,
    operation_id: &str,
    settings: &ProfileSettings,
) -> Vec<LocatedDiagnostic> {
    missing_fields_ctx(&PlanContext::from_v4(job), operation_id, settings, "source")
}

/// The shared context form: the artwork field names the collection entry
/// ("source" in schema 4, "artwork" in schema 5); the selection itself is
/// checked identically either way.
pub(crate) fn missing_fields_ctx(
    ctx: &PlanContext,
    operation_id: &str,
    settings: &ProfileSettings,
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
            "an SVG source (profiles select artwork contours)",
        );
    }
    if ctx.setup.stock.thickness_mm.is_none() {
        push("setup.stock.thickness_mm".into(), "the stock thickness");
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
    if settings.contours.is_empty() {
        push(
            format!("operations[{operation_id}].contours"),
            "at least one contour with an explicit side",
        );
    }
    if settings.stepdown_mm.is_none() {
        push(
            format!("operations[{operation_id}].stepdown_mm"),
            "the stepdown",
        );
    }
    // Climb/conventional is required whenever a retained side exists; pure
    // on-contour selections traverse by their explicit direction instead.
    if settings.direction.is_none() && settings.contours.iter().any(|c| c.side != ContourSide::On) {
        push(
            format!("operations[{operation_id}].direction"),
            "climb or conventional cutting",
        );
    }
    let tool = ctx.tool(&settings.assignment.tool_id);
    if tool.is_some_and(|t| t.geometry.is_none()) {
        push(
            format!("operations[{operation_id}].assignment.tool"),
            "the milling tool geometry",
        );
    }
    // Ramp entries carry their own angle/feed and an explicit capability.
    if let ProfileEntry::Ramp {
        max_angle_deg,
        feed_mm_min,
    } = &settings.entry
    {
        if max_angle_deg.is_none() {
            push(
                format!("operations[{operation_id}].entry.max_angle_deg"),
                "the ramp angle",
            );
        }
        if feed_mm_min.is_none() {
            push(
                format!("operations[{operation_id}].entry.feed_mm_min"),
                "the ramp feed",
            );
        }
        if tool.is_some_and(|t| t.capabilities.ramp_capable.is_none()) {
            push(
                format!(
                    "tools[{}].capabilities.ramp_capable",
                    settings.assignment.tool_id
                ),
                "whether the tool may ramp",
            );
        }
    }
    // Configured leads carry their geometry and their own feeds.
    for (spec, name) in [
        (&settings.lead_in, "lead_in"),
        (&settings.lead_out, "lead_out"),
    ] {
        match spec {
            LeadSpec::None => {}
            LeadSpec::TangentLine {
                length_mm,
                feed_mm_min,
            } => {
                if length_mm.is_none() {
                    push(
                        format!("operations[{operation_id}].{name}.length_mm"),
                        "the lead length",
                    );
                }
                if feed_mm_min.is_none() {
                    push(
                        format!("operations[{operation_id}].{name}.feed_mm_min"),
                        "the lead feed",
                    );
                }
            }
            LeadSpec::TangentArc {
                radius_mm,
                sweep_deg,
                feed_mm_min,
            } => {
                if radius_mm.is_none() {
                    push(
                        format!("operations[{operation_id}].{name}.radius_mm"),
                        "the lead arc radius",
                    );
                }
                if sweep_deg.is_none() {
                    push(
                        format!("operations[{operation_id}].{name}.sweep_deg"),
                        "the lead arc sweep",
                    );
                }
                if feed_mm_min.is_none() {
                    push(
                        format!("operations[{operation_id}].{name}.feed_mm_min"),
                        "the lead feed",
                    );
                }
            }
        }
    }
    for (value, name) in [
        (settings.assignment.spindle_rpm, "spindle_rpm"),
        (
            settings.assignment.cutting_feed_mm_min,
            "cutting_feed_mm_min",
        ),
        (settings.assignment.plunge_feed_mm_min, "plunge_feed_mm_min"),
        (settings.assignment.max_stepdown_mm, "max_stepdown_mm"),
    ] {
        if value.is_none() {
            push(
                format!("operations[{operation_id}].assignment.{name}"),
                name,
            );
        }
    }
    if settings.assignment.spindle_direction.is_none() {
        push(
            format!("operations[{operation_id}].assignment.spindle_direction"),
            "spindle_direction",
        );
    }
    if let Some(tab) = &settings.tabs {
        if tab.height_mm.is_none() {
            push(
                format!("operations[{operation_id}].tabs.height_mm"),
                "the tab height",
            );
        }
        if tab.width_mm.is_none() {
            push(
                format!("operations[{operation_id}].tabs.width_mm"),
                "the tab width",
            );
        }
    }
    if settings.finish.enabled {
        // Zero allowance is meaningful and supplied, not missing.
        if settings.finish.radial_allowance_mm.is_none() {
            push(
                format!("operations[{operation_id}].finish.radial_allowance_mm"),
                "the radial finishing allowance (zero is meaningful)",
            );
        }
        if settings.finish.feed_mm_min.is_none() {
            push(
                format!("operations[{operation_id}].finish.feed_mm_min"),
                "the finishing feed",
            );
        }
    }
    missing
}

fn cutter_radius(ctx: &PlanContext, assignment: &MillingAssignment) -> Result<f64> {
    let tool = ctx
        .tool(&assignment.tool_id)
        .ok_or_else(|| error("PROJECT_TOOL_REFERENCE", "profile tool not found"))?;
    match &tool.geometry {
        Some(ToolGeometry::Endmill(g)) => Ok(g.diameter_mm / 2.),
        Some(other) => Err(error(
            "PROJECT_TOOL_KIND",
            format!(
                "profile milling requires an endmill, found {}",
                match other {
                    ToolGeometry::Endmill(_) => "endmill",
                    ToolGeometry::Vbit(_) => "vbit",
                    ToolGeometry::DragKnife(_) => "drag_knife",
                }
            ),
        )),
        None => Err(error(
            "MISSING_MACHINING_SETTING",
            "profile tool geometry is required",
        )),
    }
}

/// One compensated centerline loop with the traversal order resolved from
/// cut direction, spindle rotation and the retained side.
struct CompensatedLoop {
    contour_id: String,
    /// Ordered vertices of the loop as it will be cut (closed by returning
    /// to the first vertex).
    vertices: Vec<Point>,
    cut_feed: f64,
    /// Resolved protected tab spans (empty when tabs are disabled).
    tab_spans: Vec<tabs::TabSpan>,
    perimeter_mm: f64,
    /// Resolved lead-in ending at the seam (shared by every layer).
    lead_in: Option<LeadGeometry>,
    /// Resolved lead-out per depth layer: it attaches where the layer's
    /// traversal ends, which moves with the ramp wrap.
    lead_out_by_layer: Vec<Option<LeadGeometry>>,
}

/// Ray-casting containment for nesting order; contours are simple closed
/// canonical rings.
fn ring_contains(vertices: &[Point], p: Point) -> bool {
    let mut inside = false;
    let n = vertices.len();
    for i in 0..n {
        let a = vertices[i];
        let b = vertices[(i + 1) % n];
        if (a.y > p.y) != (b.y > p.y) {
            let x_cross = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if p.x < x_cross {
                inside = !inside;
            }
        }
    }
    inside
}

/// Inner-before-outer ordering: cut deeper nested parts before the enclosing
/// cutout frees them; within one nesting level, holes before the outer.
fn order_contours<'a>(
    contours: &[&'a crate::contours::Contour],
    order: ContourOrder,
) -> Vec<&'a crate::contours::Contour> {
    match order {
        ContourOrder::Explicit => contours.to_vec(),
        ContourOrder::InnerBeforeOuter => {
            let mut keyed: Vec<(usize, u8, String, &crate::contours::Contour)> = contours
                .iter()
                .map(|c| {
                    // A hole belongs to its parent's nesting level, not one
                    // below it: only foreign outers deepen the level.
                    let depth = contours
                        .iter()
                        .filter(|other| {
                            other.id != c.id
                                && Some(other.id.as_str()) != c.parent_contour_id.as_deref()
                                && other.role == ContourRole::Outer
                                && ring_contains(&other.vertices, c.vertices[0])
                        })
                        .count();
                    let role = if c.role == ContourRole::Hole { 0 } else { 1 };
                    (depth, role, c.id.clone(), *c)
                })
                .collect();
            keyed.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
            keyed.into_iter().map(|(_, _, _, c)| c).collect()
        }
    }
}

/// Traverse the canonical (CCW) loop forward or reverse per the resolved
/// cut direction: climb keeps the retained material on the feed-left side
/// for a clockwise spindle, mirrored for counterclockwise.
fn traversal_forward(
    role: ContourRole,
    side: ContourSide,
    direction: CutDirection,
    spindle: crate::project::SpindleDirection,
    on_traversal: Option<TraversalDirection>,
) -> bool {
    if side == ContourSide::On {
        return on_traversal == Some(TraversalDirection::Forward);
    }
    let clockwise = spindle == crate::project::SpindleDirection::Clockwise;
    let climb_keeps_material_left = (direction == CutDirection::Climb) == clockwise;
    // Retained material lies inside the compensated loop only when an outer
    // boundary is cut on its outside (the classic part cutout).
    let material_inside_loop = role == ContourRole::Outer && side == ContourSide::Outside;
    climb_keeps_material_left == material_inside_loop
}

/// Compensated centerline loops for one contour on the explicit side.
fn compensated_loops(
    contour: &crate::contours::Contour,
    side: ContourSide,
    radius: f64,
    grid: Grid,
) -> Result<Vec<Vec<Point>>> {
    let region = contour.enclosed_region(grid)?;
    let offset = match side {
        ContourSide::On => region,
        ContourSide::Inside => region.erode(radius)?,
        ContourSide::Outside => region.dilate(radius)?,
    };
    let mut loops: Vec<Vec<Point>> = offset
        .rings()
        .iter()
        .filter(|ring| !ring.is_hole())
        .map(|ring| {
            ring.points()
                .iter()
                .map(|p| offset.grid().point(*p))
                .collect::<Vec<Point>>()
        })
        .filter(|points| points.len() >= 3)
        .map(|points| canonical_loop(&points))
        .collect();
    loops.sort_by(|a, b| {
        (a[0].x, a[0].y)
            .partial_cmp(&(b[0].x, b[0].y))
            .expect("finite offset vertices")
    });
    Ok(loops)
}

fn canonical_loop(points: &[Point]) -> Vec<Point> {
    let mut pts = points.to_vec();
    if pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    if pts.len() < 3 {
        return pts;
    }
    let area: f64 = (0..pts.len())
        .map(|i| {
            let a = pts[i];
            let b = pts[(i + 1) % pts.len()];
            a.x * b.y - b.x * a.y
        })
        .sum();
    if area < 0. {
        pts.reverse();
    }
    let start = (0..pts.len())
        .min_by(|&a, &b| {
            (pts[a].x, pts[a].y)
                .partial_cmp(&(pts[b].x, pts[b].y))
                .expect("finite offset vertices")
        })
        .expect("nonempty");
    pts.rotate_left(start);
    pts
}

/// Emit one loop's full depth pass sequence per layer: the approach above
/// the entry point, the depth entry (plunge, or a ramp descending along the
/// lead-in and the compensated contour), the optional lead moves, the
/// tab-envelope traversal — wrapping past the seam to flatten a ramped
/// section — and the clearance retract. Shared by rough and finishing
/// passes; only the cutting purpose and feed differ.
#[allow(clippy::too_many_arguments)]
fn emit_loop_layers(
    motions: &mut Vec<PlannedMotion>,
    next_id: &mut usize,
    operation_id: &str,
    stage: &str,
    tool_id: &str,
    cut_loop: &CompensatedLoop,
    layers: &[f64],
    top_z: f64,
    clearance: f64,
    entry_specs: &EntrySpecs,
    plunge_feed: f64,
    cut_purpose: MotionPurpose,
    pass: usize,
) -> std::result::Result<(), Diagnostic> {
    let mut motion = |motions: &mut Vec<PlannedMotion>,
                      purpose: MotionPurpose,
                      interpolation: Interpolation,
                      effect: MotionEffect,
                      start: Position,
                      end: Position,
                      feed: Option<f64>,
                      contour_id: Option<String>,
                      layer: usize| {
        motions.push(PlannedMotion {
            id: *next_id,
            operation_id: operation_id.into(),
            stage_id: stage.into(),
            tool_id: tool_id.into(),
            contour_id,
            pass_id: pass,
            layer,
            interpolation,
            purpose,
            effect,
            start,
            end,
            feed_mm_min: feed,
            blade_heading_deg: None,
        });
        *next_id += 1;
    };
    let vertices = &cut_loop.vertices;
    let vertex_count = vertices.len();
    let perimeter = cut_loop.perimeter_mm;
    let lead_in_len = cut_loop
        .lead_in
        .as_ref()
        .map(|lead| polyline_len(&lead.points))
        .unwrap_or(0.);
    for (layer_index, &cut_z) in layers.iter().enumerate() {
        // Material above this layer along the entry path was already removed
        // by the previous pass of the same loop (its lead and ramp cut the
        // same XY corridor at the previous depth).
        let previous_z = if layer_index == 0 {
            top_z
        } else {
            layers[layer_index - 1]
        };
        // The stage assumes the tool rests at the clearance plane above the
        // entry; descending beyond the previous layer would assert knowledge
        // this planner does not have.
        let plunge_from_z = previous_z.min(clearance);
        let contour = Some(cut_loop.contour_id.clone());
        // The entry point is the lead-in start when configured, else the seam.
        let entry_xy = cut_loop
            .lead_in
            .as_ref()
            .map(|lead| lead.points[0])
            .unwrap_or(vertices[0]);
        if clearance > plunge_from_z {
            motion(
                motions,
                MotionPurpose::Approach,
                Interpolation::Rapid,
                MotionEffect::None,
                Position {
                    x: entry_xy.x,
                    y: entry_xy.y,
                    z: clearance,
                },
                Position {
                    x: entry_xy.x,
                    y: entry_xy.y,
                    z: plunge_from_z,
                },
                None,
                contour.clone(),
                layer_index,
            );
        }
        // Depth entry (plan section 10.4). A ramp descends along the entry
        // path — the lead-in first, then the compensated contour — and the
        // traversal wraps past the seam to flatten the ramped section at
        // full depth; a blocked ramp is an error, never a plunge fallback.
        let wrap = ramp_wrap(entry_specs, plunge_from_z, cut_z, lead_in_len);
        let mut previous = Position {
            x: entry_xy.x,
            y: entry_xy.y,
            z: plunge_from_z,
        };
        match entry_specs.ramp {
            None => {
                motion(
                    motions,
                    MotionPurpose::Entry,
                    Interpolation::LinearFeed,
                    MotionEffect::MillingSweep,
                    previous,
                    Position {
                        x: entry_xy.x,
                        y: entry_xy.y,
                        z: cut_z,
                    },
                    Some(plunge_feed),
                    contour.clone(),
                    layer_index,
                );
                previous.z = cut_z;
                if let Some(lead) = &cut_loop.lead_in {
                    for pair in lead.points.windows(2) {
                        motion(
                            motions,
                            MotionPurpose::LeadIn,
                            Interpolation::LinearFeed,
                            MotionEffect::MillingSweep,
                            Position {
                                x: pair[0].x,
                                y: pair[0].y,
                                z: cut_z,
                            },
                            Position {
                                x: pair[1].x,
                                y: pair[1].y,
                                z: cut_z,
                            },
                            Some(lead.feed),
                            contour.clone(),
                            layer_index,
                        );
                        previous = Position {
                            x: pair[1].x,
                            y: pair[1].y,
                            z: cut_z,
                        };
                    }
                }
            }
            Some((angle, ramp_feed)) => {
                let drop = plunge_from_z - cut_z;
                let ramp_len = drop / angle.tan();
                let slope = drop / ramp_len;
                // Depth at path length s from the entry point.
                let z_at = |s: f64| (plunge_from_z - slope * s).max(cut_z);
                // Lead-in portion, split where the descent completes.
                if let Some(lead) = &cut_loop.lead_in {
                    let mut walked = 0f64;
                    for pair in lead.points.windows(2) {
                        let a = pair[0];
                        let b = pair[1];
                        let len = a.distance(b);
                        if len <= 0. {
                            continue;
                        }
                        if walked + len <= ramp_len + 1e-9 {
                            motion(
                                motions,
                                MotionPurpose::Entry,
                                Interpolation::LinearFeed,
                                MotionEffect::MillingSweep,
                                Position {
                                    x: a.x,
                                    y: a.y,
                                    z: z_at(walked),
                                },
                                Position {
                                    x: b.x,
                                    y: b.y,
                                    z: z_at(walked + len),
                                },
                                Some(ramp_feed),
                                contour.clone(),
                                layer_index,
                            );
                        } else if walked < ramp_len {
                            let t = (ramp_len - walked) / len;
                            let split = a.lerp(b, t);
                            motion(
                                motions,
                                MotionPurpose::Entry,
                                Interpolation::LinearFeed,
                                MotionEffect::MillingSweep,
                                Position {
                                    x: a.x,
                                    y: a.y,
                                    z: z_at(walked),
                                },
                                Position {
                                    x: split.x,
                                    y: split.y,
                                    z: cut_z,
                                },
                                Some(ramp_feed),
                                contour.clone(),
                                layer_index,
                            );
                            motion(
                                motions,
                                MotionPurpose::LeadIn,
                                Interpolation::LinearFeed,
                                MotionEffect::MillingSweep,
                                Position {
                                    x: split.x,
                                    y: split.y,
                                    z: cut_z,
                                },
                                Position {
                                    x: b.x,
                                    y: b.y,
                                    z: cut_z,
                                },
                                Some(lead.feed),
                                contour.clone(),
                                layer_index,
                            );
                        } else {
                            motion(
                                motions,
                                MotionPurpose::LeadIn,
                                Interpolation::LinearFeed,
                                MotionEffect::MillingSweep,
                                Position {
                                    x: a.x,
                                    y: a.y,
                                    z: cut_z,
                                },
                                Position {
                                    x: b.x,
                                    y: b.y,
                                    z: cut_z,
                                },
                                Some(lead.feed),
                                contour.clone(),
                                layer_index,
                            );
                        }
                        walked += len;
                    }
                }
                // Loop portion of the descent, arc [0, wrap].
                let mut consumed = 0f64;
                for edge in 0..vertex_count {
                    if consumed >= wrap - 1e-9 {
                        break;
                    }
                    let a = vertices[edge];
                    let b = vertices[(edge + 1) % vertex_count];
                    let len = a.distance(b);
                    if len <= 0. {
                        continue;
                    }
                    let take = (wrap - consumed).min(len);
                    let end = if take + 1e-9 >= len {
                        b
                    } else {
                        Point::new(
                            a.x + (b.x - a.x) * take / len,
                            a.y + (b.y - a.y) * take / len,
                        )
                    };
                    motion(
                        motions,
                        MotionPurpose::Entry,
                        Interpolation::LinearFeed,
                        MotionEffect::MillingSweep,
                        Position {
                            x: a.x,
                            y: a.y,
                            z: z_at(lead_in_len + consumed),
                        },
                        Position {
                            x: end.x,
                            y: end.y,
                            z: z_at(lead_in_len + consumed + take),
                        },
                        Some(ramp_feed),
                        contour.clone(),
                        layer_index,
                    );
                    consumed += take;
                }
                previous = Position {
                    x: tabs::point_at_arc(vertices, perimeter, wrap).x,
                    y: tabs::point_at_arc(vertices, perimeter, wrap).y,
                    z: cut_z,
                };
            }
        }
        // Traverse the loop under the tab envelope: commanded depth is
        // max(pass_z, tab_top) split exactly at the protected intervals,
        // with feed-controlled vertical rise/lower moves at each boundary
        // (plan section 10.3). With a ramp the walk starts where the descent
        // ended and continues past the seam by the same arc so the ramped
        // section is flattened at full depth. Passes at or above every tab
        // top cut an unsplit loop, byte-identical to the tab-free
        // construction.
        let pieces = tabs::depth_pieces(&cut_loop.tab_spans, cut_z, perimeter);
        let mut current_z = cut_z;
        let arc_end = perimeter + wrap;
        let point_at = |arc: f64| tabs::point_at_arc(vertices, perimeter, arc);
        // Boundaries: vertices and piece edges over the extended arc range.
        let mut bounds: Vec<f64> = vec![0., perimeter];
        let mut vertex_arc = 0f64;
        for index in 0..vertex_count {
            if index > 0 {
                bounds.push(vertex_arc);
            }
            vertex_arc += vertices[index].distance(vertices[(index + 1) % vertex_count]);
        }
        for piece in &pieces {
            bounds.extend([
                piece.start,
                piece.end,
                piece.start + perimeter,
                piece.end + perimeter,
            ]);
        }
        bounds.retain(|bound| *bound > wrap + 1e-9 && *bound < arc_end - 1e-9);
        bounds.sort_by(|a, b| a.partial_cmp(b).expect("finite arcs"));
        bounds.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        let mut spans: Vec<(f64, f64)> = vec![];
        let mut last_bound = wrap;
        for bound in bounds {
            if bound - last_bound > 1e-9 {
                spans.push((last_bound, bound));
            }
            last_bound = bound;
        }
        if arc_end - last_bound > 1e-9 {
            spans.push((last_bound, arc_end));
        }
        for (span_start, span_end) in spans {
            let midpoint = ((span_start + span_end) / 2.).rem_euclid(perimeter);
            let piece = pieces
                .iter()
                .find(|piece| midpoint >= piece.start - 1e-9 && midpoint < piece.end + 1e-9)
                .expect("pieces tile the perimeter");
            let start_xy = point_at(span_start);
            if (start_xy.x - previous.x).abs() > 1e-9 || (start_xy.y - previous.y).abs() > 1e-9 {
                // Defensive: boundaries lie on the path and spans are
                // contiguous; a gap would mean a construction bug.
                return Err(Diagnostic::new(
                    "PROFILE_TAB_PROTECTION_FAILED",
                    format!(
                        "tab envelope split produced a travel gap on pass {pass} layer {layer_index}"
                    ),
                )
                .at_stage("profile"));
            }
            if (piece.z - current_z).abs() > 1e-12 {
                // Vertical transition at the tab boundary (rise into the
                // tab, lower out of it).
                motion(
                    motions,
                    MotionPurpose::TabTransition,
                    Interpolation::LinearFeed,
                    MotionEffect::MillingSweep,
                    Position {
                        x: start_xy.x,
                        y: start_xy.y,
                        z: current_z,
                    },
                    Position {
                        x: start_xy.x,
                        y: start_xy.y,
                        z: piece.z,
                    },
                    Some(plunge_feed),
                    contour.clone(),
                    layer_index,
                );
                current_z = piece.z;
            }
            let end_xy = point_at(span_end);
            let end_position = Position {
                x: end_xy.x,
                y: end_xy.y,
                z: current_z,
            };
            motion(
                motions,
                if piece.on_tab {
                    MotionPurpose::TabTransition
                } else {
                    cut_purpose
                },
                Interpolation::LinearFeed,
                MotionEffect::MillingSweep,
                Position {
                    x: start_xy.x,
                    y: start_xy.y,
                    z: current_z,
                },
                end_position,
                Some(cut_loop.cut_feed),
                contour.clone(),
                layer_index,
            );
            previous = end_position;
        }
        // Lead-out from the traversal end, then retract to clearance.
        if let Some(lead) = cut_loop
            .lead_out_by_layer
            .get(layer_index)
            .and_then(|lead| lead.as_ref())
        {
            for pair in lead.points.windows(2) {
                motion(
                    motions,
                    MotionPurpose::LeadOut,
                    Interpolation::LinearFeed,
                    MotionEffect::MillingSweep,
                    Position {
                        x: pair[0].x,
                        y: pair[0].y,
                        z: cut_z,
                    },
                    Position {
                        x: pair[1].x,
                        y: pair[1].y,
                        z: cut_z,
                    },
                    Some(lead.feed),
                    contour.clone(),
                    layer_index,
                );
                previous = Position {
                    x: pair[1].x,
                    y: pair[1].y,
                    z: cut_z,
                };
            }
        }
        motion(
            motions,
            MotionPurpose::Clearance,
            Interpolation::Rapid,
            MotionEffect::None,
            previous,
            Position {
                z: clearance,
                x: previous.x,
                y: previous.y,
            },
            None,
            None,
            layer_index,
        );
    }
    Ok(())
}

/// Plan one profile operation.
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan(
    ctx: &PlanContext,
    operation_id: &str,
    settings: &ProfileSettings,
    published_faces: &BTreeMap<String, PublishedFace>,
    geometry: &PlannerGeometry,
) -> Result<PlannedOperation> {
    let missing = missing_fields_ctx(ctx, operation_id, settings, artwork_field(geometry));
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
    // Ramp entries need an explicitly ramp-capable tool (plan section 10.1);
    // the unset capability is a missing field, a false one is a conflict.
    if let ProfileEntry::Ramp { .. } = &settings.entry {
        let tool = ctx
            .tool(&settings.assignment.tool_id)
            .expect("tool reference validated");
        if tool.capabilities.ramp_capable == Some(false) {
            return Ok(incomplete(vec![issue(
                "PROFILE_ENTRY_CAPABILITY",
                format!(
                    "the ramp entry needs a ramp-capable tool; '{}' is not ramp capable",
                    settings.assignment.tool_id
                ),
                operation_id,
            )]));
        }
    }
    // Arc leads bulge to the explicit scrap side; on-contour selections have
    // none (a tangent line is collinear with the loop and stays allowed).
    let arc_lead = matches!(settings.lead_in, LeadSpec::TangentArc { .. })
        || matches!(settings.lead_out, LeadSpec::TangentArc { .. });
    if arc_lead && let Some(on) = settings.contours.iter().find(|c| c.side == ContourSide::On) {
        return Ok(incomplete(vec![issue(
            "PROFILE_LEAD_SIDE",
            format!(
                "tangent arc leads need a retained side; contour '{}' is selected on-contour",
                on.contour_id
            ),
            operation_id,
        )]));
    }
    let resolve_lead = |spec: &LeadSpec| -> Option<ResolvedLead> {
        match spec {
            LeadSpec::None => None,
            LeadSpec::TangentLine {
                length_mm,
                feed_mm_min,
            } => Some(ResolvedLead {
                shape: LeadShape::TangentLine {
                    length_mm: length_mm.expect("missing checked above"),
                },
                feed: feed_mm_min.expect("missing checked above"),
            }),
            LeadSpec::TangentArc {
                radius_mm,
                sweep_deg,
                feed_mm_min,
            } => Some(ResolvedLead {
                shape: LeadShape::TangentArc {
                    radius_mm: radius_mm.expect("missing checked above"),
                    sweep_rad: sweep_deg.expect("missing checked above").to_radians(),
                },
                feed: feed_mm_min.expect("missing checked above"),
            }),
        }
    };
    let entry_specs = EntrySpecs {
        ramp: match &settings.entry {
            ProfileEntry::Ramp {
                max_angle_deg,
                feed_mm_min,
            } => Some((
                max_angle_deg.expect("missing checked above").to_radians(),
                feed_mm_min.expect("missing checked above"),
            )),
            ProfileEntry::Plunge => None,
        },
        lead_in: resolve_lead(&settings.lead_in),
        lead_out: resolve_lead(&settings.lead_out),
    };
    // Anchor starts parameterize the source geometry; an anchor for a
    // contour this profile does not select is a located error. Resolution
    // against the catalogue happens once the selection is known below, and
    // a changed source fingerprint is never a nearest-contour guess
    // (plan section 7.2).
    if let StartSelection::Anchor(anchor) = &settings.start
        && !settings
            .contours
            .iter()
            .any(|c| c.contour_id == anchor.contour_id)
    {
        return Ok(incomplete(vec![issue(
            "CONTOUR_REFERENCE",
            format!(
                "start anchor references contour '{}' which this profile does not select",
                anchor.contour_id
            ),
            operation_id,
        )]));
    }

    let catalogue = geometry.catalogue()?;
    let selected = match catalogue.select(
        &settings
            .contours
            .iter()
            .map(|c| c.contour_id.clone())
            .collect::<Vec<_>>(),
    ) {
        Ok(selected) => selected,
        // Unknown contour references are generation outcomes so the rest of
        // the job stays inspectable.
        Err(d) => return Ok(incomplete(vec![issue(&d.code, d.message, operation_id)])),
    };
    let start_anchor = match &settings.start {
        StartSelection::Automatic => None,
        StartSelection::Anchor(anchor) => match catalogue.resolve_anchor(anchor) {
            Ok(resolved) => Some(resolved),
            Err(d) => {
                return Ok(incomplete(vec![issue(
                    &d.code,
                    format!("start anchor: {}", d.message),
                    operation_id,
                )]));
            }
        },
    };

    let radius = cutter_radius(ctx, &settings.assignment)?;
    // A top height referencing a face plane admits only geometry inside that
    // face's established planar coverage (plan section 6.3). The check uses
    // the cutter corridor — source contour dilated by the cutter radius —
    // because the compensated centerline never leaves it on any side.
    if let HeightReference::FaceResult { operation_id: face } = &settings.top.reference {
        let Some(plane) = published_faces.get(face) else {
            return Ok(incomplete(vec![issue(
                "HEIGHT_REFERENCE_UNRESOLVED",
                format!("the face plane of operation '{face}' is not established"),
                operation_id,
            )]));
        };
        let reserve = radius + ctx.tolerances.motion_tolerance_mm.expect("checked above");
        let covered = plane.covered;
        for contour in &selected {
            for vertex in &contour.vertices {
                if vertex.x < covered.min_x_mm - reserve
                    || vertex.y < covered.min_y_mm - reserve
                    || vertex.x > covered.min_x_mm + covered.width_mm + reserve
                    || vertex.y > covered.min_y_mm + covered.length_mm + reserve
                {
                    return Ok(incomplete(vec![issue(
                        "SURFACE_REFERENCE_OUTSIDE_COVERAGE",
                        format!(
                            "contour '{}' lies outside the planar coverage established by face operation '{face}'",
                            contour.id
                        ),
                        operation_id,
                    )]));
                }
            }
        }
    }
    let published_planes: BTreeMap<String, f64> = published_faces
        .iter()
        .map(|(id, face)| (id.clone(), face.z_mm))
        .collect();
    let heights = resolve_heights_values(
        ctx.setup.stock.thickness_mm,
        &settings.top,
        &settings.bottom,
        &published_planes,
    )?;
    // Through cutting is permission, not a moved bottom (plan section 6.4).
    let allowance = settings.through_cut_allowance_mm.unwrap_or(0.);
    if let Some(thickness) = ctx.setup.stock.thickness_mm {
        let below = -thickness - heights.bottom_z;
        if below > allowance + 1e-9 {
            return Ok(incomplete(vec![issue(
                "PROFILE_THROUGH_ALLOWANCE",
                format!(
                    "bottom {:.4} is {:.4} mm below the stock bottom; the through-cut allowance is {:.4} mm",
                    heights.bottom_z, below, allowance
                ),
                operation_id,
            )]));
        }
    }
    let stepdown = settings
        .stepdown_mm
        .expect("checked above")
        .min(settings.assignment.max_stepdown_mm.expect("checked above"));
    if stepdown <= 0. {
        return Ok(incomplete(vec![issue(
            "PROFILE_PASS_RANGE",
            "the stepdown must be positive",
            operation_id,
        )]));
    }
    let layers = crate::operations::face::depth_layers(&heights, stepdown);

    // Tabs (plan section 10.3): the protected bridge is measured from the
    // physical stock bottom, independent of any through-cut allowance.
    let thickness = ctx.setup.stock.thickness_mm.expect("checked above");
    let margin = ctx.tolerances.motion_tolerance_mm.expect("checked above");
    let mut tab_top_z: Option<f64> = None;
    let mut tab_anchors: BTreeMap<String, Vec<ResolvedAnchor>> = BTreeMap::new();
    if let Some(tab) = &settings.tabs {
        let height = tab.height_mm.expect("checked above");
        if height >= thickness {
            return Ok(incomplete(vec![issue(
                "PROFILE_TAB_RANGE",
                format!(
                    "tab height {height} mm must leave stock below the tab top in {thickness} mm stock"
                ),
                operation_id,
            )]));
        }
        let top = -thickness + height;
        if !layers.iter().any(|z| *z < top - 1e-9) {
            return Ok(incomplete(vec![issue(
                "PROFILE_TAB_INEFFECTIVE",
                format!(
                    "no depth pass of this profile reaches below the tab top {:.4}; \
                     the tabs would not hold any material",
                    top
                ),
                operation_id,
            )]));
        }
        // Manual anchors resolve against the source geometry now: a stale
        // fingerprint or unknown contour is a located generation error.
        if let TabPlacement::Manual { anchors } = &tab.placement {
            for anchor in anchors {
                if !settings
                    .contours
                    .iter()
                    .any(|c| c.contour_id == anchor.contour_id)
                {
                    return Ok(incomplete(vec![issue(
                        "CONTOUR_REFERENCE",
                        format!(
                            "tab anchor references contour '{}' which this profile does not select",
                            anchor.contour_id
                        ),
                        operation_id,
                    )]));
                }
                match catalogue.resolve_anchor(anchor) {
                    Ok(resolved) => tab_anchors
                        .entry(anchor.contour_id.clone())
                        .or_default()
                        .push(resolved),
                    Err(d) => {
                        return Ok(incomplete(vec![issue(
                            &d.code,
                            format!("tab anchor: {}", d.message),
                            operation_id,
                        )]));
                    }
                }
            }
        }
        tab_top_z = Some(top);
    }

    // Radial finishing (plan section 10.2, E2): one final pass at the exact
    // offset, depth-stepped like the rough work, with its own feed. Rough
    // offsets carry the allowance; on-contour selections cannot use one.
    let finish = if settings.finish.enabled {
        let allowance = settings.finish.radial_allowance_mm.expect("checked above");
        let feed = settings.finish.feed_mm_min.expect("checked above");
        if !allowance.is_finite() || allowance < 0. || !feed.is_finite() || feed <= 0. {
            return Ok(incomplete(vec![issue(
                "PROFILE_FINISH_RANGE",
                "the finishing allowance must be nonnegative and the feed positive",
                operation_id,
            )]));
        }
        if settings
            .contours
            .iter()
            .any(|c| c.side == ContourSide::On && allowance > 0.)
        {
            return Ok(incomplete(vec![issue(
                "PROFILE_FINISH_ALLOWANCE",
                "on-contour selections have zero offset and cannot use a radial allowance",
                operation_id,
            )]));
        }
        Some((allowance, feed))
    } else {
        None
    };
    let rough_allowance = finish.map(|(allowance, _)| allowance).unwrap_or(0.);

    let ordered = order_contours(&selected, settings.order);
    let grid = Grid::new(
        ctx.tolerances.motion_tolerance_mm.expect("checked above"),
        contour_extent(&ordered),
    )?;
    /// Rough and finishing loops of one contour: its complete work happens
    /// before the next contour's (inner-before-outer, plan section 10.2).
    struct ContourWork {
        rough: Vec<CompensatedLoop>,
        finish: Vec<CompensatedLoop>,
    }
    // Everything the loop builder needs beyond the per-call geometry.
    let clearance = ctx.setup.clearance_above_stock_mm.expect("checked above");
    let lead_in_len = entry_specs
        .lead_in
        .as_ref()
        .map(|lead| lead.shape.path_len())
        .unwrap_or(0.);
    let lead_out_len = entry_specs
        .lead_out
        .as_ref()
        .map(|lead| lead.shape.path_len())
        .unwrap_or(0.);
    // The deepest single-layer drop decides the longest ramp.
    let dz_max = std::iter::once(heights.top_z)
        .chain(layers.iter().copied())
        .zip(layers.iter().copied())
        .map(|(previous_z, cut_z)| previous_z.min(clearance) - cut_z)
        .fold(0., f64::max);
    let ramp_on_loop_max = entry_specs
        .ramp
        .map(|(angle, _)| (dz_max / angle.tan() - lead_in_len).max(0.))
        .unwrap_or(0.);
    // Retained rings of the selection for the lead checks: a lead may only
    // cut where the loop itself cuts (plan section 10.4).
    let retained_rings: Vec<(String, Vec<Point>, ContourSide)> = selected
        .iter()
        .map(|contour| {
            let side = settings
                .contours
                .iter()
                .find(|c| c.contour_id == contour.id)
                .expect("selection produced these contours")
                .side;
            (contour.id.clone(), contour.vertices.clone(), side)
        })
        .collect();
    #[allow(clippy::too_many_arguments)]
    fn build_loops(
        contour: &crate::contours::Contour,
        entry_side: ContourSide,
        offset: f64,
        cut_feed: f64,
        forward: bool,
        grid: Grid,
        tabs: Option<&TabSettings>,
        tab_top_z: Option<f64>,
        tab_anchors: &BTreeMap<String, Vec<ResolvedAnchor>>,
        margin: f64,
        radius: f64,
        entry_specs: &EntrySpecs,
        layers: &[f64],
        top_z: f64,
        clearance: f64,
        lead_in_len: f64,
        lead_out_len: f64,
        ramp_on_loop_max: f64,
        retained_rings: &[(String, Vec<Point>, ContourSide)],
        start_anchor: Option<&ResolvedAnchor>,
    ) -> Result<Vec<CompensatedLoop>> {
        let tolerance = margin;
        let mut loops = vec![];
        let canonical = compensated_loops(contour, entry_side, offset, grid)?;
        // A start anchor applies to its own contour; when an offset split
        // produced several loops, the nearest one carries the seam
        // (deterministic), and the others keep canonical starts.
        let anchor_point = start_anchor
            .filter(|anchor| anchor.contour_id == contour.id)
            .map(|anchor| anchor.point);
        let anchor_target = anchor_point.map(|point| {
            (0..canonical.len())
                .min_by(|&a, &b| {
                    loop_distance(&canonical[a], point)
                        .partial_cmp(&loop_distance(&canonical[b], point))
                        .expect("finite loop distances")
                })
                .expect("nonempty splits")
        });
        for (index, canonical_loop) in canonical.into_iter().enumerate() {
            // The ring is canonicalized CCW from its smallest vertex; the
            // anchor seam and the resolved cut direction decide the travel
            // order, and tab spans are arc-addressed along that final travel.
            let mut vertices = canonical_loop;
            if Some(index) == anchor_target
                && let Some(point) = anchor_point
            {
                reposition_seam(&mut vertices, point);
            }
            if !forward {
                vertices.reverse();
            }
            let perimeter = vertices
                .windows(2)
                .map(|w| w[0].distance(w[1]))
                .sum::<f64>()
                + vertices[vertices.len() - 1].distance(vertices[0]);
            // The ramp must fit one continuous, tab-free revolution; it
            // never wraps over a tab or becomes a plunge (plan 10.4).
            if ramp_on_loop_max >= perimeter {
                return Err(error(
                    "PROFILE_RAMP_NO_SPACE",
                    format!(
                        "the ramp entry needs {:.2} mm of continuous path on contour '{}' \
                         but its compensated loop is only {:.2} mm around; raise the ramp \
                         angle, reduce the stepdown, or add a lead-in",
                        ramp_on_loop_max, contour.id, perimeter
                    ),
                ));
            }
            // Seam neighborhoods occupied by the entry and leads, in arc
            // coordinates of this loop (the lead-in conservatively maps onto
            // the tail it approaches).
            let ramp_interval = (ramp_on_loop_max > 0.).then_some((0., ramp_on_loop_max));
            let lead_tail = (lead_in_len > 0.).then_some((perimeter - lead_in_len, perimeter));
            let lead_prefix =
                (lead_out_len > 0.).then_some((ramp_on_loop_max, ramp_on_loop_max + lead_out_len));
            let mut exclude: Vec<(f64, f64)> = vec![];
            exclude.extend(ramp_interval);
            exclude.extend(lead_tail);
            exclude.extend(lead_prefix);
            let tab_spans = match tabs {
                Some(tab) => tabs::resolve_spans(
                    tab,
                    &vertices,
                    perimeter,
                    offset,
                    radius,
                    margin,
                    tab_top_z.expect("set when tabs are configured"),
                    tab_anchors
                        .get(&contour.id)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                    &contour.id,
                    &exclude,
                )?,
                None => vec![],
            };
            // Manual anchors take precedence over the automatic exclusion,
            // but nothing may pass through a protected bridge.
            for span in &tab_spans {
                let overlaps = |interval: (f64, f64)| {
                    span.restricted.0 < interval.1 - 1e-9 && span.restricted.1 > interval.0 + 1e-9
                };
                if ramp_interval.is_some_and(overlaps) {
                    return Err(error(
                        "PROFILE_RAMP_NO_SPACE",
                        format!(
                            "the ramp entry on contour '{}' would cross the tab bridge at \
                             {:.1}–{:.1} mm; move the tab, raise the ramp angle, or add a lead-in",
                            contour.id, span.bridge.0, span.bridge.1
                        ),
                    ));
                }
                if lead_tail.is_some_and(overlaps) || lead_prefix.is_some_and(overlaps) {
                    return Err(error(
                        "PROFILE_LEAD_NO_SPACE",
                        format!(
                            "a lead on contour '{}' would cross the tab bridge at {:.1}–{:.1} mm; \
                             shorten the lead, move the tab, or choose another start",
                            contour.id, span.bridge.0, span.bridge.1
                        ),
                    ));
                }
            }
            // Lead geometry. The scrap side is explicit for inside/outside
            // cuts; on-contour arcs were rejected at planning entry and lines
            // are collinear with the loop, so they need no normal.
            let retained_inside_loop = entry_side != ContourSide::Inside;
            let first_dir = entries::edge_dir(vertices[0], vertices[1 % vertices.len()])
                .expect("canonical loops have no zero-length first edge");
            let need_scrap = |shape: &LeadShape| matches!(shape, LeadShape::TangentArc { .. });
            let seam_scrap = (need_scrap_shape(entry_specs, true)
                || need_scrap_shape(entry_specs, false))
            .then(|| entries::scrap_normal(first_dir, retained_inside_loop, forward));
            let lead_in = entry_specs.lead_in.as_ref().map(|lead| LeadGeometry {
                points: lead
                    .shape
                    .lead_in_polyline(vertices[0], first_dir, seam_scrap, tolerance),
                feed: lead.feed,
            });
            let lead_out_by_layer: Vec<Option<LeadGeometry>> = layers
                .iter()
                .enumerate()
                .map(|(layer_index, &cut_z)| {
                    let lead = entry_specs.lead_out.as_ref()?;
                    let previous_z = if layer_index == 0 {
                        top_z
                    } else {
                        layers[layer_index - 1]
                    };
                    let wrap =
                        ramp_wrap(entry_specs, previous_z.min(clearance), cut_z, lead_in_len);
                    let attach = tabs::point_at_arc(&vertices, perimeter, wrap);
                    let tangent = tabs::tangent_at_arc(&vertices, wrap).unwrap_or(first_dir);
                    let scrap = need_scrap(&lead.shape)
                        .then(|| entries::scrap_normal(tangent, retained_inside_loop, forward));
                    Some(LeadGeometry {
                        points: lead
                            .shape
                            .lead_out_polyline(attach, tangent, scrap, tolerance),
                        feed: lead.feed,
                    })
                })
                .collect();
            // A configured lead must fit: its whole cutter sweep is checked
            // against the retained side of every selected contour (plan
            // section 10.4), not just its endpoints.
            let slop = 2. * margin + 1e-9;
            let check_lead = |kind: &str, points: &[Point]| -> Result<()> {
                for (ring_id, ring, side) in retained_rings {
                    if *side == ContourSide::On {
                        // On-contour cuts sit on the line by intent; their
                        // retained side is not defined.
                        continue;
                    }
                    if let Some(violation) = entries::lead_violation(
                        points,
                        ring,
                        ring_contains,
                        *side == ContourSide::Outside,
                        radius,
                        slop,
                    ) {
                        let reason = match violation {
                            entries::LeadViolation::InsideRetained => {
                                "enters the retained material".to_string()
                            }
                            entries::LeadViolation::TooClose { distance_mm } => format!(
                                "comes within {distance_mm:.2} mm of the retained boundary \
                                 (the cutter radius is {radius:.2} mm)"
                            ),
                        };
                        return Err(error(
                            "PROFILE_LEAD_NO_SPACE",
                            format!(
                                "the {kind} on contour '{}' {reason} near contour '{ring_id}'; \
                                 shorten it, choose another start, or select another lead",
                                contour.id
                            ),
                        ));
                    }
                }
                Ok(())
            };
            if let Some(lead) = &lead_in {
                check_lead("lead-in", &lead.points)?;
            }
            for lead_out in lead_out_by_layer.iter().flatten() {
                check_lead("lead-out", &lead_out.points)?;
            }
            loops.push(CompensatedLoop {
                contour_id: contour.id.clone(),
                vertices,
                cut_feed,
                tab_spans,
                perimeter_mm: perimeter,
                lead_in,
                lead_out_by_layer,
            });
        }
        Ok(loops)
    }
    fn need_scrap_shape(specs: &EntrySpecs, lead_in: bool) -> bool {
        let lead = if lead_in {
            &specs.lead_in
        } else {
            &specs.lead_out
        };
        lead.as_ref()
            .is_some_and(|l| matches!(l.shape, LeadShape::TangentArc { .. }))
    }
    let mut work: Vec<ContourWork> = vec![];
    for contour in ordered {
        let entry = settings
            .contours
            .iter()
            .find(|c| c.contour_id == contour.id)
            .expect("selection produced these contours");
        if entry.side == ContourSide::On && entry.traversal.is_none() {
            return Ok(incomplete(vec![issue(
                "PROFILE_TRAVERSAL_REQUIRED",
                format!(
                    "on-contour selection '{}' needs an explicit traversal direction",
                    entry.contour_id
                ),
                operation_id,
            )]));
        }
        // Provenance is retained when an offset splits: every result loop is
        // cut, attributed to the same source contour.
        let forward = match settings.direction {
            Some(direction) => traversal_forward(
                contour.role,
                entry.side,
                direction,
                settings
                    .assignment
                    .spindle_direction
                    .expect("checked above"),
                entry.traversal,
            ),
            None => entry.traversal == Some(TraversalDirection::Forward),
        };
        let rough = match build_loops(
            contour,
            entry.side,
            radius + rough_allowance,
            settings
                .assignment
                .cutting_feed_mm_min
                .expect("checked above"),
            forward,
            grid,
            settings.tabs.as_ref(),
            tab_top_z,
            &tab_anchors,
            margin,
            radius,
            &entry_specs,
            &layers,
            heights.top_z,
            clearance,
            lead_in_len,
            lead_out_len,
            ramp_on_loop_max,
            &retained_rings,
            start_anchor.as_ref(),
        ) {
            Ok(loops) => loops,
            Err(d) => return Ok(incomplete(vec![issue(&d.code, d.message, operation_id)])),
        };
        if rough.is_empty() {
            return Ok(incomplete(vec![issue(
                "PROFILE_OFFSET_UNAVAILABLE",
                format!(
                    "the compensated path of contour '{}' on the {:?} side collapses; choose a smaller tool or side",
                    entry.contour_id, entry.side
                ),
                operation_id,
            )]));
        }
        let finish = match finish {
            Some((_, finish_feed)) => {
                let loops = match build_loops(
                    contour,
                    entry.side,
                    radius,
                    finish_feed,
                    forward,
                    grid,
                    settings.tabs.as_ref(),
                    tab_top_z,
                    &tab_anchors,
                    margin,
                    radius,
                    &entry_specs,
                    &layers,
                    heights.top_z,
                    clearance,
                    lead_in_len,
                    lead_out_len,
                    ramp_on_loop_max,
                    &retained_rings,
                    start_anchor.as_ref(),
                ) {
                    Ok(loops) => loops,
                    Err(d) => return Ok(incomplete(vec![issue(&d.code, d.message, operation_id)])),
                };
                if loops.is_empty() {
                    return Ok(incomplete(vec![issue(
                        "PROFILE_OFFSET_UNAVAILABLE",
                        format!(
                            "the finishing path of contour '{}' collapses; choose a smaller tool or side",
                            entry.contour_id
                        ),
                        operation_id,
                    )]));
                }
                loops
            }
            None => vec![],
        };
        work.push(ContourWork { rough, finish });
    }

    let plunge_feed = settings
        .assignment
        .plunge_feed_mm_min
        .expect("checked above");
    let mut motions: Vec<PlannedMotion> = vec![];
    let mut next_id = 0usize;
    // Contiguous same-role runs become stages; a contour's rough and finish
    // work complete before the next contour begins (plan section 10.2), so
    // multi-contour jobs alternate stage roles in nesting order.
    let mut stage_runs: Vec<(String, StageRole, usize)> = vec![];
    let open_stage = |stage_runs: &mut Vec<(String, StageRole, usize)>,
                      role: StageRole,
                      motion_start: usize|
     -> String {
        if let Some(last) = stage_runs.last()
            && last.1 == role
        {
            return last.0.clone();
        }
        let ordinal = stage_runs.iter().filter(|run| run.1 == role).count();
        let stage = format!(
            "{operation_id}-profile-{}{}",
            match role {
                StageRole::ProfileRough => "rough",
                _ => "finish",
            },
            if ordinal == 0 {
                String::new()
            } else {
                format!("-{}", ordinal + 1)
            }
        );
        stage_runs.push((stage.clone(), role, motion_start));
        stage
    };
    let mut pass_counter = 0usize;
    for contour_work in &work {
        for cut_loop in &contour_work.rough {
            let stage = open_stage(&mut stage_runs, StageRole::ProfileRough, motions.len());
            if let Err(d) = emit_loop_layers(
                &mut motions,
                &mut next_id,
                operation_id,
                &stage,
                &settings.assignment.tool_id,
                cut_loop,
                &layers,
                heights.top_z,
                clearance,
                &entry_specs,
                plunge_feed,
                MotionPurpose::Rough,
                pass_counter,
            ) {
                return Ok(incomplete(vec![issue(&d.code, d.message, operation_id)]));
            }
            pass_counter += 1;
        }
        for cut_loop in &contour_work.finish {
            let stage = open_stage(&mut stage_runs, StageRole::ProfileFinish, motions.len());
            if let Err(d) = emit_loop_layers(
                &mut motions,
                &mut next_id,
                operation_id,
                &stage,
                &settings.assignment.tool_id,
                cut_loop,
                &layers,
                heights.top_z,
                clearance,
                &entry_specs,
                plunge_feed,
                MotionPurpose::Finish,
                pass_counter,
            ) {
                return Ok(incomplete(vec![issue(&d.code, d.message, operation_id)]));
            }
            pass_counter += 1;
        }
    }

    let motion_count = motions.len();
    let intent = ProcessIntent {
        spindle: ProcessSpindle::Milling {
            rpm: settings.assignment.spindle_rpm.expect("checked above"),
            direction: settings.assignment.spindle_direction,
        },
        coolant: CoolantIntent::UseMachineProfile,
        path_control: PathControlIntent::UseMachineProfile,
    };
    // Resolved placements are stored in the plan so the preview shows exactly
    // what was generated (plan section 10.3). Rough and finishing loops share
    // the same protected footprints; the rough loops report them, with the
    // bridge cross-section as an exact overlay quad (plan section 15.3).
    let tab_placements: Vec<TabPlacementOutput> = work
        .iter()
        .flat_map(|contour_work| &contour_work.rough)
        .flat_map(|cut_loop| {
            cut_loop
                .tab_spans
                .iter()
                .map(|span| TabPlacementOutput {
                    contour_id: cut_loop.contour_id.clone(),
                    bridge_start_mm: span.bridge.0,
                    bridge_end_mm: span.bridge.1,
                    restricted_start_mm: span.restricted.0,
                    restricted_end_mm: span.restricted.1,
                    top_z_mm: span.top_z,
                    footprint_mm: tabs::span_footprint(
                        &cut_loop.vertices,
                        cut_loop.perimeter_mm,
                        span.bridge,
                        radius + margin,
                    )
                    .unwrap_or_default(),
                })
                .collect::<Vec<_>>()
        })
        .collect();
    Ok(PlannedOperation {
        named_outputs: if tab_placements.is_empty() {
            vec![]
        } else {
            vec![NamedOutput {
                kind: "profile_tabs".into(),
                source_operation_id: Some(operation_id.into()),
                z_mm: None,
                covered: None,
                tab_placements,
            }]
        },
        status: if motion_count == 0 {
            GenerationStatus::Empty
        } else {
            GenerationStatus::Complete
        },
        stages: stage_runs
            .iter()
            .enumerate()
            .map(|(index, (stage_id, role, start))| LocalStage {
                stage_id: stage_id.clone(),
                role: *role,
                tool_id: settings.assignment.tool_id.clone(),
                motion_range: (
                    *start,
                    stage_runs
                        .get(index + 1)
                        .map(|(_, _, next)| *next)
                        .unwrap_or(motion_count),
                ),
                intent: intent.clone(),
            })
            .collect(),
        motions,
        stage_evidence: vec![],
        pass_evidence: vec![],
        issues: vec![],
        preparation: vec![],
    })
}

fn contour_extent(contours: &[&crate::contours::Contour]) -> f64 {
    contours
        .iter()
        .flat_map(|c| c.vertices.iter())
        .map(|p| p.x.abs().max(p.y.abs()))
        .fold(0., f64::max)
}
