//! Drill planner: positional markers from the artwork catalogue resolved to
//! heights and a per-hole motion sequence — rapid travel at the job clearance
//! plane, rapid to the retract (R) plane, feeding plunges with optional
//! pecking and bottom dwell — expanded to explicit linear moves. No canned
//! cycles are emitted (plan section 9: MVP output is explicit G0/G1 moves).
//!
//! Peck semantics follow the classic controller cycles the research distilled
//! (FreeCAD Path Drilling, Fusion 360 Drill): a full-retract peck (G83
//! semantics) returns to the R plane and rapids back to a small clearance
//! above the previously reached depth; a chip-break peck (G73 semantics)
//! retracts only a short distance inside the hole and feeds back down through
//! it. Both are deterministic and both survive the byte-for-byte plan replay.
use crate::{
    geometry::{Diagnostic, Result},
    model::Drill,
    motion::Position,
    operations::{LocatedDiagnostic, PlanContext, PlannerGeometry},
    project::{DrillHoleOrder, DrillPeckMode, DrillSettings, ToolGeometry},
    sequence::{
        CoolantIntent, GenerationStatus, LocalStage, PathControlIntent, PlanIssue,
        PlannedOperation, ProcessIntent, ProcessSpindle, StageRole,
    },
    setup::{resolve_heights_values, resolve_top_values},
    toolpath::{Interpolation, MotionEffect, MotionPurpose, PlannedMotion},
};
use std::collections::BTreeMap;

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("drill")
}

/// Numerical reserve for height comparisons.
const RESERVE_MM: f64 = 1e-6;
/// How far above the previously reached depth a full-retract peck rapids
/// back down before feeding again (LinuxCNC G83 semantics use 0.254 mm).
const FULL_RETRACT_REENTRY_MM: f64 = 0.254;
/// Default chip-break in-hole retract when the settings leave it unset
/// (LinuxCNC G73's fixed retract is 0.254 mm).
const CHIP_BREAK_RETRACT_MM: f64 = 0.254;
/// Two selected markers closer than this are the same spot.
const COINCIDENT_POINT_MM: f64 = 1e-3;

fn issue(code: &str, message: impl Into<String>, operation_id: &str) -> PlanIssue {
    PlanIssue {
        code: code.into(),
        message: message.into(),
        operation_id: Some(operation_id.into()),
        stage_id: None,
    }
}

fn incomplete(_operation_id: &str, issues: Vec<PlanIssue>) -> PlannedOperation {
    PlannedOperation {
        status: GenerationStatus::Incomplete,
        stages: vec![],
        motions: vec![],
        named_outputs: vec![],
        stage_evidence: vec![],
        pass_evidence: vec![],
        issues,
        preparation: vec![],
    }
}

/// Required-but-unset fields for planning this drill operation.
pub fn missing_fields(
    job: &crate::project::CamJob,
    operation_id: &str,
    settings: &DrillSettings,
) -> Vec<LocatedDiagnostic> {
    missing_fields_ctx(&PlanContext::from_v4(job), operation_id, settings)
}

pub(crate) fn missing_fields_ctx(
    ctx: &PlanContext,
    operation_id: &str,
    settings: &DrillSettings,
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
            "the clearance above stock",
        );
    }
    if !ctx.has_artwork {
        push(
            "operations.drill.points".into(),
            "artwork to select drill points from",
        );
    } else if settings.points.is_empty() {
        push("operations.drill.points".into(), "at least one drill point");
    }
    let tool = ctx.tool(&settings.assignment.tool_id);
    if tool.is_none() {
        push(
            format!("operations[{operation_id}].assignment.tool_id"),
            "a drill tool",
        );
        return missing;
    }
    let tool = tool.expect("checked");
    match tool.geometry {
        Some(ToolGeometry::Drill(_)) => {}
        Some(_) => return missing,
        None => push(
            format!("tools['{}'].geometry", settings.assignment.tool_id),
            "drill geometry",
        ),
    }
    if settings.assignment.spindle_rpm.is_none() {
        push(
            format!("operations[{operation_id}].assignment.spindle_rpm"),
            "the spindle speed",
        );
    }
    if settings.assignment.plunge_feed_mm_min.is_none() {
        push(
            format!("operations[{operation_id}].assignment.plunge_feed_mm_min"),
            "the drilling feed rate",
        );
    }
    missing
}

fn drill_of(ctx: &PlanContext, settings: &DrillSettings) -> Result<Drill> {
    let tool = ctx
        .tool(&settings.assignment.tool_id)
        .ok_or_else(|| error("PROJECT_TOOL_REFERENCE", "drill tool not found"))?;
    match &tool.geometry {
        Some(ToolGeometry::Drill(spec)) => Drill::try_from(spec.clone()),
        Some(other) => Err(error(
            "PROJECT_TOOL_KIND",
            format!(
                "drilling requires a drill tool, found {}",
                match other {
                    ToolGeometry::Endmill(_) => "endmill",
                    ToolGeometry::Vbit(_) => "vbit",
                    ToolGeometry::DragKnife(_) => "drag_knife",
                    ToolGeometry::Drill(_) => unreachable!(),
                }
            ),
        )),
        None => Err(error(
            "MISSING_MACHINING_SETTING",
            "drill tool geometry is required",
        )),
    }
}

/// Plan one drill operation: resolve every selected marker to a hole, order
/// them, and emit the plunge sequence per hole.
pub(crate) fn plan(
    ctx: &PlanContext,
    operation_id: &str,
    settings: &DrillSettings,
    published_faces: &BTreeMap<String, crate::operations::PublishedFace>,
    geometry: &PlannerGeometry,
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
    let drill = drill_of(ctx, settings)?;
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
        Err(diagnostic) => {
            return Ok(incomplete(
                operation_id,
                vec![issue(&diagnostic.code, diagnostic.message, operation_id)],
            ));
        }
    };
    let retract_z = match resolve_top_values(
        ctx.setup.stock.thickness_mm,
        &settings.retract_height,
        &published_planes,
    ) {
        Ok(z) => z,
        Err(diagnostic) => {
            return Ok(incomplete(
                operation_id,
                vec![issue(&diagnostic.code, diagnostic.message, operation_id)],
            ));
        }
    };
    let clearance = ctx.setup.clearance_above_stock_mm.expect("checked above");
    if retract_z <= heights.top_z + RESERVE_MM {
        return Ok(incomplete(
            operation_id,
            vec![issue(
                "DRILL_RETRACT_BELOW_TOP",
                format!(
                    "the resolved retract height ({retract_z:.4} mm) must lie above the resolved top ({:.4} mm): feeding would start inside the material",
                    heights.top_z
                ),
                operation_id,
            )],
        ));
    }
    if retract_z > clearance + RESERVE_MM {
        return Ok(incomplete(
            operation_id,
            vec![issue(
                "DRILL_RETRACT_ABOVE_CLEARANCE",
                format!(
                    "the resolved retract height ({retract_z:.4} mm) lies above the job clearance plane ({clearance:.4} mm); raise the clearance or lower the retract height"
                ),
                operation_id,
            )],
        ));
    }
    // Full-diameter bottoms over-drill the tip by its cone height so the
    // cutting lips, not the point, reach the resolved bottom.
    let tip_extra = settings.breakthrough_extra_mm.unwrap_or(0.)
        + if settings.depth_reference == crate::project::DrillDepthReference::FullDiameter {
            drill.tip_length().mm()
        } else {
            0.
        };
    let final_z = heights.bottom_z - tip_extra;
    let cut_depth = heights.top_z - final_z;
    if cut_depth > drill.cutting_length().mm() {
        return Ok(incomplete(
            operation_id,
            vec![issue(
                "DRILL_CUTTING_LENGTH",
                format!(
                    "the {:.4} mm hole depth exceeds the drill's {:.4} mm cutting length",
                    cut_depth,
                    drill.cutting_length().mm()
                ),
                operation_id,
            )],
        ));
    }

    // Resolve the selected markers through the catalogue.
    let catalogue = geometry.catalogue()?;
    let mut points = vec![];
    for id in &settings.points {
        let Some(point) = catalogue.point(id) else {
            return Ok(incomplete(
                operation_id,
                vec![issue(
                    "DRILL_POINT_REFERENCE",
                    format!("unknown drill point '{id}'; inspect the catalogue point entries"),
                    operation_id,
                )],
            ));
        };
        points.push((point.center, point.diameter_mm, id.clone()));
    }
    if settings.hole_order == DrillHoleOrder::XThenY {
        points.sort_by(|a, b| {
            (a.0.x, a.0.y)
                .partial_cmp(&(b.0.x, b.0.y))
                .expect("finite marker centers")
        });
    }
    // Holes outside the stock rectangle would cut air (or the fixture).
    if let Some(rect) = ctx.setup.stock.xy {
        for (center, _, id) in &points {
            if center.x < rect.min_x_mm - RESERVE_MM
                || center.y < rect.min_y_mm - RESERVE_MM
                || center.x > rect.min_x_mm + rect.width_mm + RESERVE_MM
                || center.y > rect.min_y_mm + rect.length_mm + RESERVE_MM
            {
                return Ok(incomplete(
                    operation_id,
                    vec![issue(
                        "DRILL_POINT_OUTSIDE_STOCK",
                        format!(
                            "drill point '{id}' at ({:.3}, {:.3}) lies outside the stock rectangle",
                            center.x, center.y
                        ),
                        operation_id,
                    )],
                ));
            }
        }
    }
    // Non-blocking advisories: coincident markers drill the same hole twice,
    // and (opt-in) a drill wider than the drawn marker circle.
    let mut advisories = vec![];
    for (index, (center, _, id)) in points.iter().enumerate() {
        for (other_center, _, other_id) in &points[index + 1..] {
            if center.distance(*other_center) < COINCIDENT_POINT_MM {
                advisories.push(issue(
                    "DRILL_POINTS_COINCIDENT",
                    format!(
                        "drill points '{id}' and '{other_id}' sit at the same spot; the hole is drilled more than once"
                    ),
                    operation_id,
                ));
            }
        }
    }
    if settings.warn_drill_exceeds_marker {
        let drill_diameter = 2. * drill.radius().mm();
        for (center, diameter, id) in &points {
            if drill_diameter > diameter + RESERVE_MM {
                advisories.push(issue(
                    "DRILL_EXCEEDS_MARKER",
                    format!(
                        "the Ø{drill_diameter:.3} drill is wider than marker '{id}' (Ø{diameter:.3}) at ({:.3}, {:.3}); the hole will be larger than drawn",
                        center.x, center.y
                    ),
                    operation_id,
                ));
            }
        }
    }

    let feed = settings
        .assignment
        .plunge_feed_mm_min
        .expect("checked above");
    let rpm = settings.assignment.spindle_rpm.expect("checked above");
    let stage = format!("{operation_id}-drill");
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

    let dwell = settings.dwell_at_bottom_s;
    let peck = settings.peck.as_ref().map(|peck| {
        let retract = match peck.mode {
            DrillPeckMode::ChipBreak => peck.retract_mm.unwrap_or(CHIP_BREAK_RETRACT_MM),
            DrillPeckMode::FullRetract => 0.,
        };
        (peck, retract)
    });
    let mut previous_end: Option<Position> = None;
    for (center, _, _) in &points {
        let at = |z: f64| Position::new(*center, z);
        // Travel: climb to the clearance plane, then across at clearance.
        // The cursor tracks where the machine really is after every travel
        // motion so the descent always starts above its own hole.
        if let Some(prev) = previous_end
            && prev.z < clearance - RESERVE_MM
        {
            let lifted = Position::new(prev.xy(), clearance);
            motions.push(motion(
                MotionPurpose::Clearance,
                Interpolation::Rapid,
                MotionEffect::None,
                prev,
                lifted,
                None,
            ));
            previous_end = Some(lifted);
        }
        if let Some(prev) = previous_end
            && ((prev.x - center.x).abs() > RESERVE_MM || (prev.y - center.y).abs() > RESERVE_MM)
        {
            let target = at(clearance);
            motions.push(motion(
                MotionPurpose::Clearance,
                Interpolation::Rapid,
                MotionEffect::None,
                prev,
                target,
                None,
            ));
            previous_end = Some(target);
        }
        let start = previous_end.unwrap_or_else(|| at(clearance));
        // Descend to the R plane, then feed.
        if clearance > retract_z + RESERVE_MM {
            motions.push(motion(
                MotionPurpose::Approach,
                Interpolation::Rapid,
                MotionEffect::None,
                start,
                at(retract_z),
                None,
            ));
        }
        match &peck {
            None => {
                motions.push(motion(
                    MotionPurpose::Entry,
                    Interpolation::LinearFeed,
                    MotionEffect::MillingSweep,
                    at(retract_z),
                    at(final_z),
                    Some(feed),
                ));
                if let Some(seconds) = dwell {
                    motions.push(motion(
                        MotionPurpose::Entry,
                        Interpolation::Dwell { seconds },
                        MotionEffect::None,
                        at(final_z),
                        at(final_z),
                        None,
                    ));
                }
                motions.push(motion(
                    MotionPurpose::Clearance,
                    Interpolation::Rapid,
                    MotionEffect::None,
                    at(final_z),
                    at(retract_z),
                    None,
                ));
            }
            Some((peck, chip_retract)) => {
                let levels = peck.schedule(cut_depth);
                let mut previous_level_z: Option<f64> = None;
                for (index, &reached) in levels.iter().enumerate() {
                    let level_z = heights.top_z - reached;
                    let last = index + 1 == levels.len();
                    // Where the feeding cut of this peck starts. The first
                    // peck always feeds from the R plane; later pecks feed
                    // from wherever their retraction left the tool.
                    let feed_from = match peck.mode {
                        DrillPeckMode::ChipBreak => previous_level_z.unwrap_or(retract_z),
                        // A full retraction rapids back to a small clearance
                        // above the previously reached depth before feeding.
                        DrillPeckMode::FullRetract => {
                            let reentry = previous_level_z.map_or(retract_z, |previous| {
                                (previous + FULL_RETRACT_REENTRY_MM).min(retract_z)
                            });
                            if reentry < retract_z - RESERVE_MM {
                                motions.push(motion(
                                    MotionPurpose::Approach,
                                    Interpolation::Rapid,
                                    MotionEffect::None,
                                    at(retract_z),
                                    at(reentry),
                                    None,
                                ));
                            }
                            reentry
                        }
                    };
                    motions.push(motion(
                        MotionPurpose::Entry,
                        Interpolation::LinearFeed,
                        MotionEffect::MillingSweep,
                        at(feed_from),
                        at(level_z),
                        Some(feed),
                    ));
                    if last && let Some(seconds) = dwell {
                        motions.push(motion(
                            MotionPurpose::Entry,
                            Interpolation::Dwell { seconds },
                            MotionEffect::None,
                            at(level_z),
                            at(level_z),
                            None,
                        ));
                    }
                    match peck.mode {
                        DrillPeckMode::ChipBreak if !last => {
                            let lifted = level_z + chip_retract;
                            motions.push(motion(
                                MotionPurpose::Clearance,
                                Interpolation::Rapid,
                                MotionEffect::None,
                                at(level_z),
                                at(lifted),
                                None,
                            ));
                            // The next chip-break peck feeds from the lifted
                            // in-hole position.
                            previous_level_z = Some(lifted);
                        }
                        _ => {
                            motions.push(motion(
                                MotionPurpose::Clearance,
                                Interpolation::Rapid,
                                MotionEffect::None,
                                at(level_z),
                                at(retract_z),
                                None,
                            ));
                            // A full-retract peck's re-entry clears a little
                            // above the depth this peck reached.
                            previous_level_z = Some(level_z);
                        }
                    }
                }
            }
        }
        previous_end = Some(at(retract_z));
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
        named_outputs: vec![],
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
                role: StageRole::Drill,
                tool_id: settings.assignment.tool_id.clone(),
                motion_range: (0, motion_count),
                intent,
            }]
        },
        motions,
        stage_evidence: vec![],
        pass_evidence: vec![],
        issues: advisories,
        preparation: vec![],
    })
}
