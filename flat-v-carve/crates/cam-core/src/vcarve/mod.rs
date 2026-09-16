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
    geometry::{BooleanOp, Point, Region, Result, Segment},
    job::VcarveInput,
    model::Depth,
    motion::{Motion, MotionKind, Position},
    pocket::{EndmillPlan, GenerationIssue, PlanStatus},
    stock::{SliceRemoval, StockQuery, removal_at_slice},
    svg::Bounds,
};
pub use medial::{MedialAxis, MedialBranch};
pub use quality::{CombinedAnalysis, CombinedSlice, QualitySample};
use serde::{Deserialize, Serialize};
use settings::{Context, error};
pub use settings::{FinishTransit, VBitPlanningSettings};
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
        let ctx = Context::new(&endmill.input)?;
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
    let ctx = Context::new(&endmill.input)?;
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
    routing::reconcile_endpoints(ctx, &mut paths);
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
        // `previous` carries the plane the excursion is entered from: the
        // stage clearance, or the shorter lift the previous transit earned.
        push(
            MotionKind::RapidXY,
            Position::new(points[0].xy(), previous.z),
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
/// What the tool does between two cutting excursions.
struct Transit {
    /// The join happens at cutting depth: the previous excursion's retract is
    /// dropped and the travel is a cutting move.
    link: bool,
    /// The height the travel uses when it is not a join.
    plane: f64,
}
/// The plane a lifted transit travels at. Every point of the V-bit cone is at
/// or above its tip, so any tip height at or above the stock top is clear by
/// construction: the clearance an operator configures is a margin against
/// measurement error, not a geometric requirement. Measuring it from the pass
/// depth instead of the stock top is therefore the whole of "a smaller lift" —
/// and it stops there: a pass deeper than the clearance leaves no room above the
/// stock top, so the released plane stands. Below the stock top the margin stops
/// protecting anything, and the excursion has to stay down on a proof instead.
fn lift_plane(ctx: &Context, cap: f64) -> f64 {
    if cap < ctx.clearance {
        ctx.clearance - cap
    } else {
        ctx.clearance
    }
}
/// Whether a straight join at cutting depth stays inside the target. The join is
/// a cutting move, so the same continuous sweep bound the verifier applies to
/// every cut decides it.
fn against_region(ctx: &Context, from: Position, to: Position) -> bool {
    let r = |p: Position| ctx.tool.tip_radius().mm() + p.depth() * ctx.tool.angle().slope();
    ctx.target
        .boundary()
        .variable_radius_margin_mm(
            Segment {
                start: from.xy(),
                end: to.xy(),
            },
            r(from),
            r(to),
        )
        .is_ok_and(|margin| margin >= 0.)
}
/// Waypoints for a detour, as offsets from the straight line's midpoint: a fan
/// either side of the line plus points along it, all scaled by the budget. One
/// waypoint is enough whenever the shape bulges between two excursions — which
/// is what a corner is — and every candidate is checked with the same bound the
/// verifier applies, so a route is never proposed that the checks would reject.
const DETOUR_REACH: [(f64, f64); 16] = [
    (0., 0.35),
    (0., -0.35),
    (0.35, 0.),
    (-0.35, 0.),
    (0., 0.6),
    (0., -0.6),
    (0.6, 0.),
    (-0.6, 0.),
    (0.25, 0.35),
    (-0.25, 0.35),
    (0.25, -0.35),
    (-0.25, -0.35),
    (0., 0.85),
    (0., -0.85),
    (0.5, 0.5),
    (-0.5, -0.5),
];
/// A path from `from` to `to` that stays inside the shape the V-bit is meant to
/// cut at `cap`, shorter than `budget` millimetres. Travelling it removes only
/// material this stage removes anyway, so it replaces a lift rather than cutting
/// anything new; every segment is checked with the same continuous sweep bound
/// the verifier applies to a recorded cut.
fn detour(
    ctx: &Context,
    from: Position,
    to: Position,
    cap: f64,
    budget: f64,
) -> Option<Vec<Position>> {
    if from.depth() != cap || !to.xy().finite() {
        return None;
    }
    let grid = ctx.target.region().grid();
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let length = dx.hypot(dy);
    if length == 0. {
        return None;
    }
    // Perpendicular and along-line directions, in budget fractions.
    let (px, py) = (-dy / length, dx / length);
    let (ax, ay) = (dx / length, dy / length);
    let mut best: Option<(f64, Position)> = None;
    for (along_share, across_share) in DETOUR_REACH {
        let point = Point::new(
            from.x + dx / 2. + budget * (px * across_share + ax * along_share),
            from.y + dy / 2. + budget * (py * across_share + ay * along_share),
        );
        let Ok(cell) = grid.quantize(point) else {
            continue;
        };
        let way = grid.point(cell);
        let via = Position::new(way, -cap);
        let total = from.xy().distance(way) + way.distance(to.xy());
        if total >= budget || best.as_ref().is_some_and(|(b, _)| *b <= total) {
            continue;
        }
        if !against_region(ctx, from, via) || !against_region(ctx, via, to) {
            continue;
        }
        best = Some((total, via));
    }
    best.map(|(_, via)| vec![via])
}
/// How the V-bit gets from the end of the previous excursion to the first cut
/// of `target`. Shared with the verifier so a recorded program and its
/// independent re-derivation agree by construction.
fn transit(ctx: &Context, from: Position, target: &Candidate, cap: f64) -> Transit {
    let lift = Transit {
        link: false,
        plane: ctx.clearance,
    };
    let Some(&first) = profile(target, cap).first() else {
        return lift;
    };
    if !target.points.iter().any(|p| p.depth() > 0.) {
        return lift;
    }
    let joined = Transit {
        link: true,
        plane: ctx.clearance,
    };
    if routing::can_link(ctx, from, first) {
        return joined;
    }
    if ctx.settings.transit == FinishTransit::Retract {
        return lift;
    }
    let lift = Transit {
        link: false,
        plane: lift_plane(ctx, cap),
    };
    if ctx.settings.transit == FinishTransit::ShortLift || from.xy() == first.xy() {
        return lift;
    }
    // Below the stock top the cone has to be argued for, and the shape is the
    // argument: wherever the cone at pass depth fits inside the target, the cut
    // is one this stage already owes. What is left is whether travelling there
    // costs less than the lift it replaces — the lift descends at the plunge
    // feed, the join travels at the cutting feed. Rapid moves are machine-side
    // and not known here, so this compares the two feeds only and leans towards
    // lifting on a long join.
    if against_region(ctx, from, first)
        && from.xy().distance(first.xy()) <= lift_budget(ctx, lift.plane, first.z)
    {
        joined
    } else {
        lift
    }
}
/// The distance a join may cover and still be quicker than the lift it replaces:
/// the lift's descent at the plunge feed, measured at the cutting feed.
fn lift_budget(ctx: &Context, plane: f64, target_z: f64) -> f64 {
    (plane - target_z).max(0.) * ctx.feed / ctx.plunge_feed
}
/// `candidate` with a detour prepended to it, when keeping the bit down and
/// cutting across to it is quicker than lifting and the straight join would
/// leave the shape. `None` wherever the lift stands.
fn routed(
    ctx: &Context,
    moves: &[Motion],
    executions: &[Execution],
    candidate: &Candidate,
    cap: f64,
    final_finish: bool,
) -> Option<Candidate> {
    if ctx.settings.transit != FinishTransit::Route
        || !candidate.points.iter().any(|p| p.depth() > 0.)
    {
        return None;
    }
    // The previous excursion ended with a placeholder retract, and it has to be
    // one this stage would join rather than lift between.
    let retract = moves.last()?;
    let previous = executions.last()?;
    if retract.kind != MotionKind::RapidRetract
        || previous.pass_depth_mm != cap
        || previous.final_finish != final_finish
        || previous.pruned_air
        || previous.end_motion_id <= previous.first_motion_id
    {
        return None;
    }
    let straight = transit(ctx, retract.start, candidate, cap);
    if straight.link {
        return None;
    }
    let first = *profile(candidate, cap).first()?;
    let way = detour(
        ctx,
        retract.start,
        first,
        cap,
        lift_budget(ctx, straight.plane, first.z),
    )?;
    let mut points = Vec::with_capacity(way.len() + 1 + candidate.points.len());
    points.push(retract.start);
    points.extend(way);
    points.extend(candidate.points.iter().copied());
    Some(Candidate {
        family: candidate.family,
        points,
        source_branch: candidate.source_branch,
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
            endmill.input.endmill_planning.as_ref().unwrap().start_xy_mm,
            ctx.clearance,
        ),
        |m| m.end,
    );
    let pruned = !final_finish && air(ctx, candidate, cap, &stock.query);
    // A detour is worth taking when it beats the lift it replaces: the lift
    // spends its time descending at the plunge feed, the detour spends cut feed.
    // Prepending it to the excursion makes one continuous cut out of the two,
    // so the ordinary transit decision below sees a join rather than a lift.
    let extended = if pruned {
        None
    } else {
        routed(ctx, moves, executions, candidate, cap, final_finish)
    };
    let candidate = extended.as_ref().unwrap_or(candidate);
    // The previous excursion ended with a placeholder retract; the transit this
    // one needs decides what happens to it — a join at depth, a shorter lift, or
    // the stage clearance plane it already names.
    let join = moves
        .last()
        .filter(|m| m.kind == MotionKind::RapidRetract)
        .zip(executions.last())
        .filter(|(_, previous)| {
            previous.pass_depth_mm == cap
                && previous.final_finish == final_finish
                && !previous.pruned_air
                && previous.end_motion_id > previous.first_motion_id
        })
        .map(|(m, _)| transit(ctx, m.start, candidate, cap));
    let (link_from, plane) = match join {
        Some(t) if t.link => (Some(moves.last().unwrap().start), ctx.clearance),
        Some(t) => (None, t.plane),
        None => (None, ctx.clearance),
    };
    let linked = !pruned && candidate.points.iter().any(|p| p.depth() > 0.) && link_from.is_some();
    let additions = if pruned {
        vec![]
    } else if linked {
        base -= 1;
        excursion(ctx, candidate, cap, moves.last().unwrap().start, base)
    } else {
        // `previous` keeps the recorded motion's identity but takes the transit's
        // height: the entry rapid, the approach and the plunge all start from it.
        excursion(
            ctx,
            candidate,
            cap,
            Position::new(previous.xy(), plane),
            base,
        )
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
    } else if !pruned && plane < ctx.clearance {
        // Shorten the placeholder to the plane this transit actually travels
        // at, rather than climbing past it and back down.
        moves.last_mut().unwrap().end.z = plane;
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
/// Combined planning for one Flat V-carve operation. The selected region is
/// part of the input, so no source is imported or re-imported here.
pub fn plan_combined(input: &VcarveInput) -> Result<CombinedPlan> {
    let mut timing = crate::timing::Timer::new("combined");
    let mut ctx = Context::new(input)?;
    timing.lap("context");
    let (endmill, target) = crate::pocket::plan_with_target(input, Some(ctx.target.clone()))?;
    // Both contexts use this job's selected geometry, depth and V-bit angle.
    // Retain the endmill's populated Voronoi/access caches for V-bit queries.
    ctx.target = target;
    timing.lap("endmill");
    let stock = EndmillStock::new(&endmill, ctx.mill.radius().mm())?;
    let (axis, candidates) = candidates(&ctx, &endmill)?;
    let features = routing::FeatureIndex::new(ctx.target.region());
    timing.lap("candidates");
    let start = endmill.motions.last().map_or(
        Position::new(
            input.endmill_planning.as_ref().unwrap().start_xy_mm,
            ctx.clearance,
        ),
        |m| m.end,
    );
    let transition = StageTransition {
        after_motion_count: endmill.motions.len(),
        from_tool_id: input.operation.endmill_id.clone(),
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
                for c in &routing::feature_order(&group, previous, &features) {
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
            let previous = moves.last().map_or(start.xy(), |m: &Motion| m.end.xy());
            for p in routing::order_points(points, previous, &features) {
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
                let previous = moves.last().map_or(start.xy(), |m: &Motion| m.end.xy());
                for c in &routing::feature_order(&group, previous, &features) {
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
    use crate::pocket::{EntryStrategy, plan_endmill};

    fn transit_census(fixture: &str, transit: FinishTransit) -> (usize, Vec<f64>) {
        let mut job = crate::job::input_from_fixture_json(fixture).unwrap();
        job.vbit_planning.as_mut().unwrap().transit = transit;
        let plan = plan_combined(&job).unwrap();
        let retracts: Vec<_> = plan
            .vbit_motions
            .iter()
            .filter(|m| m.kind == MotionKind::RapidRetract)
            .map(|m| m.end.z)
            .collect();
        (retracts.len(), retracts)
    }

    #[test]
    fn transit_modes_share_the_join_and_differ_only_in_how_far_they_lift() {
        for fixture in [
            include_str!("../../../../fixtures/m4/wide-floor.json"),
            include_str!("../../../../fixtures/m4/island.json"),
        ] {
            let clearance = 5.;
            let (retracts, released) = transit_census(fixture, FinishTransit::Retract);
            assert!(retracts > 1);
            // The released behaviour retracts to the stage clearance plane at
            // every cycle, whatever the pass depth.
            assert!(released.iter().all(|&z| z == clearance));
            let (short, lifted) = transit_census(fixture, FinishTransit::ShortLift);
            // A short lift keeps every cycle and every path; it only stops the
            // retract at the configured clearance above the pass depth.
            assert_eq!(short, retracts);
            assert!(lifted.iter().all(|&z| z >= 0. && z <= clearance));
            assert!(lifted.iter().filter(|&&z| z < clearance).count() > 1);
            let (routed, joined) = transit_census(fixture, FinishTransit::Route);
            // Routing to the next cut can only ever remove cycles, never add
            // them, and it never leaves the tool below the stock top.
            assert!(routed <= short);
            assert!(joined.iter().all(|&z| z >= 0. && z <= clearance));
        }
    }

    #[test]
    fn routing_keeps_the_bit_down_where_a_detour_beats_the_lift() {
        let mut routed_any = false;
        for fixture in [
            include_str!("../../../../fixtures/m4/island.json"),
            include_str!("../../../../fixtures/m4/wide-floor.json"),
        ] {
            let mut job = crate::job::input_from_fixture_json(fixture).unwrap();
            let mut census = |transit: FinishTransit| {
                job.vbit_planning.as_mut().unwrap().transit = transit;
                let plan = plan_combined(&job).unwrap();
                let cycles = plan
                    .vbit_motions
                    .iter()
                    .filter(|m| m.kind == MotionKind::RapidRetract)
                    .count();
                let cut: f64 = plan
                    .vbit_motions
                    .iter()
                    .filter(|m| m.kind.cutting())
                    .map(|m| m.start.xy().distance(m.end.xy()))
                    .sum();
                (cycles, cut)
            };
            let (lifted, lifted_cut) = census(FinishTransit::ShortLift);
            let (routed, routed_cut) = census(FinishTransit::Route);
            println!("cycles {lifted} -> {routed}, cut {lifted_cut:.1} -> {routed_cut:.1}");
            // Routing may only remove lifts, and only by cutting the ground it
            // travels over — which is ground this stage clears anyway.
            assert!(
                routed <= lifted,
                "routing added {} lift cycles",
                routed.saturating_sub(lifted)
            );
            assert!(
                routed_cut >= lifted_cut,
                "routing must not lose cutting length"
            );
            routed_any |= routed < lifted;
        }
        assert!(routed_any, "routing must remove a lift somewhere");
    }

    #[test]
    fn routing_joins_only_inside_the_shape_and_only_when_it_pays() {
        let mut job = crate::job::input_from_fixture_json(include_str!(
            "../../../../fixtures/m4/wide-floor.json"
        ))
        .unwrap();
        job.vbit_planning.as_mut().unwrap().transit = FinishTransit::Route;
        let ctx = Context::new(&job).unwrap();
        let cap = ctx.target.depth_cap().mm();
        let target = |p: Point| Candidate {
            family: PathFamily::Medial,
            points: vec![Position::new(p, -cap)],
            source_branch: None,
        };
        // The fixture pocket spans x 0..30, y 10..30 in setup coordinates.
        let from = Position::new(Point::new(5., 20.), -cap);
        // A join across the middle of the floor keeps the cone inside the shape,
        // and is quicker than the lift it replaces.
        let along = transit(&ctx, from, &target(Point::new(14., 20.)), cap);
        assert!(along.link);
        // The same join crossing the wall allowance leaves the shape, and falls
        // back to a lift rather than cutting material the target keeps.
        let across = transit(&ctx, from, &target(Point::new(5., 29.5)), cap);
        assert!(!across.link);
        assert_eq!(across.plane, lift_plane(&ctx, cap));
        // A join that would spend longer at the cutting feed than the lift
        // spends descending is a lift: the budget is that descent, measured at
        // the cutting feed.
        let budget = lift_budget(&ctx, across.plane, -cap);
        assert!(budget > 10. && budget < 25., "budget {budget}");
        let far = transit(&ctx, from, &target(Point::new(5. + budget + 1., 20.)), cap);
        assert!(!far.link);
    }

    #[test]
    fn transit_planes_report_the_lift_each_pass_depth_earns() {
        for (name, fixture) in [
            (
                "wide-floor",
                include_str!("../../../../fixtures/m4/wide-floor.json"),
            ),
            (
                "island",
                include_str!("../../../../fixtures/m4/island.json"),
            ),
        ] {
            let mut job = crate::job::input_from_fixture_json(fixture).unwrap();
            let ctx = Context::new(&job).unwrap();
            let mut planes = |t: FinishTransit| {
                job.vbit_planning.as_mut().unwrap().transit = t;
                let plan = plan_combined(&job).unwrap();
                let mut zs: Vec<_> = plan
                    .vbit_motions
                    .iter()
                    .filter(|m| m.kind == MotionKind::RapidRetract)
                    .map(|m| m.end.z)
                    .collect();
                zs.sort_by(f64::total_cmp);
                zs
            };
            let released = planes(FinishTransit::Retract);
            let short = planes(FinishTransit::ShortLift);
            assert_eq!(released.len(), short.len(), "{name}");
            for (a, b) in released.iter().zip(short.iter()) {
                assert_eq!(*a, ctx.clearance, "{name}");
                assert!(
                    *b <= *a && *b >= 0.,
                    "{name}: a short lift may only stop lower, not higher"
                );
            }
            assert!(
                short.first().is_some_and(|z| *z < ctx.clearance),
                "{name}: the short lift must actually shorten the retract"
            );
            assert!(
                short.last().is_some_and(|z| *z == ctx.clearance),
                "{name}: the stage must still end at the clearance plane"
            );
        }
    }

    #[test]
    fn retained_target_matches_independent_geometry_and_access_queries() {
        for input in [
            include_str!("../../../../fixtures/m4/island.json"),
            include_str!("../../../../fixtures/m4/finite-tip.json"),
            include_str!("../../../../fixtures/m4/exact-fit.json"),
        ] {
            let job = crate::job::input_from_fixture_json(input).unwrap();
            let (endmill, target) = crate::pocket::plan_with_target(&job, None).unwrap();
            let ctx = Context::new(&job).unwrap();
            assert_eq!(
                serde_json::to_value(target.region()).unwrap(),
                serde_json::to_value(ctx.target.region()).unwrap()
            );
            assert_eq!(target.depth_cap(), ctx.target.depth_cap());
            assert_eq!(target.angle(), ctx.target.angle());
            for depth in [target.depth_cap().mm(), target.depth_cap().mm() / 2.] {
                let query = |t: &crate::target::Target| {
                    t.vbit_centers(&ctx.tool, Depth::new(depth).unwrap())
                        .unwrap()
                };
                assert_eq!(
                    serde_json::to_value(query(&target)).unwrap(),
                    serde_json::to_value(query(&ctx.target)).unwrap()
                );
            }
            assert_eq!(
                endmill.to_json().unwrap(),
                plan_endmill(&job).unwrap().to_json().unwrap()
            );
            let (shared, retained) =
                crate::pocket::plan_with_target(&job, Some(ctx.target.clone())).unwrap();
            assert!(std::sync::Arc::ptr_eq(&retained, &ctx.target));
            assert_eq!(endmill.to_json().unwrap(), shared.to_json().unwrap());
        }
    }

    #[test]
    fn parallel_candidates_preserve_families_geometry_and_error_order() {
        for input in [
            include_str!("../../../../fixtures/m4/island.json"),
            include_str!("../../../../fixtures/m4/finite-tip.json"),
            include_str!("../../../../fixtures/m4/contact-line.json"),
            include_str!("../../../../fixtures/m4/contact-point.json"),
            include_str!("../../../../fixtures/m4/resource-limit.json"),
        ] {
            let job = crate::job::input_from_fixture_json(input).unwrap();
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
            let mut job = crate::job::input_from_fixture_json(include_str!(
                "../../../../fixtures/m4/island.json"
            ))
            .unwrap();
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
