use super::settings::{Context, error};
use super::{EndmillAnalysis, EntryStrategy, PlanStatus, depths, layer_stock};
use crate::{
    geometry::{Point, PointLocation, Result, Segment},
    job::Job,
    motion::{Motion, MotionKind, Position},
};

/// Independent of offset construction: minimize distance over the whole segment.
/// The deepest endpoint bounds clearance along a linear ramp conservatively.
pub(super) fn center_margin(ctx: &Context, a: Point, b: Point, depth: f64) -> Result<f64> {
    let qa = ctx.target.boundary().sample(a)?;
    let qb = ctx.target.boundary().sample(b)?;
    let distance = ctx
        .target
        .boundary()
        .segment_distance_mm(Segment { start: a, end: b })?;
    let margin = distance
        - ctx.required_clearance(depth)
        - qa.numerical_reserve_mm.max(qb.numerical_reserve_mm);
    if qa.location != PointLocation::Inside || qb.location != PointLocation::Inside || margin < 0. {
        return Err(error(
            "MOTION_CLEARANCE",
            format!(
                "whole-segment cutter clearance fails at depth {depth} mm (margin {margin} mm)"
            ),
        ));
    }
    Ok(margin)
}

pub fn verify_endmill_motions(job: &Job, motions: &[Motion]) -> Result<EndmillAnalysis> {
    analyze(&Context::new(job)?, job, motions)
}

pub(super) fn analyze(ctx: &Context, job: &Job, motions: &[Motion]) -> Result<EndmillAnalysis> {
    let levels = depths(ctx)?;
    if motions.len() > ctx.settings.max_motions {
        return Err(error(
            "MOTION_LIMIT",
            "motion count exceeds the configured limit",
        ));
    }
    let mut previous = Position::new(ctx.settings.start_xy_mm, ctx.settings.clearance_z_mm);
    let mut previous_layer = 0;
    let mut minimum: Option<f64> = None;
    for (id, m) in motions.iter().enumerate() {
        let fail = |message: &str| error("INVALID_MOTION", format!("motion {id}: {message}"));
        if m.id != id
            || m.tool_id != ctx.tool_id
            || m.operation_id != ctx.operation_id
            || m.layer >= levels.len()
            || m.layer < previous_layer
        {
            return Err(fail("invalid identity or layer order"));
        }
        if !m.start.finite() || !m.end.finite() || m.start != previous || m.start == m.end {
            return Err(fail("nonfinite, disconnected, or zero-length motion"));
        }
        ctx.target.region().grid().quantize(m.start.xy())?;
        ctx.target.region().grid().quantize(m.end.xy())?;
        if m.start.z > ctx.settings.clearance_z_mm
            || m.end.z > ctx.settings.clearance_z_mm
            || m.start.depth() > levels[m.layer]
            || m.end.depth() > levels[m.layer]
        {
            return Err(fail("motion exceeds layer depth or clearance plane"));
        }
        let dz = m.start.z - m.end.z;
        let xy = m.start.xy().distance(m.end.xy());
        let expected_feed = if m.kind.rapid() {
            None
        } else if m.kind == MotionKind::Cut {
            Some(ctx.feed)
        } else {
            Some(ctx.entry_feed)
        };
        if m.feed_mm_min != expected_feed {
            return Err(fail("feed does not match explicit tool/entry settings"));
        }
        let valid = match m.kind {
            MotionKind::RapidXY => {
                m.start.z == ctx.settings.clearance_z_mm && m.end.z == ctx.settings.clearance_z_mm
            }
            MotionKind::RapidRetract => {
                xy == 0.
                    && dz < 0.
                    && m.end.z == ctx.settings.clearance_z_mm
                    && m.start.z < 0.
                    && id > 0
                    && motions[id - 1].kind.cutting()
            }
            MotionKind::Approach => {
                xy == 0. && m.start.z == ctx.settings.clearance_z_mm && m.end.z == 0.
            }
            MotionKind::Plunge => {
                matches!(ctx.settings.entry, EntryStrategy::Plunge)
                    && ctx.entry_supported(job)
                    && xy == 0.
                    && dz > 0.
                    && dz <= ctx.stepdown + 1e-12
                    && m.start.z <= 0.
            }
            MotionKind::Ramp => match ctx.settings.entry {
                EntryStrategy::Ramp { max_angle_deg, .. } => {
                    ctx.entry_supported(job)
                        && xy > 0.
                        && dz > 0.
                        && dz <= ctx.stepdown / 2. + 1e-12
                        && dz <= xy * max_angle_deg.to_radians().tan() + 1e-12
                        && m.start.z <= 0.
                }
                _ => false,
            },
            MotionKind::Cut => dz == 0. && xy > 0. && m.end.depth() == levels[m.layer],
        };
        if !valid {
            return Err(fail(
                "motion kind violates entry, cutting, or clearance-plane rules",
            ));
        }
        if m.kind.cutting() {
            let margin = center_margin(
                ctx,
                m.start.xy(),
                m.end.xy(),
                m.start.depth().max(m.end.depth()),
            )?;
            minimum = Some(minimum.map_or(margin, |old| old.min(margin)));
        }
        previous = m.end;
        previous_layer = m.layer;
    }
    if previous.z != ctx.settings.clearance_z_mm {
        return Err(error(
            "INVALID_MOTION",
            "stage must end at the clearance plane",
        ));
    }
    let layers = levels
        .into_iter()
        .map(|d| layer_stock(ctx, d, motions))
        .collect::<Result<Vec<_>>>()?;
    let status = if layers.iter().any(|l| l.status == PlanStatus::Inconclusive) {
        PlanStatus::Inconclusive
    } else if layers.iter().any(|l| l.status == PlanStatus::Incomplete) {
        PlanStatus::Incomplete
    } else if layers.iter().all(|l| l.status == PlanStatus::Empty) {
        PlanStatus::Empty
    } else {
        PlanStatus::Complete
    };
    let diagnostics = layers.iter().flat_map(|l| l.diagnostics.clone()).collect();
    Ok(EndmillAnalysis{status,layers,checked_motion_count:motions.len(),minimum_center_margin_mm:minimum,diagnostics,
        meaning:"Endmill stage only: continuous center clearance and accessible floor coverage at the planned depth slices, within the declared XY coverage tolerance.".into(),
        limitations:vec![
            "Clearance is checked against normalized geometry; source curve/quantization errors are reported by job inspection.".into(),
            "Slice sweep polygons bracket individual capsules; repeated integer Boolean operations are engineering approximations, not a formal interval proof.".into(),
            "Remaining target includes slopes, allowance, and endmill-inaccessible detail for M4. Adaptive full-volume verification belongs to M5.".into(),
            "Tool capabilities and feeds are explicit inputs; cutting loads, fixtures, machine motion, and G-code are not verified by M3.".into(),
        ]})
}
