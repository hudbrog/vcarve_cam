//! Combined endmill / finite-tip V-bit planning. M4 reports slice and sampled
//! quality evidence; adaptive full-volume certification remains M5.
mod medial;
mod prune;
mod quality;
mod rest;
mod routing;
mod settings;
mod verify;
use crate::{
    geometry::{BooleanOp, Point, Region, Result},
    job::Job,
    model::Depth,
    motion::{Motion, MotionKind, Position},
    pocket::{EndmillPlan, GenerationIssue, PlanStatus, plan_endmill},
    stock::{SliceRemoval, StockQuery, removal_at_slice},
    svg::Bounds,
};
pub use medial::{MedialAxis, MedialBranch};
pub use quality::{CombinedAnalysis, CombinedSlice, QualitySample};
use serde::{Deserialize, Serialize};
pub use settings::VBitPlanningSettings;
use settings::{Context, error};
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
    // Cached stock, medial geometry and samples are regenerated, never trusted.
    #[serde(skip_serializing)]
    pub analysis: CombinedAnalysis,
}

/// Immutable proof of a fresh identity check and complete M4 reconstruction.
/// Only these constructors can create it; callers cannot mutate the bound plan.
pub struct AuthenticatedPlan(CombinedPlan);
mod retained;
pub use retained::{
    VerificationReceipt, export_retained_plan, plan_combined_with_receipt, verify_retained_plan,
};
impl AuthenticatedPlan {
    pub fn from_reader(reader: impl std::io::Read) -> Result<Self> {
        CombinedPlan::from_reader(reader).map(Self)
    }
    pub fn from_plan(plan: &CombinedPlan) -> Result<Self> {
        plan.revalidated().map(Self)
    }
    pub fn plan(&self) -> &CombinedPlan {
        &self.0
    }
}
#[derive(Deserialize)]
struct Envelope {
    artifact_kind: String,
    schema_version: u32,
    engine_version: String,
    input_fingerprint: String,
    motion_fingerprint: String,
    endmill: crate::pocket::Envelope,
    transition: StageTransition,
    vbit_motions: Vec<Motion>,
    executions: Vec<Execution>,
    generation_issues: Vec<GenerationIssue>,
}
fn hash<T: Serialize>(v: &T) -> Result<String> {
    crate::plan_hash::hash(v).map_err(|e| error("PLAN_JSON", e.to_string()))
}
fn identity(endmill: &EndmillPlan) -> Result<String> {
    identity_for_endmill(&endmill.input_fingerprint)
}
fn identity_for_endmill(input_fingerprint: &str) -> Result<String> {
    hash(&(
        env!("CARGO_PKG_VERSION"),
        "combined-v1;clipper2-rust=1.1.0;boostvoronoi=0.12.1",
        input_fingerprint,
    ))
}
impl CombinedPlan {
    fn revalidated(&self) -> Result<Self> {
        Self::from_envelope(Envelope {
            artifact_kind: self.artifact_kind.clone(),
            schema_version: self.schema_version,
            engine_version: self.engine_version.clone(),
            input_fingerprint: self.input_fingerprint.clone(),
            motion_fingerprint: self.motion_fingerprint.clone(),
            endmill: self.endmill.envelope(),
            transition: self.transition.clone(),
            vbit_motions: self.vbit_motions.clone(),
            executions: self.executions.clone(),
            generation_issues: self.generation_issues.clone(),
        })
    }
    pub fn to_json(&self) -> Result<String> {
        let json = serde_json::to_string(self)
            .map(|s| s + "\n")
            .map_err(|e| error("PLAN_JSON", e.to_string()))?;
        if json.len() > 128_000_000 {
            return Err(error(
                "PLAN_RESOURCE_LIMIT",
                "combined plan exceeds the 128 MB reload limit",
            ));
        }
        Ok(json)
    }
    pub fn from_json(json: &str) -> Result<Self> {
        if json.len() > 128_000_000 {
            return Err(error("PLAN_RESOURCE_LIMIT", "combined plan exceeds 128 MB"));
        }
        let e: Envelope =
            serde_json::from_str(json).map_err(|e| error("PLAN_JSON", e.to_string()))?;
        Self::from_envelope(e)
    }

    /// Stream a plan from caller-owned storage, retaining all identity and
    /// execution checks. Callers must bound untrusted input before using this API.
    pub fn from_reader(reader: impl std::io::Read) -> Result<Self> {
        let e = serde_json::from_reader(reader).map_err(|e| error("PLAN_JSON", e.to_string()))?;
        Self::from_envelope(e)
    }

    fn from_envelope(e: Envelope) -> Result<Self> {
        if e.artifact_kind != "combined_plan"
            || e.schema_version != 1
            || e.engine_version != env!("CARGO_PKG_VERSION")
        {
            return Err(error(
                "PLAN_VERSION",
                "unsupported combined schema or engine; regenerate the plan",
            ));
        }
        let endmill = EndmillPlan::from_envelope(e.endmill)?;
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
        let checked = verify::executions(
            &ctx,
            &endmill,
            &e.transition,
            &e.vbit_motions,
            &e.executions,
        )?;
        let mut analysis = quality::analyze(&ctx, &endmill, checked, axis, None)?;
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
    let endmill = plan.endmill.revalidated()?;
    let ctx = Context::new(&endmill.job)?;
    let (axis, candidates) = candidates(&ctx, &endmill)?;
    let checked = verify::executions(
        &ctx,
        &endmill,
        &plan.transition,
        &plan.vbit_motions,
        &plan.executions,
    )?;
    let mut analysis = quality::analyze(&ctx, &endmill, checked, axis, None)?;
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
    let mut expected = candidates
        .iter()
        .filter(|c| c.family != PathFamily::Floor)
        .map(routing::candidate_key)
        .collect::<Result<Vec<_>>>()?;
    let mut actual = executions
        .iter()
        .filter(|e| e.final_finish && !e.pruned_air)
        .map(|e| routing::candidate_key(&e.candidate))
        .collect::<Result<Vec<_>>>()?;
    analysis.finish_paths_expected = expected.len();
    analysis.finish_paths_executed = actual.len();
    expected.sort_unstable();
    actual.sort_unstable();
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
/// These callers own a freshly generated or authenticated/rebuilt endmill plan.
/// Reuse a rebuilt slice only if every actual clipped XY sweep is identical.
/// In particular, ramps and motions that stop above the new slice must differ.
fn endmill_slice(ctx: &Context, endmill: &EndmillPlan, depth: f64) -> Result<SliceRemoval> {
    for layer in &endmill.analysis.layers {
        if endmill
            .motions
            .iter()
            .all(|m| m.at_depth(depth) == m.at_depth(layer.depth_mm))
        {
            let mut removal = layer.removal.clone();
            removal.depth_mm = depth;
            return Ok(removal);
        }
    }
    removal_at_slice(
        ctx.target.region().grid(),
        &endmill.motions,
        ctx.mill.radius().mm(),
        depth,
    )
}

fn candidates(ctx: &Context, endmill: &EndmillPlan) -> Result<(MedialAxis, Vec<Candidate>)> {
    let parallel = ctx
        .target
        .region()
        .rings()
        .iter()
        .map(|r| r.points.len())
        .sum::<usize>()
        > 4096
        && std::thread::available_parallelism().map_or(1, usize::from) > 1;
    candidate_families(ctx, endmill, parallel)
}

fn candidate_families(
    ctx: &Context,
    endmill: &EndmillPlan,
    parallel: bool,
) -> Result<(MedialAxis, Vec<Candidate>)> {
    let _timing = crate::timing::Timer::new("vbit candidates");
    // Medial chords and area paths read the same immutable target, but neither
    // depends on the other's candidates. Preserve family and error order when
    // collecting them, including the contact paths that follow medial paths.
    let ((axis, medial), (mut paths, contacts)) = if parallel {
        std::thread::scope(|scope| {
            let medial = scope.spawn(|| medial::build(ctx));
            let area = area_candidates(ctx, endmill);
            let medial = medial
                .join()
                .map_err(|_| error("CANDIDATE_WORKER_PANIC", "medial candidate worker failed"))?;
            Ok((medial?, area?))
        })?
    } else {
        (medial::build(ctx)?, area_candidates(ctx, endmill)?)
    };
    paths.extend(medial);
    for p in contacts {
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
    routing::weld_endpoints(ctx, &mut paths)?;
    prune::simplify_contours(ctx, &mut paths)?;
    prune::redundant_spokes(ctx, &axis, &mut paths);
    Ok((axis, paths))
}

fn area_candidates(ctx: &Context, endmill: &EndmillPlan) -> Result<(Vec<Candidate>, Vec<Point>)> {
    let _timing = crate::timing::Timer::new("vbit area candidates");
    let cap = ctx.target.depth_cap().mm();
    let full = ctx.target.vbit_centers(&ctx.tool, Depth::new(cap)?)?;
    let centers = ctx
        .target
        .region()
        .erode(ctx.tool.tip_radius().mm() + cap * ctx.tool.angle().slope() + ctx.guard)?;
    let mut paths = vec![];
    if !centers.rings().is_empty() {
        let before = endmill_slice(ctx, endmill, cap)?;
        let needed = full
            .area
            .erode(ctx.tolerance)?
            .boolean(BooleanOp::Difference, &before.lower)?;
        let spacing = ctx
            .stepover
            // Contours converge at corners; the parallel-line half-spacing
            // formula alone is insufficient there. Keep each offset band
            // within one permitted-ridge cutter footprint with a reserve.
            .min(0.9 * (ctx.tool.tip_radius().mm() + ctx.ridge * ctx.tool.angle().slope()));
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
            paths = rest::floor_paths(ctx, &centers, &needed, spacing)?;
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
    Ok((paths, full.contact_points))
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
    if previous.z <= 0. {
        push(MotionKind::Cut, points[0], Some(ctx.feed));
    } else {
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
struct EndmillStock<'a> {
    plan: &'a EndmillPlan,
    query: StockQuery<'a>,
}
impl<'a> EndmillStock<'a> {
    fn new(plan: &'a EndmillPlan, radius: f64) -> Result<Self> {
        Ok(Self {
            plan,
            query: StockQuery::endmill(&plan.motions, radius)?,
        })
    }
}
fn air(ctx: &Context, candidate: &Candidate, cap: f64, stock: &StockQuery<'_>) -> bool {
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
        stock.vbit_air(&m, &ctx.tool, ctx.guard)
    })
}
fn execute(
    ctx: &Context,
    stock: &EndmillStock<'_>,
    candidate: &Candidate,
    cap: f64,
    final_finish: bool,
    moves: &mut Vec<Motion>,
    executions: &mut Vec<Execution>,
) -> Result<()> {
    let endmill = stock.plan;
    let mut base = endmill.motions.len() + moves.len();
    let previous = moves.last().or(endmill.motions.last()).map_or(
        Position::new(
            endmill.job.endmill_planning.as_ref().unwrap().start_xy_mm,
            ctx.clearance,
        ),
        |m| m.end,
    );
    let pruned = !final_finish && air(ctx, candidate, cap, &stock.query);
    let linked = !pruned
        && candidate.points.iter().any(|p| p.depth() > 0.)
        && moves.last().is_some_and(|m| {
            m.kind == MotionKind::RapidRetract
                && executions.last().is_some_and(|e| {
                    e.pass_depth_mm == cap
                        && e.final_finish == final_finish
                        && !e.pruned_air
                        && e.end_motion_id > e.first_motion_id
                })
                && profile(candidate, cap)
                    .first()
                    .is_some_and(|&p| routing::can_link(ctx, m.start, p))
        });
    let additions = if pruned {
        vec![]
    } else if linked {
        base -= 1;
        excursion(ctx, candidate, cap, moves.last().unwrap().start, base)
    } else {
        excursion(ctx, candidate, cap, previous, base)
    };
    if moves.len() + additions.len() - usize::from(linked) > ctx.settings.max_motions {
        return Err(error(
            "VBIT_MOTION_LIMIT",
            "V-bit motion budget exhausted; complete excursions are retained",
        ));
    }
    if linked {
        moves.pop();
        executions.last_mut().unwrap().end_motion_id -= 1;
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
    let mut timing = crate::timing::Timer::new("combined");
    let ctx = Context::new(job)?;
    timing.lap("context");
    let endmill = plan_endmill(job)?;
    timing.lap("endmill");
    let stock = EndmillStock::new(&endmill, ctx.mill.radius().mm())?;
    let (axis, candidates) = candidates(&ctx, &endmill)?;
    timing.lap("candidates");
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
    let mut sample_reuse = None;
    let mut last_finish_start = 0;
    let mut cleanup_added = false;
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
            for floor in [true, false] {
                if !floor {
                    last_finish_start = executions.len();
                }
                let group: Vec<_> = candidates
                    .iter()
                    .filter(|c| (c.family == PathFamily::Floor) == floor)
                    .cloned()
                    .collect();
                let previous = moves.last().map_or(start.xy(), |m: &Motion| m.end.xy());
                for c in &routing::order(&group, previous) {
                    if let Err(d) =
                        execute(&ctx, &stock, c, cap, false, &mut moves, &mut executions)
                    {
                        issues.push(GenerationIssue {
                            code: d.code,
                            message: d.message,
                        });
                        break 'passes;
                    }
                }
            }
        }
        timing.lap("depth passes");
        for _ in 0..ctx.settings.max_cleanup_iterations {
            if !issues.is_empty() {
                break;
            }
            let (quality, floor) = quality::cleanup(&ctx, &endmill, &moves)?;
            let mut points: Vec<_> = quality
                .samples
                .iter()
                .filter(|p| p.missed_reachable_mm > p.allowed_ridge_mm + ctx.tolerance)
                .take(16)
                .map(|p| p.best_tip_center)
                .collect();
            points.extend(floor.points);
            points.dedup_by(|a, b| a.distance(*b) < ctx.guard);
            points.truncate(16);
            if points.is_empty() {
                sample_reuse = Some(quality::SampleReuse::new(
                    quality,
                    &moves,
                    Some(floor.slice),
                )?);
                break;
            }
            for p in points {
                cleanup_added = true;
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
                        execute(&ctx, &stock, &c, cap, false, &mut moves, &mut executions)
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
        timing.lap("cleanup");
        if issues.is_empty() {
            if !cleanup_added
                && executions[last_finish_start..]
                    .iter()
                    .all(|e| !e.pruned_air)
            {
                // The last full-depth boundary/detail traversal is already the
                // final family when no later cleanup cut disturbed it.
                for e in &mut executions[last_finish_start..] {
                    e.final_finish = true;
                }
            } else {
                let group: Vec<_> = candidates
                    .iter()
                    .filter(|c| c.family != PathFamily::Floor)
                    .cloned()
                    .collect();
                let previous = moves.last().map_or(start.xy(), |m| m.end.xy());
                for c in &routing::order(&group, previous) {
                    if let Err(e) = execute(
                        &ctx,
                        &stock,
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
    }
    timing.lap("final finish");
    let checked = verify::executions(&ctx, &endmill, &transition, &moves, &executions)?;
    timing.lap("verify executions");
    let mut analysis = quality::analyze(&ctx, &endmill, checked, axis, sample_reuse)?;
    timing.lap("analyze");
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

#[cfg(test)]
mod slice_reuse_tests {
    use super::*;
    use crate::pocket::EntryStrategy;

    #[test]
    fn parallel_candidates_preserve_families_geometry_and_error_order() {
        for input in [
            include_str!("../../../../fixtures/m4/island.json"),
            include_str!("../../../../fixtures/m4/finite-tip.json"),
            include_str!("../../../../fixtures/m4/contact-line.json"),
            include_str!("../../../../fixtures/m4/contact-point.json"),
            include_str!("../../../../fixtures/m4/resource-limit.json"),
        ] {
            let job = Job::from_json(input).unwrap();
            let endmill = plan_endmill(&job).unwrap();
            // Separate contexts also exercise concurrent initialization of the
            // shared target's lazy geometric data, without a warm serial cache.
            let serial = candidate_families(&Context::new(&job).unwrap(), &endmill, false);
            let parallel = candidate_families(&Context::new(&job).unwrap(), &endmill, true);
            match (serial, parallel) {
                (Ok(serial), Ok(parallel)) => assert_eq!(
                    serde_json::to_value(serial).unwrap(),
                    serde_json::to_value(parallel).unwrap()
                ),
                (Err(serial), Err(parallel)) => {
                    assert_eq!(serial.code, parallel.code);
                    assert_eq!(serial.message, parallel.message);
                }
                (serial, parallel) => panic!("candidate results differ: {serial:?}, {parallel:?}"),
            }
        }
    }

    #[test]
    fn reused_stock_matches_fresh_sweeps_for_plunges_ramps_and_multiple_layers() {
        for ramp in [false, true] {
            let mut job =
                Job::from_json(include_str!("../../../../fixtures/m4/island.json")).unwrap();
            if ramp {
                job.tools[0].ramp_capable = Some(true);
                job.endmill_planning.as_mut().unwrap().entry = EntryStrategy::Ramp {
                    max_angle_deg: 10.,
                    feed_mm_min: 100.,
                };
            }
            let endmill = plan_endmill(&job).unwrap();
            let ctx = Context::new(&job).unwrap();
            assert_eq!(endmill.analysis.layers.len(), 2);
            for depth in [0.25, 0.5, 1., 1.25, 1.875, 2., 2.25] {
                let reused = endmill_slice(&ctx, &endmill, depth).unwrap();
                let fresh = removal_at_slice(
                    ctx.target.region().grid(),
                    &endmill.motions,
                    ctx.mill.radius().mm(),
                    depth,
                )
                .unwrap();
                assert_eq!(
                    serde_json::to_value(reused).unwrap(),
                    serde_json::to_value(fresh).unwrap(),
                    "ramp={ramp}, depth={depth}"
                );
            }
            let matches_layer = endmill.analysis.layers.iter().any(|layer| {
                endmill
                    .motions
                    .iter()
                    .all(|m| m.at_depth(0.5) == m.at_depth(layer.depth_mm))
            });
            assert_eq!(
                matches_layer, !ramp,
                "a partially clipped ramp must not reuse a deeper footprint"
            );
        }
    }
}
