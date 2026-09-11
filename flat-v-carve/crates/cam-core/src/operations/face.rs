//! Face planner (plan section 11): deterministic rectangular raster at 0 or
//! 90 degrees, depth layers ending exactly at the bottom, coverage-checked
//! against the requested rectangle, with an allowed envelope derived from
//! the settings rather than from whatever paths were generated.
use crate::{
    geometry::{Diagnostic, Result},
    motion::Position,
    operations::{LocatedDiagnostic, PlanContext},
    project::{FaceArea, FaceSettings, MillingAssignment, RectXY, ToolGeometry, WorkZeroXY},
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

/// Raster rows: scan positions spaced by the stepover, extended so the union
/// of cutter sweeps covers the rectangle including corners, plus the
/// requested travel overrun beyond coverage.
pub fn raster_rows(
    geometry: &FaceGeometry,
    stepover: f64,
    radius: f64,
    entry_overrun: f64,
    exit_overrun: f64,
) -> Vec<(f64, f64, f64)> {
    let span = geometry.scan_max - geometry.scan_min;
    let first = geometry.scan_min + radius;
    let last_target = geometry.scan_max - radius;
    if last_target <= first {
        // The cutter covers the whole cross extent in one row.
        return vec![(
            geometry.cross_min - radius - entry_overrun,
            geometry.cross_max + radius + exit_overrun,
            geometry.scan_min + span / 2.,
        )];
    }
    let count = ((last_target - first) / stepover).floor() as usize + 1;
    let mut rows = vec![];
    for index in 0..count {
        rows.push((
            geometry.cross_min - radius - entry_overrun,
            geometry.cross_max + radius + exit_overrun,
            first + index as f64 * stepover,
        ));
    }
    let last = rows.last().map(|r| r.2).unwrap_or(first);
    if last_target - last > RESERVE_MM {
        rows.push((
            geometry.cross_min - radius - entry_overrun,
            geometry.cross_max + radius + exit_overrun,
            last_target,
        ));
    }
    rows
}

/// Verify the swept bands cover the requested rectangle; report missed strips.
pub fn coverage_gaps(
    geometry: &FaceGeometry,
    rows: &[(f64, f64, f64)],
    radius: f64,
) -> Vec<(f64, f64)> {
    let mut gaps = vec![];
    let mut bands: Vec<(f64, f64)> = rows.iter().map(|r| (r.2 - radius, r.2 + radius)).collect();
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

fn footprint_wholly_outside_stock(ctx: &PlanContext, x: f64, y: f64, radius: f64) -> bool {
    match ctx.setup.stock.xy {
        Some(rect) => {
            let max_x = rect.min_x_mm + rect.width_mm;
            let max_y = rect.min_y_mm + rect.length_mm;
            // Distance from the circle center to the rectangle; wholly
            // outside when the closest rectangle point is beyond the radius.
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
            dx.hypot(dy) > radius
        }
        None => false,
    }
}

fn stage_id(operation_id: &str) -> String {
    format!("{operation_id}-face")
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
    let entry_overrun = settings.entry_overrun_mm.unwrap_or(0.);
    let exit_overrun = settings.exit_overrun_mm.unwrap_or(0.);
    let rows = raster_rows(&geometry, stepover, radius, entry_overrun, exit_overrun);
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
    // Allowed envelope: coverage expanded by the travel overruns, then
    // dilated by the cutter radius plus the numerical reserve. Generated
    // paths are checked against it below; a bug cannot widen the envelope.
    let envelope = RectXY {
        min_x_mm: coverage.min_x_mm - entry_overrun - radius - RESERVE_MM,
        min_y_mm: coverage.min_y_mm - entry_overrun - radius - RESERVE_MM,
        width_mm: coverage.width_mm
            + (entry_overrun + exit_overrun + 2. * radius + 2. * RESERVE_MM),
        length_mm: coverage.length_mm
            + (entry_overrun + exit_overrun + 2. * radius + 2. * RESERVE_MM),
    };
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
    let zigzag = settings.pattern == crate::project::FacePattern::ZigZag;

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
        for (row_index, &(cross_start, cross_end, scan)) in rows.iter().enumerate() {
            let reversed = zigzag && (layer_index + row_index) % 2 == 1;
            let (start_cross, end_cross) = if reversed {
                (cross_end, cross_start)
            } else {
                (cross_start, cross_end)
            };
            let start_xy = |c: f64| Position {
                x: if geometry.along_y { scan } else { c },
                y: if geometry.along_y { c } else { scan },
                z: cut_z,
            };
            let entry = start_xy(start_cross);
            let exit = start_xy(end_cross);
            // Entry: cross-row link (zigzag), or retract/rapid/descend.
            match previous_end {
                Some(prev) if zigzag && (prev.y != entry.y || prev.x != entry.x) => {
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
                    motions.push(motion(
                        MotionPurpose::Clearance,
                        Interpolation::Rapid,
                        MotionEffect::None,
                        Position {
                            z: clearance,
                            x: prev.x,
                            y: prev.y,
                        },
                        Position {
                            z: clearance,
                            x: entry.x,
                            y: entry.y,
                        },
                        None,
                    ));
                    let air = footprint_wholly_outside_stock(ctx, entry.x, entry.y, radius);
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
                    let air = footprint_wholly_outside_stock(ctx, entry.x, entry.y, radius);
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
        // Retract after each layer.
        if let Some(prev) = previous_end {
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
            previous_end = None;
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
