//! Trace ordered, placed artwork chains with a passive Z-lift pen.
use super::{LocatedDiagnostic, PlanContext};
use crate::{
    contours::{Contour, ContourCatalogue},
    geometry::{Diagnostic, Point, Result},
    motion::Position,
    project::{
        ToolGeometry,
        v5::{PlotSettingsV5, artwork::wire_id, resolve::incomplete_with},
    },
    sequence::{
        CoolantIntent, GenerationStatus, LocalStage, PathControlIntent, PlannedOperation,
        ProcessIntent, ProcessSpindle, StageRole,
    },
    toolpath::{Interpolation, MotionEffect, MotionPurpose, PlannedMotion},
};

pub(crate) fn missing_fields(
    ctx: &PlanContext,
    id: &str,
    s: &PlotSettingsV5,
) -> Vec<LocatedDiagnostic> {
    let mut fields = Vec::new();
    let mut need =
        |path: &str, message: &str| fields.push(LocatedDiagnostic::missing(id, path, message));
    if ctx.setup.clearance_above_stock_mm.is_none() {
        need("setup.clearance_above_stock_mm", "Set the pen-up clearance");
    }
    if ctx.setup.stock.thickness_mm.is_none() {
        need("setup.stock.thickness_mm", "Set stock thickness");
    }
    if s.chains.is_empty() {
        need("chains", "Select at least one centerline to plot");
    }
    if !matches!(
        ctx.tool(&s.tool_id).and_then(|t| t.geometry),
        Some(ToolGeometry::Pen)
    ) {
        need("tool_id", "Assign a passive pen");
    }
    if s.draw_feed_mm_min.is_none() {
        need("draw_feed_mm_min", "Set the drawing feed");
    }
    if s.lower_feed_mm_min.is_none() {
        need("lower_feed_mm_min", "Set the pen lowering feed");
    }
    fields
}

/// Endpoints in drawing order. Walk backwards from the seam so even a small
/// gap on a long polyline is measured directly, without subtracting totals.
fn stroke_ends(chain: &Contour, gap: f64) -> Result<impl Iterator<Item = Point> + '_> {
    let vertices = &chain.vertices;
    let first = vertices[0];
    let ends = |keep: usize, last: Option<Point>| vertices[1..keep].iter().copied().chain(last);
    if gap == 0. || (!chain.closed && vertices.last() != Some(&first)) {
        return Ok(ends(vertices.len(), chain.closed.then_some(first)));
    }

    let too_long = || {
        Diagnostic::new(
            "PLOT_CLOSURE_GAP",
            format!(
                "Closure gap must be shorter than the {:.6} mm loop '{}'",
                chain.perimeter_mm, chain.id
            ),
        )
    };
    if gap >= chain.perimeter_mm {
        return Err(too_long());
    }
    let mut remaining = gap;
    let mut end = first;
    for (i, &start) in vertices.iter().enumerate().rev() {
        let length = start.distance(end);
        if remaining <= length {
            let tip = if remaining == length {
                start
            } else {
                end.lerp(start, remaining / length)
            };
            if tip == first {
                return Err(Diagnostic::new(
                    "PLOT_CLOSURE_GAP",
                    format!(
                        "Choose a different closure gap for '{}'. The stroke still ends at its starting point.",
                        chain.id
                    ),
                ));
            }
            return Ok(ends(i + 1, (tip != start).then_some(tip)));
        }
        remaining -= length;
        end = start;
    }
    Err(too_long())
}

pub(crate) fn drawable(closed: bool, vertices: usize, perimeter_mm: f64) -> bool {
    vertices >= if closed { 3 } else { 2 } && perimeter_mm > 0.
}

pub(crate) fn plan(
    ctx: &PlanContext,
    id: &str,
    s: &PlotSettingsV5,
    catalogue: &ContourCatalogue,
) -> Result<PlannedOperation> {
    if let Some(field) = missing_fields(ctx, id, s).first() {
        return Ok(incomplete_with(id, field.diagnostic()));
    }
    let clearance = ctx.setup.clearance_above_stock_mm.unwrap();
    let contact = -s.compression_mm;
    let stage = format!("{id}-plot");
    let mut motions = Vec::<PlannedMotion>::new();
    let mut cursor = None;
    let chains: std::collections::HashMap<_, _> = catalogue
        .open_chains
        .iter()
        .map(|c| (c.id.as_str(), c))
        .collect();
    for (index, reference) in s.chains.iter().enumerate() {
        let wire = wire_id(
            &reference.artwork_item_id,
            reference.kind,
            &reference.local_geometry_id,
        );
        let Some(chain) = chains.get(wire.as_str()) else {
            return Ok(incomplete_with(
                id,
                Diagnostic::new(
                    "PLOT_CHAIN_REFERENCE",
                    format!("Missing plot chain '{wire}'"),
                ),
            ));
        };
        if !drawable(chain.closed, chain.vertices.len(), chain.perimeter_mm) {
            return Ok(incomplete_with(
                id,
                Diagnostic::new(
                    "PLOT_DEGENERATE_CHAIN",
                    format!(
                        "Plot chain '{wire}' has no drawable path. Use Remove undrawable paths in the Paths tab."
                    ),
                ),
            ));
        }
        if motions.len() + chain.vertices.len() + 4 > 100_000 {
            return Ok(incomplete_with(
                id,
                Diagnostic::new("PROJECT_RESOURCE_LIMIT", "Plot exceeds the motion budget"),
            ));
        }
        if let Some(r) = ctx.setup.stock.xy
            && chain.vertices.iter().any(|p| {
                p.x < r.min_x_mm
                    || p.y < r.min_y_mm
                    || p.x > r.min_x_mm + r.width_mm
                    || p.y > r.min_y_mm + r.length_mm
            })
        {
            return Ok(incomplete_with(
                id,
                Diagnostic::new(
                    "PLOT_OUTSIDE_STOCK",
                    format!("Plot chain '{wire}' leaves the drawing surface"),
                ),
            ));
        }
        let ends = match stroke_ends(chain, s.closure_gap_mm) {
            Ok(ends) => ends,
            Err(issue) => return Ok(incomplete_with(id, issue)),
        };
        let first = chain.vertices[0];
        let mut pos = cursor.unwrap_or(Position::new(first, clearance));
        let continue_drawing = pos == Position::new(first, contact);
        let lifted = Position::new(pos.xy(), clearance);
        let mut move_to = |end, purpose, interpolation, effect, feed_mm_min| {
            if pos == end {
                return;
            }
            motions.push(PlannedMotion {
                id: motions.len(),
                operation_id: id.into(),
                stage_id: stage.clone(),
                tool_id: s.tool_id.clone(),
                contour_id: Some(wire.clone()),
                pass_id: index,
                layer: 0,
                interpolation,
                purpose,
                effect,
                start: pos,
                end,
                feed_mm_min,
                blade_heading_deg: None,
            });
            pos = end;
        };
        // Only an identical end and start in setup space may share pen contact.
        // A sub-tolerance gap is still a deliberate break in the artwork.
        if !continue_drawing {
            move_to(
                lifted,
                MotionPurpose::Clearance,
                Interpolation::Rapid,
                MotionEffect::None,
                None,
            );
            move_to(
                Position::new(first, clearance),
                MotionPurpose::Clearance,
                Interpolation::Rapid,
                MotionEffect::None,
                None,
            );
            move_to(
                Position::new(first, contact),
                MotionPurpose::Entry,
                Interpolation::LinearFeed,
                MotionEffect::None,
                s.lower_feed_mm_min,
            );
        }
        let mut end = first;
        for p in ends {
            end = p;
            move_to(
                Position::new(p, contact),
                MotionPurpose::Draw,
                Interpolation::LinearFeed,
                MotionEffect::PenTrace,
                s.draw_feed_mm_min,
            );
        }
        if index + 1 == s.chains.len() {
            move_to(
                Position::new(end, clearance),
                MotionPurpose::Clearance,
                Interpolation::Rapid,
                MotionEffect::None,
                None,
            );
        }
        cursor = Some(pos);
    }
    Ok(PlannedOperation {
        status: GenerationStatus::Complete,
        stages: vec![LocalStage {
            stage_id: stage,
            role: StageRole::Plot,
            tool_id: s.tool_id.clone(),
            motion_range: (0, motions.len()),
            intent: ProcessIntent {
                spindle: ProcessSpindle::Off,
                coolant: CoolantIntent::Off,
                path_control: PathControlIntent::ExactPath,
            },
        }],
        motions,
        named_outputs: vec![],
        stage_evidence: vec![],
        pass_evidence: vec![],
        issues: vec![],
        preparation: vec![],
    })
}

/// Independently compare the recorded contact moves with the placed source
/// segments and check that every lateral transfer occurs at job clearance.
pub(crate) fn verify(
    settings: &PlotSettingsV5,
    catalogue: &ContourCatalogue,
    clearance: f64,
    motions: &[PlannedMotion],
) -> Result<()> {
    let contact = -settings.compression_mm;
    let error = |message: &str| Diagnostic::new("PLOT_TRACE", message);
    if motions.first().map(|m| m.start.z) != Some(clearance)
        || motions.last().map(|m| m.end.z) != Some(clearance)
    {
        return Err(error("A plot must enter and exit at clearance"));
    }
    for m in motions {
        let same_xy = m.start.xy() == m.end.xy();
        let valid = match (m.purpose, &m.interpolation, m.effect) {
            (MotionPurpose::Draw, Interpolation::LinearFeed, MotionEffect::PenTrace) => {
                !same_xy
                    && m.start.z == contact
                    && m.end.z == contact
                    && m.feed_mm_min == settings.draw_feed_mm_min
            }
            (MotionPurpose::Entry, Interpolation::LinearFeed, MotionEffect::None) => {
                same_xy
                    && m.start.z == clearance
                    && m.end.z == contact
                    && m.feed_mm_min == settings.lower_feed_mm_min
            }
            (MotionPurpose::Clearance, Interpolation::Rapid, MotionEffect::None) => {
                m.end.z == clearance
                    && (m.start.z == clearance || (same_xy && m.start.z == contact))
            }
            _ => false,
        };
        if !valid || m.tool_id != settings.tool_id {
            return Err(error(
                "Invalid pen contact, feed, tool or travel in the recorded plot",
            ));
        }
    }
    let mut draws = motions
        .iter()
        .filter(|m| m.effect == MotionEffect::PenTrace);
    let mut previous_end = None;
    let mut expected_entries = 0;
    let chains: std::collections::HashMap<_, _> = catalogue
        .open_chains
        .iter()
        .map(|c| (c.id.as_str(), c))
        .collect();
    for reference in &settings.chains {
        let id = wire_id(
            &reference.artwork_item_id,
            reference.kind,
            &reference.local_geometry_id,
        );
        let chain = chains
            .get(id.as_str())
            .ok_or_else(|| error("Selected plot geometry is missing"))?;
        let &first = chain
            .vertices
            .first()
            .ok_or_else(|| error("Empty plot chain"))?;
        let points = stroke_ends(chain, settings.closure_gap_mm)?;
        let mut start = first;
        if previous_end != Some(start) {
            expected_entries += 1;
        }
        for end in points {
            if end == start {
                continue;
            }
            let Some(m) = draws.next() else {
                return Err(error("Plot omits a selected segment"));
            };
            if m.contour_id.as_deref() != Some(&id) || m.start.xy() != start || m.end.xy() != end {
                return Err(error(
                    "Recorded draw moves diverge from the ordered centerlines",
                ));
            }
            start = end;
        }
        previous_end = Some(start);
    }
    if draws.next().is_some()
        || motions
            .iter()
            .filter(|m| m.purpose == MotionPurpose::Entry)
            .count()
            != expected_entries
    {
        return Err(error(
            "Unexpected drawn segment or pen lift between selected paths",
        ));
    }
    Ok(())
}
