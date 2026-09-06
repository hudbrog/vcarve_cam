use super::{Context, Execution, StageTransition, air, error, excursion, profile, routing};
use crate::{
    geometry::{Result, Segment},
    job::Job,
    motion::{Motion, MotionKind, Position},
    pocket::EndmillPlan,
};
use std::collections::BTreeMap;

pub fn verify_vbit_motions(
    job: &Job,
    endmill_moves: &[Motion],
    moves: &[Motion],
) -> Result<Option<f64>> {
    let ctx = Context::new(job)?;
    let start = endmill_moves.last().map_or(
        Position::new(
            job.endmill_planning.as_ref().unwrap().start_xy_mm,
            ctx.clearance,
        ),
        |m| m.end,
    );
    check(&ctx, endmill_moves.len(), start, moves)
}
fn check(ctx: &Context, base: usize, start: Position, moves: &[Motion]) -> Result<Option<f64>> {
    let mut previous = start;
    let mut minimum: Option<f64> = None;
    let mut margins = std::collections::HashMap::new();
    if moves.len() > ctx.settings.max_motions {
        return Err(error(
            "VBIT_MOTION_LIMIT",
            "saved motion count exceeds its budget",
        ));
    }
    for (i, m) in moves.iter().enumerate() {
        let fail = |s: &str| error("INVALID_VBIT_MOTION", format!("motion {}: {s}", m.id));
        if m.id != base + i
            || m.tool_id != ctx.tool_id
            || m.operation_id != ctx.operation_id
            || m.start != previous
            || m.start == m.end
            || !m.start.finite()
            || !m.end.finite()
        {
            return Err(fail("invalid identity, continuity or coordinates"));
        }
        for p in [m.start, m.end] {
            ctx.target.region().grid().quantize(p.xy())?;
            if p.z > ctx.clearance || p.depth() > ctx.target.depth_cap().mm() {
                return Err(fail("motion exceeds clearance plane or depth cap"));
            }
        }
        let xy = m.start.xy().distance(m.end.xy());
        let drop = m.start.z - m.end.z;
        let expected = if m.kind.rapid() {
            None
        } else if m.kind == MotionKind::Cut {
            Some(ctx.feed)
        } else {
            Some(ctx.plunge_feed)
        };
        if m.feed_mm_min != expected {
            return Err(fail("feed differs from explicit tool setting"));
        }
        let valid = match m.kind {
            MotionKind::RapidXY => m.start.z == ctx.clearance && m.end.z == ctx.clearance,
            MotionKind::RapidRetract => {
                xy == 0.
                    && drop < 0.
                    && m.end.z == ctx.clearance
                    && m.start.z <= 0.
                    && i > 0
                    && moves[i - 1].kind.cutting()
            }
            MotionKind::Approach => xy == 0. && m.start.z == ctx.clearance && m.end.z == 0.,
            MotionKind::Plunge => {
                ctx.plunge_capable
                    && xy == 0.
                    && drop > 0.
                    && drop <= ctx.stepdown + 1e-12
                    && m.start.z <= 0.
            }
            MotionKind::Cut => xy > 0. && m.start.z <= 0. && m.end.z <= 0.,
            MotionKind::Ramp => false,
        };
        if !valid {
            return Err(fail("entry, cut, or clearance-plane rule violated"));
        }
        if m.kind.cutting() {
            let r = |p: Position| ctx.tool.tip_radius().mm() + p.depth() * ctx.tool.angle().slope();
            // Identity, continuity, feeds, entry and range are still checked
            // for every record above. Only an exact forward XYZ duplicate can
            // reuse the independent continuous-clearance calculation.
            let key = [
                m.start.x.to_bits(),
                m.start.y.to_bits(),
                m.start.z.to_bits(),
                m.end.x.to_bits(),
                m.end.y.to_bits(),
                m.end.z.to_bits(),
            ];
            let margin = if let Some(&margin) = margins.get(&key) {
                margin
            } else {
                let margin = ctx.target.boundary().variable_radius_margin_mm(
                    Segment {
                        start: m.start.xy(),
                        end: m.end.xy(),
                    },
                    r(m.start),
                    r(m.end),
                )?;
                if margins.len() < 131072 {
                    margins.insert(key, margin);
                }
                margin
            };
            if margin < 0. {
                return Err(error(
                    "VBIT_SWEEP_CLEARANCE",
                    format!(
                        "motion {} has negative continuous clearance bound {margin} mm",
                        m.id
                    ),
                ));
            }
            minimum = Some(minimum.map_or(margin, |old| old.min(margin)));
        }
        previous = m.end;
    }
    if previous.z != ctx.clearance {
        return Err(error(
            "INVALID_VBIT_MOTION",
            "stage must finish at clearance",
        ));
    }
    Ok(minimum)
}
/// An in-memory result tied to the exact immutable motions just checked.
/// Analysis consumes this instead of verifying the same motions a second time.
pub(super) struct CheckedMotions<'a> {
    motions: &'a [Motion],
    minimum_margin: Option<f64>,
}
impl CheckedMotions<'_> {
    pub(super) fn motions(&self) -> &[Motion] {
        self.motions
    }
    pub(super) fn minimum_margin(&self) -> Option<f64> {
        self.minimum_margin
    }
}

pub(super) fn executions<'a>(
    ctx: &Context,
    endmill: &EndmillPlan,
    transition: &StageTransition,
    moves: &'a [Motion],
    executions: &[Execution],
) -> Result<CheckedMotions<'a>> {
    let start = endmill.motions.last().map_or(
        Position::new(
            endmill.job.endmill_planning.as_ref().unwrap().start_xy_mm,
            ctx.clearance,
        ),
        |m| m.end,
    );
    if transition.after_motion_count != endmill.motions.len()
        || transition.position != start
        || transition.from_tool_id != endmill.job.operation.endmill_id
        || transition.to_tool_id != ctx.tool_id
        || start.z != ctx.clearance
    {
        return Err(error(
            "TOOL_STAGE_ORDER",
            "tool transition must follow all endmill work at the common clearance plane",
        ));
    }
    let minimum_margin = check(ctx, endmill.motions.len(), start, moves)?;
    let mut at = 0;
    let mut previous = start;
    let mut final_started = false;
    let mut progress: BTreeMap<String, f64> = BTreeMap::new();
    for e in executions {
        if e.candidate
            .points
            .iter()
            .any(|p| !p.finite() || p.z > 0. || p.depth() > ctx.target.depth_cap().mm())
        {
            return Err(error(
                "INVALID_EXECUTION",
                "path profile has invalid cutting coordinates",
            ));
        }
        for p in &e.candidate.points {
            ctx.target.region().grid().quantize(p.xy())?;
        }
        if e.final_finish && e.pass_depth_mm != ctx.target.depth_cap().mm() {
            return Err(error(
                "BOUNDARY_FINISH_DEPTH",
                "the final family must execute its complete depth profile",
            ));
        }
        if e.candidate.points.is_empty()
            || e.candidate.points.len() > ctx.settings.max_curve_segments + 1
            || !e.pass_depth_mm.is_finite()
            || e.pass_depth_mm <= 0.
            || e.pass_depth_mm > ctx.target.depth_cap().mm()
            || e.first_motion_id != endmill.motions.len() + at
            || e.end_motion_id < e.first_motion_id
            || e.end_motion_id > endmill.motions.len() + moves.len()
        {
            return Err(error(
                "INVALID_EXECUTION",
                "invalid path profile, depth or motion range",
            ));
        }
        if final_started && !e.final_finish {
            return Err(error(
                "BOUNDARY_FINISH_ORDER",
                "cleanup and roughing cannot follow the final boundary family",
            ));
        }
        final_started |= e.final_finish;
        let key = routing::candidate_key(&e.candidate)?;
        let reached = e.pass_depth_mm.min(
            e.candidate
                .points
                .iter()
                .map(|p| p.depth())
                .fold(0., f64::max),
        );
        let prior = *progress.get(&key).unwrap_or(&0.);
        if reached > prior + ctx.stepdown + 1e-12
            && !air(ctx, &e.candidate, (reached - ctx.stepdown).max(0.), endmill)
        {
            return Err(error(
                "VBIT_STOCK_STEPDOWN",
                "a depth pass was omitted without a proof that preceding material was removed",
            ));
        }
        progress.insert(key, prior.max(reached));
        if !e.pruned_air
            && previous.z <= 0.
            && !profile(&e.candidate, e.pass_depth_mm)
                .first()
                .is_some_and(|&p| routing::can_link(ctx, previous, p))
        {
            return Err(error(
                "INVALID_VBIT_LINK",
                "linked execution cannot establish clearance and stock stepdown",
            ));
        }
        let mut expected = if e.pruned_air {
            if e.final_finish || !air(ctx, &e.candidate, e.pass_depth_mm, endmill) {
                return Err(error(
                    "INVALID_AIR_PRUNING",
                    "only proved air cuts may be omitted; final finish is retained",
                ));
            }
            vec![]
        } else {
            excursion(
                ctx,
                &e.candidate,
                e.pass_depth_mm,
                previous,
                e.first_motion_id,
            )
        };
        let next = e.end_motion_id - endmill.motions.len();
        // A coincident single-point path may contribute only its retract;
        // linking the following path removes that too, leaving an empty range.
        if expected
            .last()
            .is_some_and(|m| m.kind == MotionKind::RapidRetract)
            && next - at + 1 == expected.len()
        {
            expected.pop();
        }
        if moves[at..next] != expected {
            return Err(error(
                "EXECUTION_MISMATCH",
                "recorded motion range does not execute its declared depth-limited path",
            ));
        }
        if let Some(last) = expected.last() {
            previous = last.end;
        }
        at = next;
    }
    if at != moves.len() {
        return Err(error(
            "EXECUTION_MISMATCH",
            "some cutting moves have no path execution record",
        ));
    }
    Ok(CheckedMotions {
        motions: moves,
        minimum_margin,
    })
}
