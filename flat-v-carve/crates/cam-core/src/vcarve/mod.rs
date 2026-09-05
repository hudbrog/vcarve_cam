//! Combined endmill / finite-tip V-bit planning. M4 reports slice and sampled
//! quality evidence; adaptive full-volume certification remains M5.
mod medial;
mod quality;
mod settings;
mod verify;
use crate::{
    geometry::{BooleanOp, Point, Region, Result},
    job::Job,
    model::Depth,
    motion::{Motion, MotionKind, Position},
    pocket::{EndmillPlan, GenerationIssue, PlanStatus, plan_endmill},
    stock::{removal_at_slice, vbit_air_in_endmill},
    svg::Bounds,
};
pub use medial::{MedialAxis, MedialBranch};
pub use quality::{CombinedAnalysis, CombinedSlice, QualitySample};
use serde::{Deserialize, Serialize};
pub use settings::VBitPlanningSettings;
use settings::{Context, error};
use sha2::{Digest, Sha256};
pub use verify::verify_vbit_motions;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathFamily {
    Floor,
    Boundary,
    Medial,
    Contact,
    Cleanup,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub family: PathFamily,
    pub points: Vec<Position>,
    pub source_branch: Option<usize>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Execution {
    pub candidate: Candidate,
    pub pass_depth_mm: f64,
    pub final_finish: bool,
    pub pruned_air: bool,
    pub first_motion_id: usize,
    pub end_motion_id: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageTransition {
    pub after_motion_count: usize,
    pub from_tool_id: String,
    pub to_tool_id: String,
    pub position: Position,
}
#[derive(Clone, Debug, Serialize)]
pub struct CombinedPlan {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub engine_version: String,
    pub input_fingerprint: String,
    pub motion_fingerprint: String,
    pub endmill: EndmillPlan,
    pub transition: StageTransition,
    pub vbit_spindle_rpm: f64,
    pub vbit_motions: Vec<Motion>,
    pub executions: Vec<Execution>,
    pub generation_issues: Vec<GenerationIssue>,
    pub analysis: CombinedAnalysis,
}
#[derive(Deserialize)]
struct Envelope {
    artifact_kind: String,
    schema_version: u32,
    engine_version: String,
    input_fingerprint: String,
    motion_fingerprint: String,
    endmill: serde_json::Value,
    transition: StageTransition,
    vbit_motions: Vec<Motion>,
    executions: Vec<Execution>,
    generation_issues: Vec<GenerationIssue>,
}
fn hash<T: Serialize>(v: &T) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(v).map_err(|e| error("PLAN_JSON", e.to_string()))?)
    ))
}
fn identity(endmill: &EndmillPlan) -> Result<String> {
    hash(&(
        env!("CARGO_PKG_VERSION"),
        "combined-v1;clipper2-rust=1.1.0;boostvoronoi=0.12.1",
        &endmill.input_fingerprint,
    ))
}
impl CombinedPlan {
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map(|s| s + "\n")
            .map_err(|e| error("PLAN_JSON", e.to_string()))
    }
    pub fn from_json(json: &str) -> Result<Self> {
        if json.len() > 128_000_000 {
            return Err(error("PLAN_RESOURCE_LIMIT", "combined plan exceeds 128 MB"));
        }
        let e: Envelope =
            serde_json::from_str(json).map_err(|e| error("PLAN_JSON", e.to_string()))?;
        if e.artifact_kind != "combined_plan"
            || e.schema_version != 1
            || e.engine_version != env!("CARGO_PKG_VERSION")
        {
            return Err(error(
                "PLAN_VERSION",
                "unsupported combined schema or engine; regenerate the plan",
            ));
        }
        let endmill = EndmillPlan::from_json(&e.endmill.to_string())?;
        if e.input_fingerprint != identity(&endmill)?
            || e.motion_fingerprint
                != hash(&(
                    &e.input_fingerprint,
                    &endmill.motion_fingerprint,
                    &e.transition,
                    &e.vbit_motions,
                    &e.executions,
                    &e.generation_issues,
                ))?
        {
            return Err(error(
                "STALE_PLAN",
                "combined settings or motions changed; regenerate the plan",
            ));
        }
        let ctx = Context::new(&endmill.job)?;
        let (axis, candidates) = candidates(&ctx, &endmill)?;
        verify::executions(
            &ctx,
            &endmill,
            &e.transition,
            &e.vbit_motions,
            &e.executions,
        )?;
        let mut analysis = quality::analyze(&ctx, &endmill, &e.vbit_motions, axis)?;
        finish_status(
            &mut analysis,
            &candidates,
            &e.executions,
            &e.generation_issues,
        )?;
        Ok(Self {
            artifact_kind: e.artifact_kind,
            schema_version: e.schema_version,
            engine_version: e.engine_version,
            input_fingerprint: e.input_fingerprint,
            motion_fingerprint: e.motion_fingerprint,
            endmill,
            transition: e.transition,
            vbit_spindle_rpm: ctx.spindle,
            vbit_motions: e.vbit_motions,
            executions: e.executions,
            generation_issues: e.generation_issues,
            analysis,
        })
    }
}

/// Recompute a typed plan after intentional in-memory edits. File loading also
/// checks fingerprints; this entry point independently checks motion semantics,
/// execution order, stock, and required final finishing families.
pub fn verify_combined_plan(plan: &CombinedPlan) -> Result<CombinedAnalysis> {
    let endmill = EndmillPlan::from_json(&plan.endmill.to_json()?)?;
    let ctx = Context::new(&endmill.job)?;
    let (axis, candidates) = candidates(&ctx, &endmill)?;
    verify::executions(
        &ctx,
        &endmill,
        &plan.transition,
        &plan.vbit_motions,
        &plan.executions,
    )?;
    let mut analysis = quality::analyze(&ctx, &endmill, &plan.vbit_motions, axis)?;
    finish_status(
        &mut analysis,
        &candidates,
        &plan.executions,
        &plan.generation_issues,
    )?;
    Ok(analysis)
}
fn finish_status(
    analysis: &mut CombinedAnalysis,
    candidates: &[Candidate],
    executions: &[Execution],
    issues: &[GenerationIssue],
) -> Result<()> {
    let expected = candidates
        .iter()
        .filter(|c| c.family != PathFamily::Floor)
        .map(hash)
        .collect::<Result<Vec<_>>>()?;
    let actual = executions
        .iter()
        .filter(|e| e.final_finish && !e.pruned_air)
        .map(|e| hash(&e.candidate))
        .collect::<Result<Vec<_>>>()?;
    analysis.finish_paths_expected = expected.len();
    analysis.finish_paths_executed = actual.len();
    if expected != actual {
        if analysis.status != PlanStatus::Inconclusive {
            analysis.status = PlanStatus::Incomplete;
        }
        analysis.diagnostics.push(error(
            "INCOMPLETE_BOUNDARY_FINISH",
            "complete achievable boundary and rising-detail families must end the V-bit stage",
        ));
    }
    if !issues.is_empty() {
        if analysis.status != PlanStatus::Inconclusive {
            analysis.status = if issues.iter().all(|i| i.code == "UNSUPPORTED_VBIT_ENTRY") {
                PlanStatus::Incomplete
            } else {
                PlanStatus::Inconclusive
            };
        }
        analysis
            .diagnostics
            .extend(issues.iter().map(|i| error(&i.code, &i.message)));
    }
    analysis.pruned_air_paths = executions.iter().filter(|e| e.pruned_air).count();
    analysis.cleanup_paths = executions
        .iter()
        .filter(|e| e.candidate.family == PathFamily::Cleanup)
        .count();
    Ok(())
}

fn lanes(region: &Region, spacing: f64, max: usize) -> Result<Vec<Candidate>> {
    let Some(bounds) = Bounds::of(region) else {
        return Ok(vec![]);
    };
    let count = ((bounds.max.y - bounds.min.y) / spacing).ceil().max(1.);
    if !count.is_finite() || count > max as f64 {
        return Err(error(
            "VBIT_PATH_LIMIT",
            "floor lane count exceeds the path budget",
        ));
    }
    let height = bounds.max.y - bounds.min.y;
    let n = count as usize;
    let mut paths = vec![];
    for i in 0..n {
        let y = bounds.min.y + (i as f64 + 0.5) * height / n as f64;
        let mut xs = vec![];
        for s in region.segments() {
            if (s.start.y > y) != (s.end.y > y) {
                xs.push(
                    s.start.x + (s.end.x - s.start.x) * (y - s.start.y) / (s.end.y - s.start.y),
                );
            }
        }
        xs.sort_by(f64::total_cmp);
        if xs.len() % 2 != 0 {
            return Err(error(
                "LANE_TOPOLOGY",
                "scan line has an odd number of polygon crossings",
            ));
        }
        for pair in xs.chunks_exact(2) {
            if pair[1] - pair[0] <= region.grid().snap_bound_mm() {
                continue;
            }
            let mut points = vec![
                Position::new(Point::new(pair[0], y), 0.),
                Position::new(Point::new(pair[1], y), 0.),
            ];
            if i % 2 == 1 {
                points.reverse();
            }
            paths.push(Candidate {
                family: PathFamily::Floor,
                points,
                source_branch: None,
            });
            if paths.len() > max {
                return Err(error(
                    "VBIT_PATH_LIMIT",
                    "disconnected floor lanes exceed the path budget",
                ));
            }
        }
    }
    Ok(paths)
}
fn candidates(ctx: &Context, endmill: &EndmillPlan) -> Result<(MedialAxis, Vec<Candidate>)> {
    let (axis, medial) = medial::build(ctx)?;
    let cap = ctx.target.depth_cap().mm();
    let full = ctx.target.vbit_centers(&ctx.tool, Depth::new(cap)?)?;
    let centers = ctx
        .target
        .region()
        .erode(ctx.tool.tip_radius().mm() + cap * ctx.tool.angle().slope() + ctx.guard)?;
    let mut paths = vec![];
    if !centers.rings().is_empty() {
        let before = removal_at_slice(
            ctx.target.region().grid(),
            &endmill.motions,
            ctx.mill.radius().mm(),
            cap,
        )?;
        let needed = full
            .area
            .erode(ctx.tolerance)?
            .boolean(BooleanOp::Difference, &before.lower)?;
        let spacing = ctx
            .stepover
            .min(1.6 * (ctx.tool.tip_radius().mm() + ctx.ridge * ctx.tool.angle().slope()));
        if ctx.tool.tip_radius().mm() == 0. && ctx.ridge == 0. && !needed.rings().is_empty() {
            return Err(error(
                "ZERO_RIDGE_AREA_CLEARING",
                "a pointed V-bit cannot clear remaining floor area with positive lane spacing and zero allowed ridge",
            ));
        }
        if !needed.rings().is_empty() {
            if spacing <= 4. * ctx.target.region().grid().snap_bound_mm() {
                return Err(error(
                    "VBIT_FLOOR_PRECISION",
                    "floor lane spacing cannot be represented with the current precision",
                ));
            }
            // Keep boundary ends while allowing interior thirds to receive an
            // independent whole-sweep air proof against recorded endmill cuts.
            paths = lanes(&centers, spacing, ctx.settings.max_paths)?
                .into_iter()
                .flat_map(|lane| {
                    (0..3).map(move |i| Candidate {
                        family: PathFamily::Floor,
                        points: vec![
                            lane.points[0].lerp(lane.points[1], i as f64 / 3.),
                            lane.points[0].lerp(lane.points[1], (i + 1) as f64 / 3.),
                        ],
                        source_branch: None,
                    })
                })
                .collect();
            for p in &mut paths {
                for q in &mut p.points {
                    q.z = -cap;
                }
            }
        }
        for mut ring in centers.rings_mm() {
            ring.push(ring[0]);
            paths.push(Candidate {
                family: PathFamily::Boundary,
                points: ring.into_iter().map(|p| Position::new(p, -cap)).collect(),
                source_branch: None,
            });
        }
    }
    paths.extend(medial);
    for p in full.contact_points {
        let d = ctx.safe_depth(p)?;
        if d > 0. {
            paths.push(Candidate {
                family: PathFamily::Contact,
                points: vec![Position::new(p, -d)],
                source_branch: None,
            });
        }
    }
    if paths.len() > ctx.settings.max_paths {
        return Err(error(
            "VBIT_PATH_LIMIT",
            "candidate family count exceeds the path budget",
        ));
    }
    Ok((axis, paths))
}
fn profile(candidate: &Candidate, cap: f64) -> Vec<Position> {
    let mut points = vec![];
    for (i, &p) in candidate.points.iter().enumerate() {
        if i > 0 {
            let a = candidate.points[i - 1];
            if (a.depth() < cap && p.depth() > cap) || (a.depth() > cap && p.depth() < cap) {
                points.push(a.lerp(p, (cap - a.depth()) / (p.depth() - a.depth())));
            }
        }
        points.push(Position::new(p.xy(), -p.depth().min(cap)));
    }
    points.dedup();
    points
}
fn excursion(
    ctx: &Context,
    candidate: &Candidate,
    cap: f64,
    previous: Position,
    base: usize,
) -> Vec<Motion> {
    let points = profile(candidate, cap);
    if points.is_empty() || points.iter().all(|p| p.depth() == 0.) {
        return vec![];
    }
    let mut moves = vec![];
    let mut position = previous;
    let mut push = |kind: MotionKind, end: Position, feed: Option<f64>| {
        if position != end {
            moves.push(Motion {
                id: base + moves.len(),
                tool_id: ctx.tool_id.clone(),
                operation_id: ctx.operation_id.clone(),
                layer: (cap / ctx.stepdown).ceil() as usize - 1,
                kind,
                start: position,
                end,
                feed_mm_min: feed,
            });
            position = end;
        }
    };
    push(
        MotionKind::RapidXY,
        Position::new(points[0].xy(), ctx.clearance),
        None,
    );
    push(
        MotionKind::Approach,
        Position::new(points[0].xy(), 0.),
        Some(ctx.plunge_feed),
    );
    let n = (points[0].depth() / ctx.stepdown).ceil() as usize;
    for i in 1..=n {
        push(
            MotionKind::Plunge,
            Position::new(
                points[0].xy(),
                -(i as f64 * ctx.stepdown).min(points[0].depth()),
            ),
            Some(ctx.plunge_feed),
        );
    }
    for &p in points.iter().skip(1) {
        push(MotionKind::Cut, p, Some(ctx.feed));
    }
    push(
        MotionKind::RapidRetract,
        Position::new(points.last().unwrap().xy(), ctx.clearance),
        None,
    );
    moves
}
fn air(ctx: &Context, candidate: &Candidate, cap: f64, endmill: &EndmillPlan) -> bool {
    let p = profile(candidate, cap);
    if p.is_empty() {
        return true;
    }
    (0..p.len()).all(|i| {
        let m = Motion {
            id: 0,
            tool_id: ctx.tool_id.clone(),
            operation_id: ctx.operation_id.clone(),
            layer: 0,
            kind: MotionKind::Cut,
            start: p[i],
            end: p[(i + 1).min(p.len() - 1)],
            feed_mm_min: Some(ctx.feed),
        };
        vbit_air_in_endmill(
            &m,
            &ctx.tool,
            &endmill.motions,
            ctx.mill.radius().mm(),
            ctx.guard,
        )
    })
}
fn execute(
    ctx: &Context,
    endmill: &EndmillPlan,
    candidate: &Candidate,
    cap: f64,
    final_finish: bool,
    moves: &mut Vec<Motion>,
    executions: &mut Vec<Execution>,
) -> Result<()> {
    let base = endmill.motions.len() + moves.len();
    let previous = moves.last().or(endmill.motions.last()).map_or(
        Position::new(
            endmill.job.endmill_planning.as_ref().unwrap().start_xy_mm,
            ctx.clearance,
        ),
        |m| m.end,
    );
    let pruned = !final_finish && air(ctx, candidate, cap, endmill);
    let additions = if pruned {
        vec![]
    } else {
        excursion(ctx, candidate, cap, previous, base)
    };
    if moves.len() + additions.len() > ctx.settings.max_motions {
        return Err(error(
            "VBIT_MOTION_LIMIT",
            "V-bit motion budget exhausted; complete excursions are retained",
        ));
    }
    moves.extend(additions);
    executions.push(Execution {
        candidate: candidate.clone(),
        pass_depth_mm: cap,
        final_finish,
        pruned_air: pruned,
        first_motion_id: base,
        end_motion_id: endmill.motions.len() + moves.len(),
    });
    Ok(())
}
pub fn plan_combined(job: &Job) -> Result<CombinedPlan> {
    let ctx = Context::new(job)?;
    let endmill = plan_endmill(job)?;
    let (axis, candidates) = candidates(&ctx, &endmill)?;
    let start = endmill.motions.last().map_or(
        Position::new(
            job.endmill_planning.as_ref().unwrap().start_xy_mm,
            ctx.clearance,
        ),
        |m| m.end,
    );
    let transition = StageTransition {
        after_motion_count: endmill.motions.len(),
        from_tool_id: job.operation.endmill_id.clone(),
        to_tool_id: ctx.tool_id.clone(),
        position: start,
    };
    let mut moves = vec![];
    let mut executions = vec![];
    let mut issues = vec![];
    let levels = (1..=(ctx.target.depth_cap().mm() / ctx.stepdown).ceil() as usize)
        .map(|i| (i as f64 * ctx.stepdown).min(ctx.target.depth_cap().mm()))
        .collect::<Vec<_>>();
    if !ctx.plunge_capable && !candidates.is_empty() {
        issues.push(GenerationIssue {
            code: "UNSUPPORTED_VBIT_ENTRY".into(),
            message: "M4 requires an explicitly plunge-capable V-bit and plunge feed".into(),
        });
    } else {
        'passes: for &cap in &levels {
            for c in &candidates {
                if let Err(d) = execute(&ctx, &endmill, c, cap, false, &mut moves, &mut executions)
                {
                    issues.push(GenerationIssue {
                        code: d.code,
                        message: d.message,
                    });
                    break 'passes;
                }
            }
        }
        for _ in 0..ctx.settings.max_cleanup_iterations {
            if !issues.is_empty() {
                break;
            }
            let quality = quality::sample(&ctx, &endmill, &moves)?;
            let mut points: Vec<_> = quality
                .samples
                .iter()
                .filter(|p| p.missed_reachable_mm > p.allowed_ridge_mm + ctx.tolerance)
                .take(16)
                .map(|p| p.best_tip_center)
                .collect();
            points.extend(quality::floor_cleanup_centers(&ctx, &endmill, &moves)?);
            points.dedup_by(|a, b| a.distance(*b) < ctx.guard);
            points.truncate(16);
            if points.is_empty() {
                break;
            }
            for p in points {
                let d = ctx.safe_depth(p)?;
                if d == 0. {
                    continue;
                }
                let c = Candidate {
                    family: PathFamily::Cleanup,
                    points: vec![Position::new(p, -d)],
                    source_branch: None,
                };
                for &cap in &levels {
                    if let Err(e) =
                        execute(&ctx, &endmill, &c, cap, false, &mut moves, &mut executions)
                    {
                        issues.push(GenerationIssue {
                            code: e.code,
                            message: e.message,
                        });
                        break;
                    }
                }
                if !issues.is_empty() {
                    break;
                }
            }
        }
        if issues.is_empty() {
            for c in candidates.iter().filter(|c| c.family != PathFamily::Floor) {
                if let Err(e) = execute(
                    &ctx,
                    &endmill,
                    c,
                    ctx.target.depth_cap().mm(),
                    true,
                    &mut moves,
                    &mut executions,
                ) {
                    issues.push(GenerationIssue {
                        code: e.code,
                        message: e.message,
                    });
                    break;
                }
            }
        }
    }
    verify::executions(&ctx, &endmill, &transition, &moves, &executions)?;
    let mut analysis = quality::analyze(&ctx, &endmill, &moves, axis)?;
    finish_status(&mut analysis, &candidates, &executions, &issues)?;
    let input_fingerprint = identity(&endmill)?;
    let motion_fingerprint = hash(&(
        &input_fingerprint,
        &endmill.motion_fingerprint,
        &transition,
        &moves,
        &executions,
        &issues,
    ))?;
    Ok(CombinedPlan {
        artifact_kind: "combined_plan".into(),
        schema_version: 1,
        engine_version: env!("CARGO_PKG_VERSION").into(),
        input_fingerprint,
        motion_fingerprint,
        endmill,
        transition,
        vbit_spindle_rpm: ctx.spindle,
        vbit_motions: moves,
        executions,
        generation_issues: issues,
        analysis,
    })
}
