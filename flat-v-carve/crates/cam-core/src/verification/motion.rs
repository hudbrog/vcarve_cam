use super::{Context, Finding, VerificationStatus};
use crate::{
    geometry::Segment,
    motion::{Motion, MotionKind, Position},
    pocket::EntryStrategy,
};

fn coordinate(value: f64, places: usize) -> f64 {
    // Check the actual decimal representation, including ties-to-even, rather
    // than assuming multiplication/round/division equals machine formatting.
    format!("{value:.places$}").parse().expect("formatted f64")
}
fn position(p: Position, places: usize) -> Position {
    Position {
        x: coordinate(p.x, places),
        y: coordinate(p.y, places),
        z: coordinate(p.z, places),
    }
}
fn finding(code: &str, message: &str, m: &Motion) -> Finding {
    Finding {
        code: code.into(),
        status: VerificationStatus::Failed,
        message: message.into(),
        location: m.start.xy(),
        cell: None,
        motion_id: Some(m.id),
        measured_mm: None,
        limit_mm: None,
    }
}

pub(super) fn rounded(
    e: &[Motion],
    v: &[Motion],
    places: usize,
) -> (Vec<Motion>, Vec<Motion>, f64, Vec<Finding>) {
    let mut change: f64 = 0.;
    let mut findings = vec![];
    let mut round = |moves: &[Motion]| {
        moves
            .iter()
            .map(|m| {
                let mut q = m.clone();
                q.start = position(m.start, places);
                q.end = position(m.end, places);
                for (p, r) in [(m.start, q.start), (m.end, q.end)] {
                    change = change
                        .max((p.x - r.x).abs())
                        .max((p.y - r.y).abs())
                        .max((p.z - r.z).abs());
                }
                let dot = (m.end.x - m.start.x) * (q.end.x - q.start.x)
                    + (m.end.y - m.start.y) * (q.end.y - q.start.y)
                    + (m.end.z - m.start.z) * (q.end.z - q.start.z);
                if q.start == q.end {
                    findings.push(finding(
                        "ROUNDING_ZERO_LENGTH",
                        "decimal formatting collapses this move; no move was silently discarded",
                        m,
                    ));
                } else if dot <= 0. {
                    findings.push(finding(
                        "ROUNDING_REVERSED_MOVE",
                        "formatted move does not preserve its direction",
                        m,
                    ));
                } else if matches!(
                    m.kind,
                    MotionKind::Plunge
                        | MotionKind::Ramp
                        | MotionKind::RapidRetract
                        | MotionKind::Approach
                ) && m.end.z != m.start.z
                    && q.end.z == q.start.z
                {
                    findings.push(finding(
                        "ROUNDING_LOST_Z",
                        "formatting removes the required Z displacement",
                        m,
                    ));
                }
                q
            })
            .collect()
    };
    let endmill = round(e);
    let vbit = round(v);
    (endmill, vbit, change, findings)
}

/// Semantic checks include every approach/retract/link. Positive analytical
/// margins prove a complete sweep clear; ambiguous margins go to the adaptive
/// overcut comparison, rather than turning a negative lower bound into a gouge.
pub(super) fn check(
    ctx: &Context<'_>,
    e: &[Motion],
    v: &[Motion],
    places: Option<usize>,
) -> (usize, Vec<Finding>) {
    let settings = ctx.job.endmill_planning.as_ref().unwrap();
    let round = |x| places.map_or(x, |n| coordinate(x, n));
    let clearance = round(settings.clearance_z_mm);
    let mut previous = Position::new(settings.start_xy_mm, settings.clearance_z_mm);
    if let Some(n) = places {
        previous = position(previous, n);
    }
    let mut clear = 0;
    let mut findings = vec![];
    let cap = ctx.target.depth_cap().mm();
    for (i, m) in e.iter().chain(v).enumerate() {
        let is_endmill = i < e.len();
        let stage_start = i == 0 || i == e.len();
        let id = if is_endmill {
            &ctx.job.operation.endmill_id
        } else {
            &ctx.job.operation.vbit_id
        };
        let slot = ctx.job.tools.iter().find(|t| &t.id == id).unwrap();
        let xy = m.start.xy().distance(m.end.xy());
        let drop = m.start.z - m.end.z;
        let step = slot.max_stepdown_mm.unwrap_or(0.);
        let plunge = if is_endmill {
            matches!(settings.entry, EntryStrategy::Plunge)
                && slot.plunge_capable.unwrap_or(ctx.mill.plunge_capable())
        } else {
            slot.plunge_capable == Some(true)
        };
        let entry_feed = if is_endmill {
            match settings.entry {
                EntryStrategy::Plunge => slot.plunge_feed_mm_min,
                EntryStrategy::Ramp { feed_mm_min, .. } => Some(feed_mm_min),
            }
        } else {
            slot.plunge_feed_mm_min
        };
        let feed = if m.kind.rapid() {
            None
        } else if m.kind == MotionKind::Cut {
            slot.cutting_feed_mm_min
        } else {
            entry_feed
        };
        let valid = match m.kind {
            MotionKind::RapidXY => m.start.z == clearance && m.end.z == clearance,
            MotionKind::RapidRetract => {
                xy == 0.
                    && drop < 0.
                    && m.start.z <= 0.
                    && m.end.z == clearance
                    && !stage_start
                    && e.iter()
                        .chain(v)
                        .nth(i - 1)
                        .is_some_and(|p| p.kind.cutting())
            }
            MotionKind::Approach => xy == 0. && m.start.z == clearance && m.end.z == 0.,
            MotionKind::Plunge => {
                plunge && xy == 0. && drop > 0. && drop <= step + ctx.reserve && m.start.z <= 0.
            }
            MotionKind::Ramp => {
                is_endmill
                    && slot.ramp_capable == Some(true)
                    && match settings.entry {
                        EntryStrategy::Ramp { max_angle_deg, .. } => {
                            xy > 0.
                                && drop > 0.
                                && drop <= step / 2. + ctx.reserve
                                && drop <= xy * max_angle_deg.to_radians().tan() + ctx.reserve
                                && m.start.z <= 0.
                        }
                        _ => false,
                    }
            }
            MotionKind::Cut => {
                xy > 0. && m.start.z <= 0. && m.end.z <= 0. && (!is_endmill || drop == 0.)
            }
        };
        if !m.start.finite()
            || !m.end.finite()
            || m.start == m.end
            || m.start != previous
            || m.id != i
            || &m.tool_id != id
            || m.operation_id != ctx.job.operation.id
            || !valid
            || m.feed_mm_min != feed
            || !m.kind.rapid() && feed.is_none()
            || m.start.z > clearance
            || m.end.z > clearance
            || m.start.depth().max(m.end.depth()) > cap
            || clearance < settings.clearance_z_mm
            || stage_start && m.start.z != clearance
        {
            findings.push(finding(
                "M5_INVALID_MOTION",
                "identity, continuity, feed, entry, depth, or clearance-plane contract violated",
                m,
            ));
        }
        if m.kind.cutting() && m.start.finite() && m.end.finite() {
            for t in [0., 0.5, 1.] {
                let p = m.start.lerp(m.end, t);
                if let Ok(target) = ctx.target.nominal_depth(p.xy()) {
                    let excess = p.depth() - target.mm();
                    if excess - ctx.reserve > ctx.tolerance {
                        let mut f = finding(
                            "M5_OVERCUT",
                            "cutting depth exceeds the nominal surface at this independent motion witness",
                            m,
                        );
                        f.location = p.xy();
                        f.measured_mm = Some(super::Interval::positive(
                            excess - ctx.reserve,
                            excess + ctx.reserve,
                        ));
                        f.limit_mm = Some(ctx.tolerance);
                        findings.push(f);
                    }
                }
            }
            let radius = |p: Position| {
                p.depth() * ctx.target.angle().slope()
                    + if is_endmill {
                        ctx.mill.radius().mm() + ctx.allowance.mm()
                    } else {
                        ctx.vbit.tip_radius().mm()
                    }
            };
            if ctx
                .target
                .boundary()
                .variable_radius_margin_mm(
                    Segment {
                        start: m.start.xy(),
                        end: m.end.xy(),
                    },
                    radius(m.start),
                    radius(m.end),
                )
                .is_ok_and(|margin| margin >= 0.)
            {
                clear += 1;
            }
        }
        previous = m.end;
    }
    if previous.z != clearance
        && let Some(m) = v.last().or_else(|| e.last())
    {
        findings.push(finding(
            "M5_FINAL_CLEARANCE",
            "final motion must end on the clearance plane",
            m,
        ));
    }
    (clear, findings)
}
