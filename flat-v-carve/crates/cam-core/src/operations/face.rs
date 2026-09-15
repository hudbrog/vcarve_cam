//! Face planner (plan section 11): deterministic rectangular raster at 0 or
//! 90 degrees, depth layers ending exactly at the bottom, coverage-checked
//! against the requested rectangle, with an allowed envelope derived from
//! the settings rather than from whatever paths were generated.
use crate::{
    geometry::{Diagnostic, Result},
    motion::Position,
    operations::{LocatedDiagnostic, PlanContext},
    project::{
        FaceArea, FaceEntry, FaceSettings, MillingAssignment, RectXY, ToolGeometry, WorkZeroXY,
    },
    sequence::{
        CoolantIntent, GenerationStatus, LocalStage, PathControlIntent, PlanIssue,
        PlannedOperation, ProcessIntent, ProcessSpindle, StageRole,
    },
    setup::{ResolvedHeights, resolve_heights_values},
    toolpath::{Interpolation, MotionEffect, MotionPurpose, PlannedMotion},
};
use std::collections::BTreeMap;

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("face")
}

/// Numerical reserve kept between generated paths and the allowed envelope.
const RESERVE_MM: f64 = 1e-6;

pub struct FaceGeometry {
    pub coverage: RectXY,
    /// First/last scan coordinates and their scan-line span.
    pub scan_min: f64,
    pub scan_max: f64,
    pub cross_min: f64,
    pub cross_max: f64,
    /// Rows run along X (0 degrees) or Y (90 degrees).
    pub along_y: bool,
}

/// Expand the requested area by its per-side margins.
pub(crate) fn coverage_rectangle(settings: &FaceSettings, ctx: &PlanContext) -> Result<RectXY> {
    let base = match &settings.area {
        FaceArea::EntireStock => ctx.setup.stock.xy.ok_or_else(|| {
            error(
                "SETUP_STOCK_XY_REQUIRED",
                "facing the entire stock requires physical stock XY dimensions",
            )
        })?,
        FaceArea::Rectangle { rect } => *rect,
    };
    Ok(expand(base, &settings.margins))
}

fn expand(rect: RectXY, margins: &crate::project::FaceMargins) -> RectXY {
    RectXY {
        min_x_mm: rect.min_x_mm - margins.min_x_mm.unwrap_or(0.),
        min_y_mm: rect.min_y_mm - margins.min_y_mm.unwrap_or(0.),
        width_mm: rect.width_mm + margins.min_x_mm.unwrap_or(0.) + margins.max_x_mm.unwrap_or(0.),
        length_mm: rect.length_mm + margins.min_y_mm.unwrap_or(0.) + margins.max_y_mm.unwrap_or(0.),
    }
}

/// Required-but-unset fields for planning this face operation (schema-4
/// service surface; delegates to the shared context form).
pub fn missing_fields(
    job: &crate::project::CamJob,
    operation_id: &str,
    settings: &FaceSettings,
) -> Vec<LocatedDiagnostic> {
    missing_fields_ctx(&PlanContext::from_v4(job), operation_id, settings)
}

pub(crate) fn missing_fields_ctx(
    ctx: &PlanContext,
    operation_id: &str,
    settings: &FaceSettings,
) -> Vec<LocatedDiagnostic> {
    let mut missing = vec![];
    let mut push = |path: String, what: &str| {
        missing.push(LocatedDiagnostic::missing(
            operation_id,
            &path,
            format!("set {what} before planning operation '{operation_id}'"),
        ));
    };
    if ctx.setup.stock.thickness_mm.is_none() {
        push("setup.stock.thickness_mm".into(), "the stock thickness");
    }
    if ctx.setup.clearance_above_stock_mm.is_none() {
        push(
            "setup.clearance_above_stock_mm".into(),
            "the clearance plane",
        );
    }
    if matches!(settings.area, FaceArea::EntireStock) && ctx.setup.stock.xy.is_none() {
        push(
            "setup.stock.xy".into(),
            "physical stock XY dimensions (entire-stock facing)",
        );
    }
    if ctx.tolerances.motion_tolerance_mm.is_none() {
        push(
            "tolerances.motion_tolerance_mm".into(),
            "the motion tolerance",
        );
    }
    if settings.stepdown_mm.is_none() {
        push(
            format!("operations[{operation_id}].stepdown_mm"),
            "the stepdown",
        );
    }
    if settings.stepover_mm.is_none() {
        push(
            format!("operations[{operation_id}].stepover_mm"),
            "the stepover",
        );
    }
    let tool = ctx.tool(&settings.assignment.tool_id);
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
    // Facing uses no work-zero XY dependency; the setup origin keeps legacy
    // documents usable. Anchor selections still transform output only.
    if !matches!(ctx.setup.work_zero.xy, WorkZeroXY::SetupOrigin) && ctx.setup.stock.xy.is_none() {
        push(
            format!("operations[{operation_id}]"),
            "an XY work-zero selection with physical stock dimensions",
        );
    }
    missing
}

fn cutter_radius(ctx: &PlanContext, assignment: &MillingAssignment) -> Result<f64> {
    let tool = ctx
        .tool(&assignment.tool_id)
        .ok_or_else(|| error("PROJECT_TOOL_REFERENCE", "face tool not found"))?;
    match &tool.geometry {
        Some(ToolGeometry::Endmill(g)) => Ok(g.diameter_mm / 2.),
        Some(other) => Err(error(
            "PROJECT_TOOL_KIND",
            format!("face milling requires an endmill, found {}", kind_of(other)),
        )),
        None => Err(error(
            "MISSING_MACHINING_SETTING",
            "face tool geometry is required",
        )),
    }
}

fn kind_of(geometry: &ToolGeometry) -> &'static str {
    match geometry {
        ToolGeometry::Endmill(_) => "endmill",
        ToolGeometry::Vbit(_) => "vbit",
        ToolGeometry::DragKnife(_) => "drag_knife",
    }
}

fn incomplete(_operation_id: &str, issues: Vec<PlanIssue>) -> PlannedOperation {
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

/// Depth layers from the resolved top down to exactly the bottom.
pub fn depth_layers(heights: &ResolvedHeights, stepdown: f64) -> Vec<f64> {
    let depth = heights.top_z - heights.bottom_z;
    if depth <= stepdown {
        return vec![heights.bottom_z];
    }
    let layers = (depth / stepdown).ceil() as usize;
    let mut values = vec![];
    for index in 1..=layers {
        let z = heights.top_z - index as f64 * stepdown;
        values.push(z.max(heights.bottom_z));
    }
    if let Some(last) = values.last_mut() {
        // The final depth is assigned exactly, never left floating.
        *last = heights.bottom_z;
    }
    values
}

/// One raster row: the cross coordinates the pass sweeps (already dilated by
/// the cutter radius) and the scan position it runs at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceRow {
    /// Cross coordinate of the low side: `coverage.min - radius`.
    pub low: f64,
    /// Cross coordinate of the high side: `coverage.max + radius`.
    pub high: f64,
    pub scan: f64,
}

/// Where the passes of one face operation enter (plan section 11.1,
/// 2026-09-15). The user chooses one place: the low end of the pass axis, the
/// high end, an explicit position on that axis, or the pre-2026-09-15 flip
/// between both ends with the depth layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedEntry {
    /// The end every depth layer's first pass descends at (`true` = low).
    anchor_low: bool,
    /// `FaceEntry::Alternate`: the anchor flips with the depth layer, so both
    /// ends of the coverage are entries and both need clearance.
    flip_per_layer: bool,
    /// Travel past the coverage at the end a pass descends from. For an
    /// explicit position this is the distance from the coverage edge to it, so
    /// the coordinate and the travel are the same fact stated two ways.
    entry_travel: f64,
    /// Travel past the coverage at the end a pass leaves.
    exit_travel: f64,
}

impl ResolvedEntry {
    pub fn anchor_low(&self, layer_index: usize) -> bool {
        if self.flip_per_layer {
            layer_index.is_multiple_of(2)
        } else {
            self.anchor_low
        }
    }

    /// Whether the pass that starts this layer runs from the low end.
    pub fn starts_low(&self, layer_index: usize, row_index: usize, zigzag: bool) -> bool {
        let anchor = self.anchor_low(layer_index);
        if zigzag {
            anchor == row_index.is_multiple_of(2)
        } else {
            anchor
        }
    }

    /// Start and end cross coordinates of one pass.
    ///
    /// A pass that **descends** enters at the entry end with the entry travel.
    /// A pass that **continues** its layer (a zig-zag link) begins where the
    /// previous pass ended, which is the far end with the exit travel — it is
    /// already at depth, so it has no entry of its own to clear. The end a
    /// pass leaves always carries the exit travel.
    pub fn pass_span(&self, row: &FaceRow, start_low: bool, descends: bool) -> (f64, f64) {
        let start_travel = if descends {
            self.entry_travel
        } else {
            self.exit_travel
        };
        let start = if start_low {
            row.low - start_travel
        } else {
            row.high + start_travel
        };
        let end = if start_low {
            row.high + self.exit_travel
        } else {
            row.low - self.exit_travel
        };
        (start, end)
    }
}

/// Resolve the requested entry against the coverage. `low_edge` and
/// `high_edge` are the cross coordinates a pass has to reach to sweep the
/// coverage, i.e. the coverage dilated by the cutter radius.
fn resolve_entry(
    settings: &FaceSettings,
    low_edge: f64,
    high_edge: f64,
) -> std::result::Result<ResolvedEntry, (f64, f64, f64)> {
    let entry_overrun = settings.entry_overrun_mm.unwrap_or(0.);
    let exit_travel = settings.exit_overrun_mm.unwrap_or(0.);
    let base = |anchor_low: bool, entry_travel: f64| ResolvedEntry {
        anchor_low,
        flip_per_layer: false,
        entry_travel,
        exit_travel,
    };
    match settings.entry {
        FaceEntry::Min => Ok(base(true, entry_overrun)),
        FaceEntry::Max => Ok(base(false, entry_overrun)),
        FaceEntry::Alternate => Ok(ResolvedEntry {
            flip_per_layer: true,
            ..base(true, entry_overrun)
        }),
        FaceEntry::At { coordinate_mm } => {
            if coordinate_mm <= low_edge + RESERVE_MM {
                Ok(base(true, low_edge - coordinate_mm))
            } else if coordinate_mm >= high_edge - RESERVE_MM {
                Ok(base(false, coordinate_mm - high_edge))
            } else {
                // A pass starting here would leave the strip behind it
                // unswept; the operation is reported rather than shortened.
                Err((coordinate_mm, low_edge, high_edge))
            }
        }
    }
}

/// Raster rows: scan positions spaced by the stepover, extended so the union
/// of cutter sweeps covers the rectangle including corners. Travel is applied
/// per pass by [`ResolvedEntry::pass_span`], because which end of a row is the
/// entry depends on the chosen entry and the pass direction.
pub fn raster_rows(geometry: &FaceGeometry, stepover: f64, radius: f64) -> Vec<FaceRow> {
    let span = geometry.scan_max - geometry.scan_min;
    let first = geometry.scan_min + radius;
    let last_target = geometry.scan_max - radius;
    let row = |scan: f64| FaceRow {
        low: geometry.cross_min - radius,
        high: geometry.cross_max + radius,
        scan,
    };
    if last_target <= first {
        // The cutter covers the whole cross extent in one row.
        return vec![row(geometry.scan_min + span / 2.)];
    }
    let count = ((last_target - first) / stepover).floor() as usize + 1;
    let mut rows = vec![];
    for index in 0..count {
        rows.push(row(first + index as f64 * stepover));
    }
    let last = rows.last().map(|r| r.scan).unwrap_or(first);
    if last_target - last > RESERVE_MM {
        rows.push(row(last_target));
    }
    rows
}

/// Verify the swept bands cover the requested rectangle; report missed strips.
pub fn coverage_gaps(geometry: &FaceGeometry, rows: &[FaceRow], radius: f64) -> Vec<(f64, f64)> {
    let mut gaps = vec![];
    let mut bands: Vec<(f64, f64)> = rows
        .iter()
        .map(|r| (r.scan - radius, r.scan + radius))
        .collect();
    bands.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut covered_to = geometry.scan_min;
    for (low, high) in bands {
        if low > covered_to + RESERVE_MM {
            gaps.push((covered_to, low));
        }
        covered_to = covered_to.max(high);
    }
    if covered_to < geometry.scan_max - RESERVE_MM {
        gaps.push((covered_to, geometry.scan_max));
    }
    gaps
}

/// Distance from a cutter centre to the closest point of the physical stock
/// rectangle, or `None` when no physical stock is known.
fn stock_distance(ctx: &PlanContext, x: f64, y: f64) -> Option<f64> {
    match ctx.setup.stock.xy {
        Some(rect) => {
            let max_x = rect.min_x_mm + rect.width_mm;
            let max_y = rect.min_y_mm + rect.length_mm;
            let dx = if x < rect.min_x_mm {
                rect.min_x_mm - x
            } else if x > max_x {
                x - max_x
            } else {
                0.
            };
            let dy = if y < rect.min_y_mm {
                rect.min_y_mm - y
            } else if y > max_y {
                y - max_y
            } else {
                0.
            };
            Some(dx.hypot(dy))
        }
        None => None,
    }
}

/// Whether a descent at this point can reach stock *material*. A cutter whose
/// sweep only touches the stock boundary has zero overlap with the material —
/// descending there is a side entry, not a plunge — so tangent contact counts
/// as clear. Anything that reaches inside must be authorized by the tool's
/// plunge capability. An unknown stock rectangle is never claimed clear.
fn entry_clear_of_stock(ctx: &PlanContext, x: f64, y: f64, radius: f64) -> bool {
    stock_distance(ctx, x, y).is_some_and(|distance| distance + RESERVE_MM >= radius)
}

/// Where the passes start, in the user's terms, for the entry issues.
fn entry_description(entry: FaceEntry, axis: &str, start_low: bool) -> String {
    match entry {
        FaceEntry::Min => format!("every pass starts at the {axis} minimum edge of the coverage"),
        FaceEntry::Max => format!("every pass starts at the {axis} maximum edge of the coverage"),
        FaceEntry::At { coordinate_mm } => {
            format!("every pass starts at {axis} = {coordinate_mm:.3}")
        }
        FaceEntry::Alternate => format!(
            "the pass start flips between the two ends of the coverage with the depth layer, and this layer starts at the {axis} {} edge",
            if start_low { "minimum" } else { "maximum" }
        ),
    }
}

/// The position on the pass axis, and the entry travel, that put the cutter's
/// sweep clear of the stock for passes starting at `start_low`. The bound is
/// the stock edge plus the cutter radius measured along the pass axis, which is
/// sufficient everywhere (it ignores any help from the perpendicular
/// direction), so the number it reports is never optimistic. `None` when no
/// physical stock rectangle is known.
fn clearing_entry(
    ctx: &PlanContext,
    along_y: bool,
    low_edge: f64,
    high_edge: f64,
    start_low: bool,
    radius: f64,
) -> Option<(f64, f64)> {
    let stock = ctx.setup.stock.xy?;
    let (min, max) = if along_y {
        (stock.min_y_mm, stock.min_y_mm + stock.length_mm)
    } else {
        (stock.min_x_mm, stock.min_x_mm + stock.width_mm)
    };
    Some(if start_low {
        let required = min - radius;
        (required, low_edge - required)
    } else {
        let required = max + radius;
        (required, required - high_edge)
    })
}

fn stage_id(operation_id: &str) -> String {
    format!("{operation_id}-face")
}

/// A point on one row at a given depth. Rows run along X (0 degrees) or Y (90
/// degrees), so a cross coordinate maps to the other axis.
fn cut_position(along_y: bool, cross: f64, scan: f64, z: f64) -> Position {
    Position {
        x: if along_y { scan } else { cross },
        y: if along_y { cross } else { scan },
        z,
    }
}

/// The allowed sweep envelope, derived from the settings and the pass pattern
/// alone — never from a generated path, which could then authorize itself: the
/// coverage, the largest travel the entry mode actually uses at each end of the
/// pass axis, the cutter radius and a numerical reserve.
fn travel_envelope(
    geometry: &FaceGeometry,
    rows: &[FaceRow],
    entry: &ResolvedEntry,
    radius: f64,
    layers: usize,
    zigzag: bool,
) -> RectXY {
    let coverage = geometry.coverage;
    let mut low: f64 = 0.;
    let mut high: f64 = 0.;
    for layer_index in 0..layers {
        for row_index in 0..rows.len() {
            let start_low = entry.starts_low(layer_index, row_index, zigzag);
            let descends = !(zigzag && row_index != 0);
            let start_travel = if descends {
                entry.entry_travel
            } else {
                entry.exit_travel
            };
            let (row_low, row_high) = if start_low {
                (start_travel, entry.exit_travel)
            } else {
                (entry.exit_travel, start_travel)
            };
            low = low.max(row_low);
            high = high.max(row_high);
        }
    }
    let (x_low, x_high, y_low, y_high) = if geometry.along_y {
        (0., 0., low, high)
    } else {
        (low, high, 0., 0.)
    };
    RectXY {
        min_x_mm: coverage.min_x_mm - x_low - radius - RESERVE_MM,
        min_y_mm: coverage.min_y_mm - y_low - radius - RESERVE_MM,
        width_mm: coverage.width_mm + x_low + x_high + 2. * radius + 2. * RESERVE_MM,
        length_mm: coverage.length_mm + y_low + y_high + 2. * radius + 2. * RESERVE_MM,
    }
}

/// What the face editor can state before the operation is generated: the axis
/// the passes run along, the span they have to sweep, every position the
/// cutter descends at, how far its sweep clears the stock there, and the
/// travel that would clear it. The same resolution the planner performs, so
/// the panel cannot promise something the plan contradicts.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceEntryPreview {
    /// The axis the passes run along: `X` at 0 degrees, `Y` at 90.
    pub axis: char,
    /// The requested area, when the operation names one.
    pub area: Option<RectXY>,
    /// The coverage: the requested area plus the margins. What has to end up
    /// flat, and the only part the passes have to sweep.
    pub coverage: RectXY,
    /// The pass's own span on that axis: the coverage dilated by the radius.
    pub pass_low_mm: f64,
    pub pass_high_mm: f64,
    /// Where the cutter descends, in execution order: one per depth layer for
    /// a zig-zag, one per pass when every pass is its own plunge.
    pub entries: Vec<f64>,
    /// The clearance of the cutter's sweep at each entry, in the same order.
    /// Positive values miss the stock entirely; `None` when no stock rectangle
    /// is known.
    pub clearances: Vec<Option<f64>>,
    /// The position and the travel that would put the sweep clear of the stock.
    pub clearing: Option<(f64, f64)>,
    /// The travel the settings use at the entry end and at the exit end.
    pub entry_travel_mm: f64,
    pub exit_travel_mm: f64,
    /// The allowed sweep envelope, in setup coordinates.
    pub envelope: RectXY,
    /// The requested position, when it names an entry the pass cannot start at.
    pub inside_coverage: Option<f64>,
}

/// The entry facts the face panel shows, or `None` while the operation is
/// missing something the resolution needs.
pub(crate) fn entry_preview(
    ctx: &PlanContext,
    settings: &FaceSettings,
) -> Result<Option<FaceEntryPreview>> {
    let angle = settings.pass_angle_deg.unwrap_or(0.);
    if angle != 0. && angle != 90. {
        return Ok(None);
    }
    let Ok(coverage) = coverage_rectangle(settings, ctx) else {
        return Ok(None);
    };
    let Ok(radius) = cutter_radius(ctx, &settings.assignment) else {
        return Ok(None);
    };
    let (Some(stepover), Some(stepdown)) = (settings.stepover_mm, settings.stepdown_mm) else {
        return Ok(None);
    };
    if stepover <= 0. || stepover > 2. * radius - RESERVE_MM {
        return Ok(None);
    }
    let Ok(heights) = resolve_heights_values(
        ctx.setup.stock.thickness_mm,
        &settings.top,
        &settings.bottom,
        &BTreeMap::new(),
    ) else {
        return Ok(None);
    };
    let layers = depth_layers(
        &heights,
        stepdown.min(
            settings
                .assignment
                .max_stepdown_mm
                .unwrap_or(stepdown),
        ),
    );
    let along_y = angle == 90.;
    let geometry = FaceGeometry {
        along_y,
        scan_min: if along_y {
            coverage.min_x_mm
        } else {
            coverage.min_y_mm
        },
        scan_max: if along_y {
            coverage.min_x_mm + coverage.width_mm
        } else {
            coverage.min_y_mm + coverage.length_mm
        },
        cross_min: if along_y {
            coverage.min_y_mm
        } else {
            coverage.min_x_mm
        },
        cross_max: if along_y {
            coverage.min_y_mm + coverage.length_mm
        } else {
            coverage.min_x_mm + coverage.width_mm
        },
        coverage,
    };
    let low_edge = geometry.cross_min - radius;
    let high_edge = geometry.cross_max + radius;
    // An entry the pass cannot start at is reported, not shown as if it were a
    // resolution: no entry positions are published for it, and the panel states
    // the position and both allowed ones instead.
    let (entry, inside_coverage) = match resolve_entry(settings, low_edge, high_edge) {
        Ok(entry) => (entry, None),
        Err((coordinate, _, _)) => (
            ResolvedEntry {
                anchor_low: true,
                flip_per_layer: false,
                entry_travel: settings.entry_overrun_mm.unwrap_or(0.),
                exit_travel: settings.exit_overrun_mm.unwrap_or(0.),
            },
            Some(coordinate),
        ),
    };
    let zigzag = settings.pattern == crate::project::FacePattern::ZigZag;
    let rows = raster_rows(&geometry, stepover, radius);
    let mut entries = vec![];
    let mut clearances = vec![];
    if inside_coverage.is_none() {
        for (layer_index, _) in layers.iter().enumerate() {
            for (row_index, row) in rows.iter().enumerate() {
                if zigzag && row_index != 0 {
                    continue;
                }
                let start_low = entry.starts_low(layer_index, row_index, zigzag);
                let (start_cross, _) = entry.pass_span(row, start_low, true);
                let point = cut_position(along_y, start_cross, row.scan, 0.);
                entries.push(start_cross);
                clearances.push(stock_distance(ctx, point.x, point.y).map(|d| d - radius));
            }
        }
    }
    let clearing = entries.first().and_then(|_| {
        let start_low = entry.starts_low(0, 0, zigzag);
        clearing_entry(ctx, along_y, low_edge, high_edge, start_low, radius)
    });
    let envelope = travel_envelope(&geometry, &rows, &entry, radius, layers.len(), zigzag);
    Ok(Some(FaceEntryPreview {
        axis: if along_y { 'Y' } else { 'X' },
        area: match &settings.area {
            FaceArea::Rectangle { rect } => Some(*rect),
            FaceArea::EntireStock => None,
        },
        coverage,
        pass_low_mm: low_edge,
        pass_high_mm: high_edge,
        entries,
        clearances,
        clearing,
        entry_travel_mm: entry.entry_travel,
        exit_travel_mm: entry.exit_travel,
        envelope,
        inside_coverage,
    }))
}

/// Plan one face operation.
pub(crate) fn plan(
    ctx: &PlanContext,
    operation_id: &str,
    settings: &FaceSettings,
) -> Result<PlannedOperation> {
    let missing = missing_fields_ctx(ctx, operation_id, settings);
    if !missing.is_empty() {
        return Ok(incomplete(
            operation_id,
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
    let angle = settings.pass_angle_deg.unwrap_or(0.);
    if angle != 0. && angle != 90. {
        return Ok(incomplete(
            operation_id,
            vec![issue(
                "FACE_ANGLE_UNSUPPORTED",
                format!(
                    "pass angle {angle} degrees is not supported yet; only 0 and 90 ship with the face milestone"
                ),
                operation_id,
            )],
        ));
    }
    let coverage = coverage_rectangle(settings, ctx)?;
    let radius = cutter_radius(ctx, &settings.assignment)?;
    let stepover = settings.stepover_mm.expect("checked above");
    let stepdown = settings
        .stepdown_mm
        .expect("checked above")
        .min(settings.assignment.max_stepdown_mm.expect("checked above"));
    if stepover <= 0. || stepover > 2. * radius - RESERVE_MM {
        return Ok(incomplete(
            operation_id,
            vec![issue(
                "FACE_STEPOVER_RANGE",
                format!(
                    "stepover {stepover} mm must be positive and no greater than the cutter diameter minus a numerical reserve ({:.4} mm)",
                    2. * radius
                ),
                operation_id,
            )],
        ));
    }
    let heights = resolve_heights_values(
        ctx.setup.stock.thickness_mm,
        &settings.top,
        &settings.bottom,
        &BTreeMap::new(),
    )?;
    let layers = depth_layers(&heights, stepdown);
    let geometry = FaceGeometry {
        along_y: angle == 90.,
        scan_min: if angle == 90. {
            coverage.min_x_mm
        } else {
            coverage.min_y_mm
        },
        scan_max: if angle == 90. {
            coverage.min_x_mm + coverage.width_mm
        } else {
            coverage.min_y_mm + coverage.length_mm
        },
        cross_min: if angle == 90. {
            coverage.min_y_mm
        } else {
            coverage.min_x_mm
        },
        cross_max: if angle == 90. {
            coverage.min_y_mm + coverage.length_mm
        } else {
            coverage.min_x_mm + coverage.width_mm
        },
        coverage,
    };
    let low_edge = geometry.cross_min - radius;
    let high_edge = geometry.cross_max + radius;
    let axis = if geometry.along_y { "Y" } else { "X" };
    let entry = match resolve_entry(settings, low_edge, high_edge) {
        Ok(entry) => entry,
        Err((coordinate, low_edge, high_edge)) => {
            return Ok(incomplete(
                operation_id,
                vec![issue(
                    "FACE_ENTRY_INSIDE_COVERAGE",
                    format!(
                        "the face entry at {axis} = {coordinate:.3} is inside the pass's own span: covering the requested area needs every pass to run from {axis} = {low_edge:.3} to {axis} = {high_edge:.3}, so a pass starting in between would leave the strip behind it unswept. Start at or below {axis} = {low_edge:.3}, or at or above {axis} = {high_edge:.3}"
                    ),
                    operation_id,
                )],
            ));
        }
    };
    let rows = raster_rows(&geometry, stepover, radius);
    let gaps = coverage_gaps(&geometry, &rows, radius);
    if !gaps.is_empty() {
        return Ok(incomplete(
            operation_id,
            vec![issue(
                "FACE_COVERAGE_INCOMPLETE",
                format!(
                    "the raster leaves {} uncovered strip(s): {:?}",
                    gaps.len(),
                    gaps.iter()
                        .map(|(a, b)| format!("{a:.3}..{b:.3}"))
                        .collect::<Vec<_>>()
                ),
                operation_id,
            )],
        ));
    }
    let zigzag = settings.pattern == crate::project::FacePattern::ZigZag;
    let envelope = travel_envelope(&geometry, &rows, &entry, radius, layers.len(), zigzag);
    let clearance = ctx.setup.clearance_above_stock_mm.expect("checked above");
    let cutting_feed = settings
        .assignment
        .cutting_feed_mm_min
        .expect("checked above");
    let plunge_feed = settings
        .assignment
        .plunge_feed_mm_min
        .expect("checked above");
    let rpm = settings.assignment.spindle_rpm.expect("checked above");

    // Every pass that descends to cutting depth must do so where the cutter is
    // clear of the stock. A zig-zag row after the first is entered by a
    // cutting link that is already at depth; the first row of each layer and
    // every one-way row descend instead, so those are the entries this
    // operation has to prove. A tool that cannot plunge must never be sent
    // down through material, whatever the preview appears to show.
    let mut unclear_entry: Option<(Position, bool)> = None;
    'layers: for (layer_index, &cut_z) in layers.iter().enumerate() {
        for (row_index, row) in rows.iter().enumerate() {
            if zigzag && row_index != 0 {
                continue;
            }
            let start_low = entry.starts_low(layer_index, row_index, zigzag);
            let (start_cross, _) = entry.pass_span(row, start_low, true);
            let point = cut_position(geometry.along_y, start_cross, row.scan, cut_z);
            if !entry_clear_of_stock(ctx, point.x, point.y, radius) {
                unclear_entry = Some((point, start_low));
                break 'layers;
            }
        }
    }
    if let Some((point, start_low)) = unclear_entry {
        let capability = ctx
            .tool(&settings.assignment.tool_id)
            .and_then(|tool| tool.capabilities.plunge_capable);
        if capability != Some(true) {
            let description = entry_description(settings.entry, axis, start_low);
            let advice = match clearing_entry(
                ctx,
                geometry.along_y,
                low_edge,
                high_edge,
                start_low,
                radius,
            ) {
                Some((required, _)) if matches!(settings.entry, FaceEntry::At { .. }) => {
                    format!("the cutter only clears the stock from {axis} = {required:.3} outward")
                }
                Some((required, travel)) => format!(
                    "the cutter only clears the stock from {axis} = {required:.3} outward, which needs at least {travel:.3} mm of entry travel (currently {:.3} mm)",
                    entry.entry_travel
                ),
                None => "add entry travel until the cutter sweep clears the stock".into(),
            };
            let (code, message) = match capability {
                Some(false) => (
                    "FACE_ENTRY_UNSAFE",
                    format!(
                        "the tool is marked as unable to plunge, and the Ø{:.3} cutter descends into the stock at ({:.3}, {:.3}) where {description}: {advice}; or choose an entry that clears the stock, or mark the tool as able to plunge",
                        radius * 2.,
                        point.x,
                        point.y,
                    ),
                ),
                _ => (
                    "MISSING_MACHINING_SETTING",
                    format!(
                        "set operations[{operation_id}].assignment.tool.capabilities.plunge_capable before planning operation '{operation_id}': the descent at ({:.3}, {:.3}) where {description} goes through material, so the planner must know whether this tool can plunge",
                        point.x, point.y
                    ),
                ),
            };
            return Ok(incomplete(
                operation_id,
                vec![issue(code, message, operation_id)],
            ));
        }
    }

    let stage = stage_id(operation_id);
    let mut motions: Vec<PlannedMotion> = vec![];
    let mut next_id = 0usize;
    let mut motion = |purpose: MotionPurpose,
                      interpolation: Interpolation,
                      effect: MotionEffect,
                      start: Position,
                      end: Position,
                      feed: Option<f64>|
     -> PlannedMotion {
        let motion = PlannedMotion {
            id: next_id,
            operation_id: operation_id.into(),
            stage_id: stage.clone(),
            tool_id: settings.assignment.tool_id.clone(),
            contour_id: None,
            pass_id: 0,
            layer: 0,
            interpolation,
            purpose,
            effect,
            start,
            end,
            feed_mm_min: feed,
            blade_heading_deg: None,
        };
        next_id += 1;
        motion
    };

    let mut previous_end: Option<Position> = None;
    for (layer_index, &cut_z) in layers.iter().enumerate() {
        for (row_index, row) in rows.iter().enumerate() {
            let start_low = entry.starts_low(layer_index, row_index, zigzag);
            let descends = !(zigzag && row_index != 0);
            let (start_cross, end_cross) = entry.pass_span(row, start_low, descends);
            let start_xy = |c: f64| cut_position(geometry.along_y, c, row.scan, cut_z);
            let entry = start_xy(start_cross);
            let exit = start_xy(end_cross);
            // Entry: cross-row link (zigzag), or retract/rapid/descend.
            match previous_end {
                // Only a pass that continues the same layer links at depth.
                // The first pass of a layer always descends, and it does so
                // from the retract the previous layer ended with.
                Some(prev) if zigzag && row_index != 0 => {
                    let link = motion(
                        MotionPurpose::Rough,
                        Interpolation::LinearFeed,
                        MotionEffect::MillingSweep,
                        prev,
                        Position { z: cut_z, ..entry },
                        Some(cutting_feed),
                    );
                    motions.push(link);
                }
                Some(prev) => {
                    let lifted = Position {
                        z: clearance,
                        x: prev.x,
                        y: prev.y,
                    };
                    if prev.z < clearance - RESERVE_MM {
                        motions.push(motion(
                            MotionPurpose::Clearance,
                            Interpolation::Rapid,
                            MotionEffect::None,
                            prev,
                            lifted,
                            None,
                        ));
                    }
                    // The travel to the entry is a motion of its own: the post
                    // emits one block per motion endpoint, so a transition the
                    // plan leaves out becomes a diagonal rapid from wherever
                    // the tool really is.
                    if (prev.x - entry.x).abs() > RESERVE_MM
                        || (prev.y - entry.y).abs() > RESERVE_MM
                    {
                        motions.push(motion(
                            MotionPurpose::Clearance,
                            Interpolation::Rapid,
                            MotionEffect::None,
                            lifted,
                            Position {
                                z: clearance,
                                x: entry.x,
                                y: entry.y,
                            },
                            None,
                        ));
                    }
                    let air = entry_clear_of_stock(ctx, entry.x, entry.y, radius);
                    let descend_from = if air {
                        clearance
                    } else {
                        heights.top_z.max(cut_z) + 0.
                    };
                    if air {
                        // A wholly-outside-stock footprint enters air: the
                        // descent is not a plunge into material.
                        motions.push(motion(
                            MotionPurpose::Entry,
                            Interpolation::Rapid,
                            MotionEffect::None,
                            Position {
                                z: clearance,
                                x: entry.x,
                                y: entry.y,
                            },
                            entry,
                            None,
                        ));
                    } else {
                        if clearance > descend_from {
                            motions.push(motion(
                                MotionPurpose::Approach,
                                Interpolation::Rapid,
                                MotionEffect::None,
                                Position {
                                    z: clearance,
                                    x: entry.x,
                                    y: entry.y,
                                },
                                Position {
                                    z: descend_from,
                                    x: entry.x,
                                    y: entry.y,
                                },
                                None,
                            ));
                        }
                        motions.push(motion(
                            MotionPurpose::Entry,
                            Interpolation::LinearFeed,
                            MotionEffect::MillingSweep,
                            Position {
                                z: descend_from,
                                x: entry.x,
                                y: entry.y,
                            },
                            entry,
                            Some(plunge_feed),
                        ));
                    }
                }
                None => {
                    // First motion of the stage: assume clearance above the
                    // entry point; nothing about the prior position is claimed.
                    let air = entry_clear_of_stock(ctx, entry.x, entry.y, radius);
                    let start_z = clearance;
                    let mid_z = if air { cut_z } else { heights.top_z.max(cut_z) };
                    if start_z > mid_z {
                        motions.push(motion(
                            if air {
                                MotionPurpose::Entry
                            } else {
                                MotionPurpose::Approach
                            },
                            Interpolation::Rapid,
                            MotionEffect::None,
                            Position {
                                z: start_z,
                                x: entry.x,
                                y: entry.y,
                            },
                            Position {
                                z: mid_z,
                                x: entry.x,
                                y: entry.y,
                            },
                            None,
                        ));
                    }
                    if mid_z > cut_z {
                        motions.push(motion(
                            MotionPurpose::Entry,
                            if air {
                                Interpolation::Rapid
                            } else {
                                Interpolation::LinearFeed
                            },
                            if air {
                                MotionEffect::None
                            } else {
                                MotionEffect::MillingSweep
                            },
                            Position {
                                z: mid_z,
                                x: entry.x,
                                y: entry.y,
                            },
                            entry,
                            if air { None } else { Some(plunge_feed) },
                        ));
                    }
                }
            }
            // The row cut itself.
            motions.push(motion(
                MotionPurpose::Rough,
                Interpolation::LinearFeed,
                MotionEffect::MillingSweep,
                entry,
                exit,
                Some(cutting_feed),
            ));
            previous_end = Some(exit);
        }
        // Retract after each layer, and remember where the tool now is: the
        // next layer starts from the clearance plane at this position, and the
        // travel from here to its first entry is a motion of its own.
        if let Some(prev) = previous_end {
            if prev.z < clearance - RESERVE_MM {
                motions.push(motion(
                    MotionPurpose::Clearance,
                    Interpolation::Rapid,
                    MotionEffect::None,
                    prev,
                    Position {
                        z: clearance,
                        x: prev.x,
                        y: prev.y,
                    },
                    None,
                ));
            }
            previous_end = Some(Position {
                z: clearance,
                x: prev.x,
                y: prev.y,
            });
        }
    }

    // Envelope containment of every generated path.
    let envelope_max_x = envelope.min_x_mm + envelope.width_mm;
    let envelope_max_y = envelope.min_y_mm + envelope.length_mm;
    for motion in &motions {
        for p in [motion.start, motion.end] {
            if p.x < envelope.min_x_mm - RESERVE_MM
                || p.x > envelope_max_x + RESERVE_MM
                || p.y < envelope.min_y_mm - RESERVE_MM
                || p.y > envelope_max_y + RESERVE_MM
            {
                return Ok(incomplete(
                    operation_id,
                    vec![issue(
                        "FACE_ENVELOPE_EXCEEDED",
                        format!(
                            "generated motion {} leaves the allowed envelope; the path cannot authorize itself",
                            motion.id
                        ),
                        operation_id,
                    )],
                ));
            }
        }
    }
    let intent = ProcessIntent {
        spindle: ProcessSpindle::Milling {
            rpm,
            direction: settings.assignment.spindle_direction,
        },
        coolant: CoolantIntent::UseMachineProfile,
        path_control: PathControlIntent::UseMachineProfile,
    };
    let motion_count = motions.len();
    Ok(PlannedOperation {
        named_outputs: if motion_count == 0 {
            vec![]
        } else {
            vec![crate::sequence::NamedOutput {
                kind: "face_plane".into(),
                source_operation_id: Some(operation_id.into()),
                z_mm: Some(*layers.last().expect("layers are nonempty")),
                covered: Some(coverage),
                tab_placements: vec![],
            }]
        },
        status: if motion_count == 0 {
            GenerationStatus::Empty
        } else {
            GenerationStatus::Complete
        },
        stages: if motion_count == 0 {
            vec![]
        } else {
            vec![LocalStage {
                stage_id: stage,
                role: StageRole::Face,
                tool_id: settings.assignment.tool_id.clone(),
                motion_range: (0, motion_count),
                intent,
            }]
        },
        motions,
        stage_evidence: vec![],
        pass_evidence: vec![],
        issues: vec![],
        preparation: vec![],
    })
}
