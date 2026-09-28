//! Independent whole-sweep containment and conservative floor coverage.
use crate::{
    geometry::{
        BooleanOp, BoundaryQuery, Diagnostic, PointLocation, Region, Result, Segment,
        union::UnionAccumulator,
    },
    stock::capsule_bounds,
    toolpath::{Interpolation, MotionEffect, MotionPurpose, PlannedMotion},
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
    for (index, m) in motions
        .iter()
        .enumerate()
        .filter(|(_, m)| m.purpose == MotionPurpose::Approach)
    {
        let prefix = &motions[..index];
        let valid = if m.start.z == m.end.z {
            short_link(
                prefix,
                m.start,
                m.end,
                settings,
                region.grid().tolerance_mm(),
            )
        } else {
            m.start.xy() == m.end.xy()
                && m.start.z > m.end.z
                && endpoint_floor(prefix, m.end.xy(), top) <= m.end.z
        };
        let entry_feed = match settings.entry {
            crate::project::v5::PocketEntry::Plunge => settings.assignment.plunge_feed_mm_min,
            crate::project::v5::PocketEntry::Ramp { feed_mm_min, .. }
            | crate::project::v5::PocketEntry::Helix { feed_mm_min, .. } => feed_mm_min,
        }
        .unwrap_or(0.);
        let feed_limit = if m.start.z == m.end.z {
            let cutting_feed = if prefix
                .iter()
                .rev()
                .find(|p| matches!(p.purpose, MotionPurpose::Rough | MotionPurpose::Finish))
                .is_some_and(|p| p.purpose == MotionPurpose::Finish)
            {
                settings
                    .finish_feed_mm_min
                    .or(settings.assignment.cutting_feed_mm_min)
            } else {
                settings.assignment.cutting_feed_mm_min
            };
            entry_feed.min(cutting_feed.unwrap_or(0.))
        } else {
            entry_feed
        };
        if m.interpolation != Interpolation::LinearFeed
            || !valid
            || !m.feed_mm_min.is_some_and(|f| f > 0. && f <= feed_limit)
        {
            return Err(error(
                "POCKET_LINK_UNVERIFIED",
                format!("motion {} has no established link/entry clearance", m.id),
            ));
        }
    }
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

/// A bridge advances at most one stepover from an already established cutter
/// footprint. It cannot chain into a long slot: the preceding motion must be
/// a contour cut or its lead-out, not another bridge. Wall/island containment
/// is checked separately over the entire capsule.
pub(super) fn short_link(
    prefix: &[PlannedMotion],
    from: crate::motion::Position,
    to: crate::motion::Position,
    settings: &crate::project::v5::PocketSettingsV5,
    e: f64,
) -> bool {
    from.z == to.z
        && from.xy().distance(to.xy()) <= settings.assignment.stepover_mm.unwrap_or(0.) + 4. * e
        && prefix.last().is_some_and(|m| {
            m.end == from
                && m.start.z == from.z
                && m.effect == MotionEffect::MillingSweep
                && matches!(
                    m.purpose,
                    MotionPurpose::Rough | MotionPurpose::Finish | MotionPurpose::LeadOut
                )
        })
}

/// Exact endpoints of earlier cutter sweeps establish vertical cleared columns.
/// No sampled stock display or tolerance-expanded removal is used as evidence.
pub(super) fn endpoint_floor(
    motions: &[PlannedMotion],
    at: crate::geometry::Point,
    top: f64,
) -> f64 {
    motions
        .iter()
        .filter(|m| m.effect == MotionEffect::MillingSweep)
        .flat_map(|m| [m.start, m.end])
        .filter(|p| p.xy() == at)
        .fold(top, |z, p| z.min(p.z))
}

/// Reuse only exactly matching, level paths. For a helix both half-circles
/// must already have run at the proposed surface; a descending arc is not proof.
pub(super) fn path_floor(
    motions: &[PlannedMotion],
    paths: &[(
        crate::geometry::Point,
        crate::geometry::Point,
        Interpolation,
    )],
    top: f64,
) -> f64 {
    paths
        .iter()
        .map(|&(a, b, interpolation)| {
            motions
                .iter()
                .filter(|m| {
                    m.effect == MotionEffect::MillingSweep
                        && m.start.z == m.end.z
                        && m.interpolation == interpolation
                        && ((m.start.xy() == a && m.end.xy() == b)
                            || (interpolation == Interpolation::LinearFeed
                                && m.start.xy() == b
                                && m.end.xy() == a))
                })
                .fold(top, |z, m| z.min(m.end.z))
        })
        .reduce(f64::max)
        .unwrap_or(top)
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
