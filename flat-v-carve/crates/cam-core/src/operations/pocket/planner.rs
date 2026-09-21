use super::{
    missing_fields,
    verify::{self, error},
};
use crate::{
    geometry::{Point, Region, Result},
    motion::Position,
    operations::{PlanContext, PublishedFaceMap},
    project::{
        CutDirection, HeightReference, SpindleDirection, ToolGeometry,
        v5::{PocketEntry, PocketSettingsV5, resolve::ResolvedVcarveRegion},
    },
    sequence::{
        CoolantIntent, GenerationStatus, LocalStage, PathControlIntent, PlanIssue,
        PlannedOperation, ProcessIntent, ProcessSpindle, StageRole,
    },
    toolpath::{Interpolation, MotionEffect, MotionPurpose, PlannedMotion},
};

pub(crate) fn plan(
    ctx: &PlanContext,
    id: &str,
    s: &PocketSettingsV5,
    resolved: &ResolvedVcarveRegion,
    faces: &PublishedFaceMap,
) -> Result<PlannedOperation> {
    let missing = missing_fields(ctx, id, s);
    if let Some(d) = missing.first() {
        return Ok(crate::project::v5::resolve::incomplete_with(
            id,
            d.diagnostic(),
        ));
    }
    match generate(ctx, id, s, resolved, faces) {
        Ok(plan) => Ok(plan),
        Err(d) => Ok(crate::project::v5::resolve::incomplete_with(id, d)),
    }
}

pub(super) struct Builder<'a> {
    pub id: &'a str,
    pub settings: &'a PocketSettingsV5,
    pub motions: Vec<PlannedMotion>,
    pub cursor: Position,
    pub stage: String,
    pub pocket: String,
    pub layer: usize,
}
impl Builder<'_> {
    pub fn push(
        &mut self,
        to: Position,
        interpolation: Interpolation,
        purpose: MotionPurpose,
        feed: Option<f64>,
    ) -> Result<()> {
        if self.motions.len() >= self.settings.limits.max_motions {
            return Err(error(
                "PLANNING_RESOURCE_LIMIT",
                "pocket motion budget exhausted",
            ));
        }
        if self.cursor == to {
            return Ok(());
        }
        self.motions.push(PlannedMotion {
            id: self.motions.len(),
            operation_id: self.id.into(),
            stage_id: self.stage.clone(),
            tool_id: self.settings.assignment.tool_id.clone(),
            contour_id: Some(self.pocket.clone()),
            pass_id: self.layer,
            layer: self.layer,
            interpolation,
            purpose,
            effect: if matches!(interpolation, Interpolation::Rapid) {
                MotionEffect::None
            } else {
                MotionEffect::MillingSweep
            },
            start: self.cursor,
            end: to,
            feed_mm_min: feed,
            blade_heading_deg: None,
        });
        self.cursor = to;
        Ok(())
    }
    pub fn rapid(&mut self, to: Position) -> Result<()> {
        self.push(to, Interpolation::Rapid, MotionPurpose::Clearance, None)
    }
    pub fn feed(&mut self, to: Position, purpose: MotionPurpose, feed: f64) -> Result<()> {
        self.push(to, Interpolation::LinearFeed, purpose, Some(feed))
    }
}

fn generate(
    ctx: &PlanContext,
    id: &str,
    s: &PocketSettingsV5,
    resolved: &ResolvedVcarveRegion,
    faces: &PublishedFaceMap,
) -> Result<PlannedOperation> {
    let tool = ctx
        .tool(&s.assignment.tool_id)
        .ok_or_else(|| error("PROJECT_TOOL_REFERENCE", "pocket tool missing"))?;
    let Some(ToolGeometry::Endmill(spec)) = tool.geometry else {
        return Err(error("PROJECT_TOOL_KIND", "Pocket requires a flat endmill"));
    };
    let radius = spec.diameter_mm / 2.;
    let planes = faces.iter().map(|(id, f)| (id.clone(), f.z_mm)).collect();
    let heights = crate::setup::resolve_heights_values(
        ctx.setup.stock.thickness_mm,
        &s.top,
        &s.bottom,
        &planes,
    )?;
    let (top, bottom) = (heights.top_z, heights.bottom_z);
    let thickness = ctx.setup.stock.thickness_mm.unwrap();
    if top > 0. || bottom < -thickness || top <= bottom {
        return Err(error(
            "POCKET_DEPTH",
            "Pocket needs top ≤ stock top, bottom ≥ stock bottom and positive depth",
        ));
    }
    if -bottom > spec.cutting_length_mm {
        return Err(error(
            "ENDMILL_CUTTING_LENGTH",
            "Pocket depth from original stock exceeds cutting length",
        ));
    }
    let stock = ctx.setup.stock.xy.unwrap();
    let bounds = resolved
        .bounds
        .ok_or_else(|| error("EMPTY_SELECTION", "Pocket selection is empty"))?;
    let covers = |rect: crate::project::RectXY| {
        bounds.min.x >= rect.min_x_mm
            && bounds.min.y >= rect.min_y_mm
            && bounds.max.x <= rect.min_x_mm + rect.width_mm
            && bounds.max.y <= rect.min_y_mm + rect.length_mm
    };
    if !covers(stock) {
        return Err(error(
            "POCKET_OUTSIDE_STOCK",
            "Pocket lies outside stock XY",
        ));
    }
    if let HeightReference::FaceResult { operation_id } = &s.top.reference
        && !faces.get(operation_id).is_some_and(|f| covers(f.covered))
    {
        return Err(error(
            "POCKET_FACE_COVERAGE",
            "Referenced face does not cover every selected pocket",
        ));
    }
    let established = faces
        .values()
        .filter(|f| covers(f.covered))
        .fold(0_f64, |z, f| z.min(f.z_mm));
    if top < established - 1e-9 {
        return Err(error(
            "POCKET_TOP_BELOW_SURFACE",
            "Pocket top lies below the established material surface",
        ));
    }
    let e = resolved.region.grid().tolerance_mm();
    let guard = 4. * e;
    let tolerance = ctx.tolerances.verification_tolerance_mm.unwrap();
    if ctx.tolerances.motion_tolerance_mm.unwrap() < guard + resolved.source_error_mm
        || tolerance < 8. * e
    {
        return Err(error(
            "POCKET_PRECISION",
            "Refine source geometry to meet pocket motion/verification tolerances",
        ));
    }
    let stepdown = s.assignment.max_stepdown_mm.unwrap();
    let stepover = s.assignment.stepover_mm.unwrap();
    if stepover > radius || stepover <= guard {
        return Err(error(
            "STEPOVER_RANGE",
            "Pocket stepover must exceed four geometry tolerances and be at most the cutter radius",
        ));
    }
    let layers = ((top - bottom) / stepdown).ceil();
    if layers > s.limits.max_layers as f64 {
        return Err(error(
            "PLANNING_RESOURCE_LIMIT",
            "Pocket exceeds layer budget",
        ));
    }
    let supported = match s.entry {
        PocketEntry::Plunge => tool.capabilities.plunge_capable == Some(true),
        _ => tool.capabilities.ramp_capable == Some(true),
    };
    if !supported {
        return Err(error(
            "UNSUPPORTED_ENTRY",
            "Pocket entry requires explicit plunge/ramp capability",
        ));
    }
    let feed = s.assignment.cutting_feed_mm_min.unwrap();
    let clearance = ctx.setup.clearance_above_stock_mm.unwrap();
    let intent = ProcessIntent {
        spindle: ProcessSpindle::Milling {
            rpm: s.assignment.spindle_rpm.unwrap(),
            direction: s.assignment.spindle_direction,
        },
        coolant: CoolantIntent::UseMachineProfile,
        path_control: PathControlIntent::ExactPath,
    };
    let mut b = Builder {
        id,
        settings: s,
        motions: vec![],
        cursor: Position::new(ctx.setup.start_xy_mm.unwrap(), clearance),
        stage: String::new(),
        pocket: String::new(),
        layer: 0,
    };
    let mut stages = vec![];
    let mut issues = vec![];
    for (index, region) in resolved.region.components().into_iter().enumerate() {
        b.pocket = format!("{id}-pocket-{}", index + 1);
        let rough_centers = region.erode(radius + s.wall_allowance_mm.unwrap() + guard)?;
        if rough_centers.rings().is_empty() {
            return Err(error(
                "POCKET_NO_ACCESS",
                format!("{} has no positive-area cutter access", b.pocket),
            ));
        }
        for finish in [false, true] {
            if finish && !s.finish_walls {
                continue;
            }
            let centers = if finish {
                region.erode(radius + guard)?
            } else {
                rough_centers.clone()
            };
            let loops = loops(&centers, stepover, s.limits.max_loops_per_layer, finish)?;
            b.stage = format!("{}-{}", b.pocket, if finish { "finish" } else { "rough" });
            let first = b.motions.len();
            for layer in 1..=layers as usize {
                b.layer = layer;
                let z = (top - layer as f64 * stepdown).max(bottom);
                for points in &loops {
                    let mut points = points.clone();
                    // Clear offsets from the inside outward. Normalized outer
                    // loops are CCW and island loops CW: the remaining wall is
                    // on the right, as required for climb with a CW spindle.
                    let reverse = (s.direction == Some(CutDirection::Climb))
                        != (s
                            .assignment
                            .spindle_direction
                            .unwrap_or(SpindleDirection::Clockwise)
                            == SpindleDirection::Clockwise);
                    if reverse {
                        points.reverse();
                    }
                    super::entries::seam(&mut points);
                    super::entries::cut_run(
                        &mut b,
                        super::entries::Run {
                            region: &region,
                            centers: &centers,
                            points: &points,
                            radius,
                            clearance_radius: radius
                                + if finish {
                                    0.
                                } else {
                                    s.wall_allowance_mm.unwrap()
                                },
                            top,
                            z,
                            clearance_z: clearance,
                            feed: if finish {
                                s.finish_feed_mm_min.unwrap_or(feed)
                            } else {
                                feed
                            },
                            purpose: if finish {
                                MotionPurpose::Finish
                            } else {
                                MotionPurpose::Rough
                            },
                        },
                    )
                    .map_err(|mut diagnostic| {
                        diagnostic.message =
                            format!("{} layer {layer}: {}", b.pocket, diagnostic.message);
                        diagnostic
                    })?;
                }
                // Finishing follows roughing: its coverage uses both stages.
                let remaining = verify::coverage(
                    &region,
                    &centers,
                    radius,
                    z,
                    tolerance,
                    if finish {
                        &b.motions
                    } else {
                        &b.motions[first..]
                    },
                )?;
                if !remaining.rings().is_empty() {
                    return Err(error(
                        "POCKET_COVERAGE",
                        format!(
                            "{} layer {layer} leaves {:.6} mm² reachable material",
                            b.pocket,
                            remaining.area_mm2()
                        ),
                    ));
                }
            }
            verify::containment(
                &region,
                radius
                    + if finish {
                        0.
                    } else {
                        s.wall_allowance_mm.unwrap()
                    },
                top,
                bottom,
                &b.motions[first..],
            )?;
            stages.push(LocalStage {
                stage_id: b.stage.clone(),
                role: if finish {
                    StageRole::PocketFinish
                } else {
                    StageRole::PocketRough
                },
                tool_id: s.assignment.tool_id.clone(),
                motion_range: (first, b.motions.len()),
                intent: intent.clone(),
            });
        }
        let nominal_centers = region.erode(radius + guard)?;
        let inaccessible = region.boolean(
            crate::geometry::BooleanOp::Difference,
            &nominal_centers.dilate(radius + tolerance)?,
        )?;
        if !inaccessible.rings().is_empty() {
            issues.push(PlanIssue {
                code: "POCKET_UNREACHABLE_RESIDUAL".into(),
                message: format!(
                    "{}: {:.4} mm² outside cutter reach",
                    b.pocket,
                    inaccessible.area_mm2()
                ),
                operation_id: Some(id.into()),
                stage_id: None,
            });
        }
        if !s.finish_walls && s.wall_allowance_mm.unwrap() > 0. {
            issues.push(PlanIssue {
                code: "POCKET_WALL_ALLOWANCE".into(),
                message: format!(
                    "{}: {:.4} mm radial wall allowance intentionally retained",
                    b.pocket,
                    s.wall_allowance_mm.unwrap()
                ),
                operation_id: Some(id.into()),
                stage_id: None,
            });
        }
    }
    Ok(PlannedOperation {
        status: GenerationStatus::Complete,
        stages,
        motions: b.motions,
        named_outputs: vec![],
        stage_evidence: vec![],
        pass_evidence: vec![],
        issues,
        preparation: vec![],
    })
}

fn loops(centers: &Region, stepover: f64, limit: usize, finish: bool) -> Result<Vec<Vec<Point>>> {
    let mut result = vec![];
    for i in 0..limit {
        let inset = centers.erode(i as f64 * stepover)?;
        if inset.rings().is_empty() {
            // Each later offset cuts outward into the remaining shell. This
            // preserves the requested milling direction around both wall types.
            result.reverse();
            return Ok(result);
        }
        result.extend(inset.rings_mm());
        if result.len() > limit {
            break;
        }
        if finish {
            return Ok(result);
        }
    }
    Err(error(
        "PLANNING_RESOURCE_LIMIT",
        "Pocket offset-loop budget exhausted",
    ))
}
