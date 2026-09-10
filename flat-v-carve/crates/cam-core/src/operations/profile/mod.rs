//! Profile planner (plan sections 10.1-10.3, slices D2-E1): explicit-side
//! closed contour offsets, climb/conventional traversal from the retained
//! side, depth passes ending exactly at the bottom, through allowance, a
//! simple plunge entry, and rectangular tabs with protected footprints
//! applied to every deep pass. Finishing allowances, leads, ramps and anchor
//! starts ship in later slices and produce specific diagnostics, never
//! silent changes.
mod tabs;

use crate::{
    contours::{ContourCatalogue, ContourRole, ResolvedAnchor},
    geometry::{Diagnostic, Grid, Point, Result},
    motion::Position,
    operations::{LocatedDiagnostic, PublishedFace},
    project::{
        CamJob, ContourOrder, ContourSide, CutDirection, HeightReference, MillingAssignment,
        ProfileEntry, ProfileSettings, StartSelection, TabPlacement, ToolGeometry,
        TraversalDirection,
    },
    sequence::{
        CoolantIntent, GenerationStatus, LocalStage, NamedOutput, PathControlIntent, PlanIssue,
        PlannedOperation, ProcessIntent, ProcessSpindle, StageRole, TabPlacementOutput,
    },
    setup::resolve_heights,
    toolpath::{Interpolation, MotionEffect, MotionPurpose, PlannedMotion},
};
use std::collections::BTreeMap;

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("profile")
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

/// Required-but-unset fields for planning this profile operation.
pub fn missing_fields(
    job: &CamJob,
    operation_id: &str,
    settings: &ProfileSettings,
) -> Vec<LocatedDiagnostic> {
    let mut missing = vec![];
    let mut push = |path: String, what: &str| {
        missing.push(LocatedDiagnostic::missing(
            operation_id,
            &path,
            format!("set {what} before planning operation '{operation_id}'"),
        ));
    };
    if job.source.is_none() {
        push(
            "source".into(),
            "an SVG source (profiles select artwork contours)",
        );
    }
    if job.setup.stock.thickness_mm.is_none() {
        push("setup.stock.thickness_mm".into(), "the stock thickness");
    }
    if job.setup.clearance_above_stock_mm.is_none() {
        push(
            "setup.clearance_above_stock_mm".into(),
            "the clearance plane",
        );
    }
    if job.tolerances.motion_tolerance_mm.is_none() {
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
    let tool = job
        .tools
        .iter()
        .find(|t| t.id == settings.assignment.tool_id);
    if tool.is_some_and(|t| t.geometry.is_none()) {
        push(
            format!("operations[{operation_id}].assignment.tool"),
            "the milling tool geometry",
        );
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

fn cutter_radius(job: &CamJob, assignment: &MillingAssignment) -> Result<f64> {
    let tool = job
        .tools
        .iter()
        .find(|t| t.id == assignment.tool_id)
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

/// Emit one loop's full depth pass sequence: plunge entry per layer, the
/// tab-envelope traversal, and the clearance retract. Shared by rough and
/// finishing passes; only the cutting purpose and feed differ.
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
        });
        *next_id += 1;
    };
    let start_xy = cut_loop.vertices[0];
    for (layer_index, &cut_z) in layers.iter().enumerate() {
        // Material above this layer at the start point was already removed
        // by the previous pass of the same loop.
        let previous_z = if layer_index == 0 {
            top_z
        } else {
            layers[layer_index - 1]
        };
        let entry = Position {
            x: start_xy.x,
            y: start_xy.y,
            z: cut_z,
        };
        // The stage assumes the tool rests at the clearance plane above the
        // entry; descending beyond the previous layer would assert knowledge
        // this planner does not have.
        let plunge_from_z = previous_z.min(clearance);
        let contour = Some(cut_loop.contour_id.clone());
        if clearance > plunge_from_z {
            motion(
                motions,
                MotionPurpose::Approach,
                Interpolation::Rapid,
                MotionEffect::None,
                Position {
                    x: start_xy.x,
                    y: start_xy.y,
                    z: clearance,
                },
                Position {
                    x: start_xy.x,
                    y: start_xy.y,
                    z: plunge_from_z,
                },
                None,
                contour.clone(),
                layer_index,
            );
        }
        motion(
            motions,
            MotionPurpose::Entry,
            Interpolation::LinearFeed,
            MotionEffect::MillingSweep,
            Position {
                x: start_xy.x,
                y: start_xy.y,
                z: plunge_from_z,
            },
            entry,
            Some(plunge_feed),
            contour.clone(),
            layer_index,
        );
        // Traverse the loop under the tab envelope: commanded depth is
        // max(pass_z, tab_top) split exactly at the protected intervals,
        // with feed-controlled vertical rise/lower moves at each boundary
        // (plan section 10.3). Passes at or above every tab top cut an
        // unsplit loop, byte-identical to the tab-free construction.
        let pieces = tabs::depth_pieces(&cut_loop.tab_spans, cut_z, cut_loop.perimeter_mm);
        let mut current_z = cut_z;
        let mut previous = entry;
        let vertex_count = cut_loop.vertices.len();
        for edge in 0..vertex_count {
            let a = cut_loop.vertices[edge];
            let b = cut_loop.vertices[(edge + 1) % vertex_count];
            let edge_len = a.distance(b);
            if edge_len <= 0. {
                continue;
            }
            let edge_start = (0..edge)
                .map(|i| cut_loop.vertices[i].distance(cut_loop.vertices[i + 1]))
                .sum::<f64>();
            for piece in &pieces {
                let lo = piece.start.max(edge_start);
                let hi = piece.end.min(edge_start + edge_len);
                if hi <= lo {
                    continue;
                }
                let at = |arc: f64| {
                    Point::new(
                        a.x + (b.x - a.x) * (arc - edge_start) / edge_len,
                        a.y + (b.y - a.y) * (arc - edge_start) / edge_len,
                    )
                };
                let p = at(lo);
                let q = at(hi);
                if (p.x - previous.x).abs() > 1e-12 || (p.y - previous.y).abs() > 1e-12 {
                    // Defensive: piece boundaries always lie inside edges and
                    // pieces tile the perimeter, so travel is continuous; a
                    // gap would mean a construction bug.
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
                            x: p.x,
                            y: p.y,
                            z: current_z,
                        },
                        Position {
                            x: p.x,
                            y: p.y,
                            z: piece.z,
                        },
                        Some(plunge_feed),
                        contour.clone(),
                        layer_index,
                    );
                    current_z = piece.z;
                    previous = Position {
                        x: p.x,
                        y: p.y,
                        z: piece.z,
                    };
                }
                let end = Position {
                    x: q.x,
                    y: q.y,
                    z: piece.z,
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
                    previous,
                    end,
                    Some(cut_loop.cut_feed),
                    contour.clone(),
                    layer_index,
                );
                previous = end;
            }
        }
        motion(
            motions,
            MotionPurpose::Clearance,
            Interpolation::Rapid,
            MotionEffect::None,
            entry,
            Position {
                z: clearance,
                x: entry.x,
                y: entry.y,
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
    job: &CamJob,
    operation_id: &str,
    settings: &ProfileSettings,
    published_faces: &BTreeMap<String, PublishedFace>,
) -> Result<PlannedOperation> {
    let missing = missing_fields(job, operation_id, settings);
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
    // Later-slice features are explicit unsupported combinations, not silent
    // substitutions (plan section 1.3).
    let unsupported: Vec<PlanIssue> = [
        (
            settings.entry != ProfileEntry::Plunge,
            "PROFILE_ENTRY_UNSUPPORTED",
            "ramp entry ships with the entries slice; configure a plunge or wait",
        ),
        (
            !matches!(settings.start, StartSelection::Automatic),
            "PROFILE_START_UNSUPPORTED",
            "anchor starts ship with the entries slice; the deterministic automatic start is used",
        ),
        (
            !matches!(settings.lead_in, crate::project::LeadSpec::None)
                || !matches!(settings.lead_out, crate::project::LeadSpec::None),
            "PROFILE_LEAD_UNSUPPORTED",
            "lead-in/out ships with the entries slice",
        ),
    ]
    .iter()
    .filter(|(active, _, _)| *active)
    .map(|(_, code, message)| issue(code, (*message).to_string(), operation_id))
    .collect();
    if !unsupported.is_empty() {
        return Ok(incomplete(unsupported));
    }

    let catalogue = ContourCatalogue::build(job)?;
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

    let radius = cutter_radius(job, &settings.assignment)?;
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
        let reserve = radius + job.tolerances.motion_tolerance_mm.expect("checked above");
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
    let heights = resolve_heights(job, 0, &settings.top, &settings.bottom, &published_planes)?;
    // Through cutting is permission, not a moved bottom (plan section 6.4).
    let allowance = settings.through_cut_allowance_mm.unwrap_or(0.);
    if let Some(thickness) = job.setup.stock.thickness_mm {
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
    let thickness = job.setup.stock.thickness_mm.expect("checked above");
    let margin = job.tolerances.motion_tolerance_mm.expect("checked above");
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
        job.tolerances.motion_tolerance_mm.expect("checked above"),
        contour_extent(&ordered),
    )?;
    /// Rough and finishing loops of one contour: its complete work happens
    /// before the next contour's (inner-before-outer, plan section 10.2).
    struct ContourWork {
        rough: Vec<CompensatedLoop>,
        finish: Vec<CompensatedLoop>,
    }
    let build_loops = |contour: &crate::contours::Contour,
                       entry_side: ContourSide,
                       offset: f64,
                       cut_feed: f64,
                       forward: bool|
     -> Result<Vec<CompensatedLoop>> {
        let mut loops = vec![];
        for canonical in compensated_loops(contour, entry_side, offset, grid)? {
            // The ring is canonicalized CCW from its smallest vertex; the
            // resolved cut direction decides the travel order, and tab spans
            // are arc-addressed along that final travel.
            let mut vertices = canonical;
            if !forward {
                vertices.reverse();
            }
            let perimeter = vertices
                .windows(2)
                .map(|w| w[0].distance(w[1]))
                .sum::<f64>()
                + vertices[vertices.len() - 1].distance(vertices[0]);
            let tab_spans = match &settings.tabs {
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
                )?,
                None => vec![],
            };
            loops.push(CompensatedLoop {
                contour_id: contour.id.clone(),
                vertices,
                cut_feed,
                tab_spans,
                perimeter_mm: perimeter,
            });
        }
        Ok(loops)
    };
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
                let loops = match build_loops(contour, entry.side, radius, finish_feed, forward) {
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

    let clearance = job.setup.clearance_above_stock_mm.expect("checked above");
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
    // the same protected footprints; the rough loops report them.
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
