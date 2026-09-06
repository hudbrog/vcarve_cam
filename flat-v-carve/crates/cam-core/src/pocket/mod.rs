//! Conservative endmill-only clearing. Every saved report is derived from actual motions.
mod settings;
mod verify;
use crate::{
    geometry::{BooleanOp, Diagnostic, Point, Region, Result},
    job::Job,
    model::{Depth, Length},
    motion::{Motion, MotionKind, Position},
    stock::{SliceRemoval, removal_at_slice},
    target::CenterSetStatus,
};
use serde::{Deserialize, Serialize};
pub use settings::{ClearingStrategy, EndmillPlanningSettings, EntryStrategy};
use settings::{Context, error};
pub use verify::verify_endmill_motions;

pub const PLAN_SCHEMA_VERSION: u32 = 1;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Complete,
    Empty,
    Incomplete,
    Inconclusive,
}
#[derive(Clone, Debug, Serialize)]
pub struct LayerReport {
    pub depth_mm: f64,
    pub requested_centers: Region,
    pub nominal_section: Region,
    pub accessible_floor: Region,
    pub removal: SliceRemoval,
    pub remaining_target: Region,
    pub missing_floor_beyond_tolerance: Region,
    pub possible_overcut: Region,
    pub status: PlanStatus,
    pub diagnostics: Vec<Diagnostic>,
}
#[derive(Clone, Debug, Serialize)]
pub struct EndmillAnalysis {
    pub status: PlanStatus,
    pub layers: Vec<LayerReport>,
    pub checked_motion_count: usize,
    pub minimum_center_margin_mm: Option<f64>,
    pub diagnostics: Vec<Diagnostic>,
    pub meaning: String,
    pub limitations: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenerationIssue {
    pub code: String,
    pub message: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct EndmillPlan {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub engine_version: String,
    pub input_fingerprint: String,
    pub motion_fingerprint: String,
    pub job: Job,
    pub spindle_rpm: f64,
    pub motions: Vec<Motion>,
    pub generation_issues: Vec<GenerationIssue>,
    // Derived geometry is rebuilt on load and belongs in inspection reports.
    #[serde(skip_serializing)]
    pub analysis: EndmillAnalysis,
}
#[derive(Deserialize)]
pub(crate) struct Envelope {
    artifact_kind: String,
    schema_version: u32,
    engine_version: String,
    pub(crate) input_fingerprint: String,
    pub(crate) motion_fingerprint: String,
    pub(crate) job: Job,
    pub(crate) motions: Vec<Motion>,
    generation_issues: Vec<GenerationIssue>,
}

fn hash<T: Serialize>(value: &T) -> Result<String> {
    crate::plan_hash::hash(value).map_err(|e| error("PLAN_JSON", e.to_string()))
}
fn input_hash(job: &Job) -> Result<String> {
    hash(&(
        env!("CARGO_PKG_VERSION"),
        "endmill-v1;clipper2-rust=1.1.0",
        job,
    ))
}
impl EndmillPlan {
    pub(crate) fn envelope(&self) -> Envelope {
        Envelope {
            artifact_kind: self.artifact_kind.clone(),
            schema_version: self.schema_version,
            engine_version: self.engine_version.clone(),
            input_fingerprint: self.input_fingerprint.clone(),
            motion_fingerprint: self.motion_fingerprint.clone(),
            job: self.job.clone(),
            motions: self.motions.clone(),
            generation_issues: self.generation_issues.clone(),
        }
    }
    pub(crate) fn revalidated(&self) -> Result<Self> {
        Self::from_envelope(self.envelope())
    }
    pub fn to_json(&self) -> Result<String> {
        let json = serde_json::to_string(self)
            .map(|s| s + "\n")
            .map_err(|e| error("PLAN_JSON", e.to_string()))?;
        if json.len() > 128_000_000 {
            return Err(error(
                "PLAN_RESOURCE_LIMIT",
                "plan exceeds the 128 MB reload limit",
            ));
        }
        Ok(json)
    }
    /// Recompute geometry, clearance and stock; serialized analysis is never trusted.
    pub fn from_json(json: &str) -> Result<Self> {
        if json.len() > 128_000_000 {
            return Err(error(
                "PLAN_RESOURCE_LIMIT",
                "plan exceeds the 128 MB limit",
            ));
        }
        let e: Envelope =
            serde_json::from_str(json).map_err(|e| error("PLAN_JSON", e.to_string()))?;
        Self::from_envelope(e)
    }

    /// Stream a plan from caller-owned storage, authenticating and rebuilding
    /// all analysis. Callers must bound untrusted input before using this API.
    pub fn from_reader(reader: impl std::io::Read) -> Result<Self> {
        let e = serde_json::from_reader(reader).map_err(|e| error("PLAN_JSON", e.to_string()))?;
        Self::from_envelope(e)
    }

    pub(crate) fn from_envelope(e: Envelope) -> Result<Self> {
        e.check_identity()?;
        let ctx = Context::new(&e.job)?;
        let mut analysis = verify::analyze(&ctx, &e.job, &e.motions)?;
        apply_generation(&mut analysis, &e.generation_issues);
        Ok(Self {
            artifact_kind: e.artifact_kind,
            schema_version: e.schema_version,
            engine_version: e.engine_version,
            input_fingerprint: e.input_fingerprint,
            motion_fingerprint: e.motion_fingerprint,
            job: e.job,
            spindle_rpm: ctx.spindle,
            motions: e.motions,
            generation_issues: e.generation_issues,
            analysis,
        })
    }
}

impl Envelope {
    /// Identity checks for a service-retained plan whose planning evidence was
    /// already established. This does not reconstruct stock or sampled reports.
    pub(crate) fn check_identity(&self) -> Result<()> {
        if self.artifact_kind != "endmill_plan"
            || self.schema_version != PLAN_SCHEMA_VERSION
            || self.engine_version != env!("CARGO_PKG_VERSION")
        {
            return Err(error(
                "PLAN_VERSION",
                "unsupported plan schema or engine version; regenerate the plan",
            ));
        }
        if input_hash(&self.job)? != self.input_fingerprint
            || hash(&(
                &self.input_fingerprint,
                &self.motions,
                &self.generation_issues,
            ))? != self.motion_fingerprint
        {
            return Err(error(
                "STALE_PLAN",
                "job settings or motions changed; regenerate the plan",
            ));
        }
        Ok(())
    }
}

fn depths(ctx: &Context) -> Result<Vec<f64>> {
    let n = (ctx.target.depth_cap().mm() / ctx.stepdown).ceil();
    if n > ctx.settings.max_layers as f64 {
        return Err(error(
            "PLANNING_RESOURCE_LIMIT",
            "requested depth needs more than max_layers; no plan was generated",
        ));
    }
    Ok((1..=n as usize)
        .map(|i| (i as f64 * ctx.stepdown).min(ctx.target.depth_cap().mm()))
        .collect())
}
fn strategy_depth(ctx: &Context, depth: f64) -> f64 {
    match ctx.settings.strategy {
        ClearingStrategy::DepthDependent => depth,
        ClearingStrategy::DeepestRegion => ctx.target.depth_cap().mm(),
    }
}

pub fn plan_endmill(job: &Job) -> Result<EndmillPlan> {
    let mut timing = crate::timing::Timer::new("endmill");
    let ctx = Context::new(job)?;
    timing.lap("context");
    let levels = depths(&ctx)?;
    let mut motions = vec![];
    let mut diagnostics = vec![];
    let mut resource_hit = false;
    for (layer, &depth) in levels.iter().enumerate() {
        let planning_depth = strategy_depth(&ctx, depth);
        let access = ctx.target.endmill_centers(
            &ctx.mill,
            Depth::new(planning_depth)?,
            Length::new(ctx.allowance)?,
        )?;
        if access.status == CenterSetStatus::Empty {
            continue;
        }
        if !ctx.entry_supported(job) {
            diagnostics.push(error(
                "UNSUPPORTED_ENTRY",
                format!("layer {layer}: selected entry is not explicitly supported by this tool"),
            ));
            continue;
        }
        if access.status == CenterSetStatus::Unresolved {
            diagnostics.push(error(
                "CENTER_SET_UNRESOLVED",
                format!("layer {layer}: refine geometry before planning"),
            ));
            continue;
        }
        if !access.contact_points.is_empty() || !access.contact_segments.is_empty() {
            diagnostics.push(error("ZERO_MARGIN_ACCESS",format!("layer {layer}: exact-fit contacts retained in geometry require a machining margin; positive-area regions may still be cleared")));
        }
        let centers = ctx
            .target
            .region()
            .erode(ctx.required_clearance(planning_depth) + ctx.guard)?;
        if centers.rings().is_empty() {
            diagnostics.push(error(
                "NO_NUMERICAL_MARGIN",
                format!("layer {layer}: access has no area after reserving the geometry margin"),
            ));
            continue;
        }
        let mut loop_count = 0;
        let mut loops = vec![];
        let mut cut_count = 0;
        for offset in 0..ctx.settings.max_loops_per_layer {
            let inset = centers.erode(offset as f64 * ctx.stepover)?;
            if inset.rings().is_empty() {
                break;
            }
            for mut points in inset.rings_mm() {
                if loop_count >= ctx.settings.max_loops_per_layer || resource_hit {
                    resource_hit = true;
                    break;
                }
                loop_count += 1;
                // Start on the longest edge to give a ramp the largest available traverse.
                let start = (0..points.len())
                    .max_by(|&a, &b| {
                        points[a]
                            .distance(points[(a + 1) % points.len()])
                            .total_cmp(&points[b].distance(points[(b + 1) % points.len()]))
                    })
                    .unwrap();
                points.rotate_left(start);
                if points
                    .iter()
                    .zip(points.iter().cycle().skip(1))
                    .any(|(&a, &b)| verify::center_margin(&ctx, a, b, depth).is_err())
                {
                    diagnostics.push(error(
                        "LOOP_CLEARANCE",
                        format!(
                            "layer {layer}: an offset segment could not establish cutter clearance"
                        ),
                    ));
                    continue;
                }
                cut_count += points.len();
                loops.push(points);
                // Bound the collected routing batch even when a few offsets
                // already contain more cuts than the entire motion budget.
                if cut_count >= ctx.settings.max_motions {
                    resource_hit = true;
                    break;
                }
            }
            if resource_hit {
                break;
            }
            if offset + 1 == ctx.settings.max_loops_per_layer {
                resource_hit = true;
            }
        }
        let mut entries = vec![];
        for (i, points) in loops.iter().enumerate() {
            let count = if matches!(ctx.settings.entry, EntryStrategy::Plunge) {
                points.len()
            } else {
                1
            };
            for (v, &p) in points.iter().take(count).enumerate() {
                let end = match ctx.settings.entry {
                    EntryStrategy::Ramp { max_angle_deg, .. } => {
                        let drop = (points[0].distance(points[1])
                            * max_angle_deg.to_radians().tan())
                        .min(ctx.stepdown / 2.);
                        if (depth / drop).ceil() % 2. == 1. {
                            points[1]
                        } else {
                            p
                        }
                    }
                    EntryStrategy::Plunge => p,
                };
                entries.push((i, v, p, end));
            }
        }
        let start = motions
            .last()
            .map_or(ctx.settings.start_xy_mm, |m: &Motion| m.end.xy());
        for (i, vertex) in crate::routing::nearest_order(entries, loops.len(), start) {
            let mut points = std::mem::take(&mut loops[i]);
            points.rotate_left(vertex);
            let previous = motions.last().map_or(
                Position::new(ctx.settings.start_xy_mm, ctx.settings.clearance_z_mm),
                |m| m.end,
            );
            let linked = matches!(ctx.settings.entry, EntryStrategy::Plunge)
                && depth <= ctx.stepdown
                && motions.last().is_some_and(|m| {
                    m.kind == MotionKind::RapidRetract
                        && m.layer == layer
                        && m.start.xy().distance(points[0]) <= ctx.stepover
                        && verify::center_margin(&ctx, m.start.xy(), points[0], depth)
                            .is_ok_and(|margin| margin >= ctx.guard / 2.)
                });
            let base = motions.len() - usize::from(linked);
            let previous = if linked {
                motions.last().unwrap().start
            } else {
                previous
            };
            match make_loop(&ctx, layer, depth, points, previous, base) {
                Ok(additions) if base + additions.len() <= ctx.settings.max_motions => {
                    if linked {
                        motions.pop();
                    }
                    motions.extend(additions);
                }
                Ok(_) => {
                    resource_hit = true;
                    break;
                }
                Err(d) => {
                    diagnostics.push(d);
                    resource_hit = true;
                    break;
                }
            }
        }
        if resource_hit {
            break;
        }
    }
    if resource_hit {
        diagnostics.push(error(
            "PLANNING_RESOURCE_LIMIT",
            "loop/motion budget was exhausted; retained motions are a partial plan",
        ));
    }
    timing.lap("generate motions");
    let mut analysis = verify::analyze(&ctx, job, &motions)?;
    timing.lap("analyze");
    let generation_issues: Vec<_> = diagnostics
        .into_iter()
        .map(|d| GenerationIssue {
            code: d.code,
            message: d.message,
        })
        .collect();
    apply_generation(&mut analysis, &generation_issues);
    let input_fingerprint = input_hash(job)?;
    let motion_fingerprint = hash(&(&input_fingerprint, &motions, &generation_issues))?;
    Ok(EndmillPlan {
        artifact_kind: "endmill_plan".into(),
        schema_version: PLAN_SCHEMA_VERSION,
        engine_version: env!("CARGO_PKG_VERSION").into(),
        input_fingerprint,
        motion_fingerprint,
        job: job.clone(),
        spindle_rpm: ctx.spindle,
        motions,
        generation_issues,
        analysis,
    })
}

fn apply_generation(analysis: &mut EndmillAnalysis, issues: &[GenerationIssue]) {
    if issues.is_empty() {
        return;
    }
    let uncertain = issues
        .iter()
        .any(|d| d.code != "UNSUPPORTED_ENTRY" && d.code != "LOOP_CLEARANCE");
    if analysis.status != PlanStatus::Inconclusive {
        analysis.status = if uncertain {
            PlanStatus::Inconclusive
        } else {
            PlanStatus::Incomplete
        };
    }
    analysis
        .diagnostics
        .extend(issues.iter().map(|d| error(&d.code, &d.message)));
}

fn make_loop(
    ctx: &Context,
    layer: usize,
    depth: f64,
    mut points: Vec<Point>,
    previous: Position,
    base_id: usize,
) -> Result<Vec<Motion>> {
    let mut moves = vec![];
    let mut position = previous;
    let mut push = |kind: MotionKind, end: Position, feed: Option<f64>| {
        if position != end {
            moves.push(Motion {
                id: base_id + moves.len(),
                tool_id: ctx.tool_id.clone(),
                operation_id: ctx.operation_id.clone(),
                layer,
                kind,
                start: position,
                end,
                feed_mm_min: feed,
            });
            position = end;
        }
    };
    if previous.z < 0. {
        push(
            MotionKind::Cut,
            Position::new(points[0], -depth),
            Some(ctx.feed),
        );
    } else {
        push(
            MotionKind::RapidXY,
            Position::new(points[0], ctx.settings.clearance_z_mm),
            None,
        );
        push(
            MotionKind::Approach,
            Position::new(points[0], 0.),
            Some(ctx.entry_feed),
        );
        match ctx.settings.entry {
            EntryStrategy::Plunge => {
                let n = (depth / ctx.stepdown).ceil() as usize;
                for i in 1..=n {
                    push(
                        MotionKind::Plunge,
                        Position::new(points[0], -(i as f64 * ctx.stepdown).min(depth)),
                        Some(ctx.entry_feed),
                    );
                }
            }
            EntryStrategy::Ramp { max_angle_deg, .. } => {
                let drop = (points[0].distance(points[1]) * max_angle_deg.to_radians().tan())
                    .min(ctx.stepdown / 2.);
                let count = (depth / drop).ceil();
                if !count.is_finite() || count > ctx.settings.max_motions as f64 {
                    return Err(error(
                        "ENTRY_RESOURCE_LIMIT",
                        "ramp requires too many traverses; use a larger entry region or an appropriate tool",
                    ));
                }
                for i in 1..=count as usize {
                    push(
                        MotionKind::Ramp,
                        Position::new(points[i % 2], -(i as f64 * drop).min(depth)),
                        Some(ctx.entry_feed),
                    );
                }
                if count as usize % 2 == 1 {
                    points.rotate_left(1);
                }
            }
        }
    }
    for &p in points.iter().skip(1).chain(points.first()) {
        push(MotionKind::Cut, Position::new(p, -depth), Some(ctx.feed));
    }
    push(
        MotionKind::RapidRetract,
        Position::new(points[0], ctx.settings.clearance_z_mm),
        None,
    );
    Ok(moves)
}

fn layer_stock(ctx: &Context, depth: f64, motions: &[Motion]) -> Result<LayerReport> {
    let eval_depth = strategy_depth(ctx, depth);
    let access = ctx.target.endmill_centers(
        &ctx.mill,
        Depth::new(eval_depth)?,
        Length::new(ctx.allowance)?,
    )?;
    let nominal = ctx.target.section_area(Depth::new(depth)?)?;
    let accessible = access.area.dilate(ctx.mill.radius().mm())?;
    let removal = removal_at_slice(
        ctx.target.region().grid(),
        motions,
        ctx.mill.radius().mm(),
        depth,
    )?;
    let missing = accessible
        .erode(ctx.coverage_tolerance)?
        .boolean(BooleanOp::Difference, &removal.lower)?;
    let remaining = nominal.boolean(BooleanOp::Difference, &removal.lower)?;
    let overcut = removal.upper.boolean(BooleanOp::Difference, &nominal)?;
    let mut diagnostics = vec![];
    if access.status == CenterSetStatus::Unresolved {
        diagnostics.push(error(
            "CENTER_SET_UNRESOLVED",
            "admissible center geometry is unresolved",
        ));
    }
    if !access.contact_points.is_empty() || !access.contact_segments.is_empty() {
        diagnostics.push(error(
            "ZERO_MARGIN_ACCESS",
            "zero-margin center contacts have no certified machining allowance",
        ));
    }
    if !overcut.rings().is_empty() {
        diagnostics.push(error(
            "SWEEP_OVERCUT_UNCERTAINTY",
            "outer sweep polygons extend outside the nominal section; refine the comparison",
        ));
    }
    let uncertain = !diagnostics.is_empty();
    if !missing.rings().is_empty() {
        diagnostics.push(error(
            "INCOMPLETE_FLOOR_COVERAGE",
            "actual lower-bound sweeps leave requested floor beyond the XY coverage tolerance",
        ));
    }
    let status = if uncertain {
        PlanStatus::Inconclusive
    } else if !missing.rings().is_empty() {
        PlanStatus::Incomplete
    } else if access.status == CenterSetStatus::Empty {
        PlanStatus::Empty
    } else {
        PlanStatus::Complete
    };
    Ok(LayerReport {
        depth_mm: depth,
        requested_centers: access.area,
        nominal_section: nominal,
        accessible_floor: accessible,
        removal,
        remaining_target: remaining,
        missing_floor_beyond_tolerance: missing,
        possible_overcut: overcut,
        status,
        diagnostics,
    })
}
