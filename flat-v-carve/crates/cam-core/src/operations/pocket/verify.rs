//! Independent whole-sweep containment and conservative floor coverage.
use crate::{
    geometry::{
        BooleanOp, BoundaryQuery, Diagnostic, PointLocation, Region, Result, Segment,
        union::UnionAccumulator,
    },
    stock::capsule_bounds,
    toolpath::{Interpolation, MotionEffect, PlannedMotion},
};

/// Recheck recorded Pocket cuts without regenerating their paths. The caller
/// supplies resolved heights; the operation's portable settings remain the
/// authority for depth increments, allowance, and requested finish coverage.
pub fn verify_recorded_motions(
    region: &Region,
    settings: &crate::project::v5::PocketSettingsV5,
    radius: f64,
    top: f64,
    bottom: f64,
    tolerance: f64,
    motions: &[PlannedMotion],
) -> Result<()> {
    let stepdown = settings
        .assignment
        .max_stepdown_mm
        .ok_or_else(|| error("MISSING_MACHINING_SETTING", "Pocket stepdown is required"))?;
    let allowance = settings.wall_allowance_mm.ok_or_else(|| {
        error(
            "MISSING_MACHINING_SETTING",
            "Pocket wall allowance is required",
        )
    })?;
    if !radius.is_finite()
        || radius <= 0.
        || !stepdown.is_finite()
        || stepdown <= 0.
        || !top.is_finite()
        || !bottom.is_finite()
        || bottom >= top
        || !tolerance.is_finite()
        || tolerance < 8. * region.grid().tolerance_mm()
        || !allowance.is_finite()
        || allowance < 0.
    {
        return Err(error(
            "POCKET_VERIFICATION_INPUT",
            "Invalid pocket verification inputs",
        ));
    }
    let layers = ((top - bottom) / stepdown).ceil();
    if layers > settings.limits.max_layers as f64 || motions.len() > settings.limits.max_motions {
        return Err(error(
            "POCKET_VERIFICATION_LIMIT",
            "Pocket exceeds verification limits",
        ));
    }
    containment(
        region,
        radius + if settings.finish_walls { 0. } else { allowance },
        top,
        bottom,
        motions,
    )?;
    for m in motions
        .iter()
        .filter(|m| m.effect == MotionEffect::MillingSweep)
    {
        if m.layer == 0
            || m.layer > layers as usize
            || m.start.z.min(m.end.z) < (top - m.layer as f64 * stepdown).max(bottom) - 1e-9
        {
            return Err(error(
                "POCKET_STEPDOWN",
                format!("motion {} exceeds its depth layer", m.id),
            ));
        }
    }
    let guard = 4. * region.grid().tolerance_mm();
    let centers =
        region.erode(radius + if settings.finish_walls { 0. } else { allowance } + guard)?;
    for component in region.components() {
        if component
            .erode(radius + allowance + guard)?
            .rings()
            .is_empty()
        {
            return Err(error(
                "POCKET_NO_ACCESS",
                "Selected pocket has no positive-area roughing access",
            ));
        }
    }
    // Restrict the slice to motions at this layer or earlier. A deeper pass
    // cannot be used to disguise a missing earlier clearing layer.
    for layer in 1..=layers as usize {
        let z = (top - layer as f64 * stepdown).max(bottom);
        let prefix: Vec<_> = motions
            .iter()
            .filter(|m| m.layer <= layer)
            .cloned()
            .collect();
        if !coverage(region, &centers, radius, z, tolerance, &prefix)?
            .rings()
            .is_empty()
        {
            return Err(error(
                "POCKET_COVERAGE",
                format!("Pocket layer {layer} leaves reachable material"),
            ));
        }
    }
    Ok(())
}

pub(super) fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("pocket")
}

pub(super) fn segment_inside(
    query: &BoundaryQuery,
    a: crate::geometry::Point,
    b: crate::geometry::Point,
    radius: f64,
) -> Result<bool> {
    let at = query.sample(a)?;
    Ok(at.location == PointLocation::Inside
        && query.segment_distance_mm(Segment { start: a, end: b })?
            >= radius + at.numerical_reserve_mm)
}

/// Enclose an arc in chord capsules expanded by their maximum sagitta.
/// Bounded subdivision is for a geometric enclosure, never point sampling proof.
pub(super) fn chords(
    m: &PlannedMotion,
    tolerance: f64,
) -> Result<Vec<(crate::motion::Position, crate::motion::Position, f64)>> {
    let Interpolation::ArcFeed(arc) = m.interpolation else {
        return Ok(vec![(m.start, m.end, 0.)]);
    };
    let radius = arc
        .radius(m.start.xy())
        .ok_or_else(|| error("POCKET_ARC", "degenerate arc"))?;
    let sweep = arc
        .sweep_rad(m.start.xy(), m.end.xy())
        .ok_or_else(|| error("POCKET_ARC", "degenerate arc"))?;
    let step = (2. * (1. - (tolerance / radius).min(0.5)).acos()).min(std::f64::consts::FRAC_PI_2);
    let n = (sweep / step).ceil().max(1.);
    if !n.is_finite() || n > 16384. {
        return Err(error(
            "POCKET_VERIFICATION_LIMIT",
            "arc enclosure exceeds subdivision budget",
        ));
    }
    let n = n as usize;
    let sagitta = radius * (1. - (sweep / n as f64 / 2.).cos());
    let mut parts = Vec::with_capacity(n);
    let mut from = m.start;
    for i in 1..=n {
        let t = i as f64 / n as f64;
        let to = if i == n {
            m.end
        } else {
            crate::motion::Position::new(
                arc.point_at(m.start.xy(), m.end.xy(), t).unwrap(),
                m.start.z + (m.end.z - m.start.z) * t,
            )
        };
        parts.push((from, to, sagitta));
        from = to;
    }
    Ok(parts)
}

pub(super) fn containment(
    region: &Region,
    radius: f64,
    top: f64,
    bottom: f64,
    motions: &[PlannedMotion],
) -> Result<()> {
    let query = BoundaryQuery::new(region);
    let mut work = 0;
    for m in motions
        .iter()
        .filter(|m| m.effect == MotionEffect::MillingSweep)
    {
        if m.start.z.min(m.end.z) < bottom - 1e-9 || m.start.z.max(m.end.z) > top + 1e-9 {
            return Err(error(
                "POCKET_DEPTH",
                format!("motion {} exceeds pocket depth bounds", m.id),
            ));
        }
        for (a, b, sagitta) in chords(m, region.grid().tolerance_mm() / 4.)? {
            work += 1;
            if work > 1_000_000 {
                return Err(error(
                    "POCKET_VERIFICATION_LIMIT",
                    "sweep enclosure exceeds work budget",
                ));
            }
            if !segment_inside(&query, a.xy(), b.xy(), radius + sagitta)? {
                return Err(error(
                    "POCKET_CONTAINMENT",
                    format!(
                        "motion {} cannot establish cutter clearance from pocket walls/islands",
                        m.id
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Only level moves at/below the requested slice contribute to this lower
/// removal bound. Omitting descent removal is conservative; it cannot hide gaps.
pub(super) fn coverage(
    region: &Region,
    centers: &Region,
    radius: f64,
    z: f64,
    tolerance: f64,
    motions: &[PlannedMotion],
) -> Result<Region> {
    let mut removed = UnionAccumulator::new(region.grid());
    let mut work = 0;
    for m in motions.iter().filter(|m| {
        m.effect == MotionEffect::MillingSweep && m.start.z <= z + 1e-9 && m.end.z <= z + 1e-9
    }) {
        for (a, b, sagitta) in chords(m, region.grid().tolerance_mm() / 4.)? {
            work += 1;
            if work > 1_000_000 {
                return Err(error(
                    "POCKET_VERIFICATION_LIMIT",
                    "coverage exceeds work budget",
                ));
            }
            if radius <= sagitta {
                continue;
            }
            removed.push(capsule_bounds(region.grid(), a.xy(), b.xy(), radius - sagitta)?.lower)?;
        }
    }
    let removed = removed.finish()?;
    let reachable = centers
        .dilate(radius)?
        .boolean(BooleanOp::Intersection, region)?;
    reachable
        .erode(tolerance)?
        .boolean(BooleanOp::Difference, &removed.dilate(tolerance)?)
}
