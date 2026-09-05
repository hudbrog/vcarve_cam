//! Stock derived only from recorded cutting motions. Slice polygons supplement exact point queries.
use crate::{
    geometry::{BooleanOp, Diagnostic, Grid, Point, Region, Result},
    motion::Motion,
};
use serde::Serialize;

fn validate_motions(motions: &[Motion]) -> Result<()> {
    if motions.iter().any(|m| !m.start.finite() || !m.end.finite()) {
        return Err(
            Diagnostic::new("STOCK_MOTION", "stock input contains a nonfinite motion")
                .at_stage("stock"),
        );
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
pub struct SliceRemoval {
    pub depth_mm: f64,
    pub lower: Region,
    pub upper: Region,
    pub contributing_motion_ids: Vec<usize>,
    pub capsule_radial_error_mm: f64,
}
#[derive(Clone, Debug, Serialize)]
pub struct CapsuleBounds {
    pub lower: Region,
    pub upper: Region,
    pub radial_error_mm: f64,
}

pub fn capsule_bounds(grid: Grid, a: Point, b: Point, radius: f64) -> Result<CapsuleBounds> {
    let snap = grid.snap_bound_mm();
    let tolerance = grid.tolerance_mm();
    if !radius.is_finite() || radius <= 4. * snap {
        return Err(Diagnostic::new(
            "STOCK_PRECISION",
            "tool radius is too small for bounded sweep polygons",
        )
        .at_stage("stock"));
    }
    let n = (std::f64::consts::PI * (2. * radius / tolerance).sqrt())
        .ceil()
        .max(16.);
    if n > 1024. {
        return Err(Diagnostic::new(
            "STOCK_RESOURCE_LIMIT",
            "capsule needs more than 1024 circle sides",
        )
        .at_stage("stock"));
    }
    let n = n as usize;
    let cosine = (std::f64::consts::PI / n as f64).cos();
    let inner = radius - 2. * snap;
    let outer = (radius + 2. * snap) / cosine;
    let hull = |r: f64| {
        let mut points = Vec::with_capacity(2 * n);
        for p in [a, b] {
            for i in 0..n {
                let t = std::f64::consts::TAU * i as f64 / n as f64;
                points.push(Point::new(p.x + r * t.cos(), p.y + r * t.sin()));
            }
        }
        Region::convex_hull(grid, &points)
    };
    Ok(CapsuleBounds {
        lower: hull(inner)?,
        upper: hull(outer)?,
        radial_error_mm: (radius - inner * cosine + snap).max(outer + snap - radius),
    })
}

pub fn removal_at_slice(
    grid: Grid,
    motions: &[Motion],
    radius: f64,
    depth: f64,
) -> Result<SliceRemoval> {
    validate_motions(motions)?;
    if !radius.is_finite() || radius <= 0. {
        return Err(
            Diagnostic::new("STOCK_PRECISION", "positive finite tool radius required")
                .at_stage("stock"),
        );
    }
    if !depth.is_finite() || depth < 0. {
        return Err(
            Diagnostic::new("STOCK_DEPTH", "slice depth must be finite and nonnegative")
                .at_stage("stock"),
        );
    }
    let mut lower = Region::from_rings(grid, &[])?;
    let mut upper = lower.clone();
    let mut ids = vec![];
    let mut error: f64 = 0.;
    for m in motions {
        if let Some((a, b)) = m.at_depth(depth) {
            let capsule = capsule_bounds(grid, a, b, radius)?;
            error = error.max(capsule.radial_error_mm);
            lower = lower.boolean(BooleanOp::Union, &capsule.lower)?;
            upper = upper.boolean(BooleanOp::Union, &capsule.upper)?;
            ids.push(m.id);
        }
    }
    Ok(SliceRemoval {
        depth_mm: depth,
        lower,
        upper,
        contributing_motion_ids: ids,
        capsule_radial_error_mm: error,
    })
}

/// Exact linear-motion/cylindrical-tool point query (up to floating-point arithmetic).
/// Solve the interval where XY lies within the disk, then maximize the linear depth.
pub fn removed_depth_at(motions: &[Motion], radius: f64, p: Point) -> Result<f64> {
    validate_motions(motions)?;
    if !p.finite() || !radius.is_finite() || radius <= 0. {
        return Err(
            Diagnostic::new("STOCK_QUERY", "positive radius and finite point required")
                .at_stage("stock"),
        );
    }
    let mut removed: f64 = 0.;
    for m in motions.iter().filter(|m| m.kind.cutting()) {
        let a = m.start.xy();
        let b = m.end.xy();
        let vx = b.x - a.x;
        let vy = b.y - a.y;
        let dx = a.x - p.x;
        let dy = a.y - p.y;
        let aa = vx * vx + vy * vy;
        if !aa.is_finite() || !(dx * dx + dy * dy + radius * radius).is_finite() {
            return Err(Diagnostic::new(
                "STOCK_QUERY_RANGE",
                "query arithmetic exceeds the supported range",
            )
            .at_stage("stock"));
        }
        if aa == 0. {
            if dx.hypot(dy) <= radius {
                removed = removed.max(m.start.depth()).max(m.end.depth());
            }
            continue;
        }
        let center = -(dx * vx + dy * vy) / aa;
        let closest = Point::new(dx + center * vx, dy + center * vy);
        let discriminant = radius * radius - closest.x * closest.x - closest.y * closest.y;
        if discriminant < 0. {
            continue;
        }
        let half = (discriminant / aa).sqrt();
        let lo = (center - half).max(0.);
        let hi = (center + half).min(1.);
        if lo <= hi {
            removed = removed
                .max(m.start.lerp(m.end, lo).depth())
                .max(m.start.lerp(m.end, hi).depth());
        }
    }
    Ok(removed)
}

/// Convex hull of two disks with different radii. For a tapered apex, move the
/// inner endpoint a bounded distance into the sweep before snapping its disk.
pub fn variable_capsule_bounds(
    grid: Grid,
    a: Point,
    ra: f64,
    b: Point,
    rb: f64,
) -> Result<CapsuleBounds> {
    if !ra.is_finite() || !rb.is_finite() || ra < 0. || rb < 0. {
        return Err(
            Diagnostic::new("STOCK_RADIUS", "finite nonnegative radii required").at_stage("stock"),
        );
    }
    let r = ra.max(rb);
    let snap = grid.snap_bound_mm();
    let n = (std::f64::consts::PI * (2. * r / grid.tolerance_mm()).sqrt())
        .ceil()
        .max(16.);
    if n > 1024. {
        return Err(Diagnostic::new(
            "STOCK_RESOURCE_LIMIT",
            "variable capsule requires more than 1024 circle sides",
        )
        .at_stage("stock"));
    }
    let n = n as usize;
    let cosine = (std::f64::consts::PI / n as f64).cos();
    let trim = |p: Point, radius: f64, q: Point, other: f64| {
        if radius <= 2. * snap && other > 4. * snap {
            let t = (4. * snap - radius) / (other - radius);
            (
                p.lerp(q, t),
                4. * snap,
                t * (p.distance(q) + (other - radius).abs()),
            )
        } else {
            (p, radius, 0.)
        }
    };
    let ta = trim(a, ra, b, rb);
    let tb = trim(b, rb, a, ra);
    let hull = |upper: bool| -> Result<Region> {
        let mut points = vec![];
        for (p, radius) in if upper {
            [(a, ra), (b, rb)]
        } else {
            [(ta.0, ta.1), (tb.0, tb.1)]
        } {
            if !upper && radius <= 2. * snap {
                continue;
            }
            let radius = if upper {
                (radius + 2. * snap) / cosine
            } else {
                radius - 2. * snap
            };
            for i in 0..n {
                let t = std::f64::consts::TAU * i as f64 / n as f64;
                points.push(Point::new(p.x + radius * t.cos(), p.y + radius * t.sin()));
            }
        }
        if points.is_empty() {
            Region::from_rings(grid, &[])
        } else {
            Region::convex_hull(grid, &points)
        }
    };
    Ok(CapsuleBounds {
        lower: hull(false)?,
        upper: hull(true)?,
        radial_error_mm: (r - (r - 2. * snap).max(0.) * cosine + snap)
            .max((r + 2. * snap) / cosine + snap - r)
            + ta.2.max(tb.2),
    })
}

pub fn vbit_removal_at_slice(
    grid: Grid,
    motions: &[Motion],
    tool: &crate::model::VBit,
    depth: f64,
) -> Result<SliceRemoval> {
    validate_motions(motions)?;
    if motions.iter().filter(|m| m.kind.cutting()).any(|m| {
        m.start.depth() > tool.cutting_height().mm() || m.end.depth() > tool.cutting_height().mm()
    }) {
        return Err(Diagnostic::new(
            "STOCK_CUTTING_HEIGHT",
            "recorded depth exceeds the modeled V-bit cutting height",
        )
        .at_stage("stock"));
    }
    if !depth.is_finite() || depth < 0. {
        return Err(
            Diagnostic::new("STOCK_DEPTH", "finite nonnegative slice depth required")
                .at_stage("stock"),
        );
    }
    let mut lower = Region::from_rings(grid, &[])?;
    let mut upper = lower.clone();
    let mut ids = vec![];
    let mut error: f64 = 0.;
    for m in motions.iter().filter(|m| m.kind.cutting()) {
        let da = m.start.depth();
        let db = m.end.depth();
        if da < depth && db < depth {
            continue;
        }
        let (lo, hi) = if da >= depth && db >= depth {
            (0., 1.)
        } else if db > da {
            ((depth - da) / (db - da), 1.)
        } else {
            (0., (depth - da) / (db - da))
        };
        let a = m.start.lerp(m.end, lo);
        let b = m.start.lerp(m.end, hi);
        let radius = |d: f64| tool.tip_radius().mm() + (d - depth).max(0.) * tool.angle().slope();
        let c =
            variable_capsule_bounds(grid, a.xy(), radius(a.depth()), b.xy(), radius(b.depth()))?;
        lower = lower.boolean(BooleanOp::Union, &c.lower)?;
        upper = upper.boolean(BooleanOp::Union, &c.upper)?;
        error = error.max(c.radial_error_mm);
        ids.push(m.id);
    }
    Ok(SliceRemoval {
        depth_mm: depth,
        lower,
        upper,
        contributing_motion_ids: ids,
        capsule_radial_error_mm: error,
    })
}

/// Maximize the concave linear-depth-minus-cone-height function analytically.
/// Endpoint, flat-tip interval, closest-point, and flank stationary candidates
/// suffice; there is no sampling of the motion parameter.
pub fn vbit_removed_depth_at(
    motions: &[Motion],
    tool: &crate::model::VBit,
    p: Point,
) -> Result<f64> {
    validate_motions(motions)?;
    if motions.iter().filter(|m| m.kind.cutting()).any(|m| {
        m.start.depth() > tool.cutting_height().mm() || m.end.depth() > tool.cutting_height().mm()
    }) {
        return Err(Diagnostic::new(
            "STOCK_CUTTING_HEIGHT",
            "recorded depth exceeds the modeled V-bit cutting height",
        )
        .at_stage("stock"));
    }
    if !p.finite() {
        return Err(
            Diagnostic::new("STOCK_QUERY", "finite query point required").at_stage("stock"),
        );
    }
    let mut removed: f64 = 0.;
    let slope = tool.angle().slope();
    let tip = tool.tip_radius().mm();
    for m in motions.iter().filter(|m| m.kind.cutting()) {
        let a = m.start.xy();
        let b = m.end.xy();
        let vx = b.x - a.x;
        let vy = b.y - a.y;
        let length = vx.hypot(vy);
        let evaluate = |t: f64| {
            let q = m.start.lerp(m.end, t);
            (q.depth() - ((q.xy().distance(p) - tip).max(0.) / slope)).max(0.)
        };
        removed = removed.max(evaluate(0.)).max(evaluate(1.));
        if length == 0. {
            continue;
        }
        let dx = a.x - p.x;
        let dy = a.y - p.y;
        let along = (dx * vx + dy * vy) / length;
        let perpendicular = (dx * vy - dy * vx).abs() / length;
        if !along.is_finite() || !perpendicular.is_finite() {
            return Err(Diagnostic::new(
                "STOCK_QUERY_RANGE",
                "point query exceeds supported arithmetic range",
            )
            .at_stage("stock"));
        }
        let mut candidates = [-along / length, f64::NAN, f64::NAN, f64::NAN];
        if tip >= perpendicular {
            let half = ((tip - perpendicular) * (tip + perpendicular)).sqrt();
            candidates[1] = (-along - half) / length;
            candidates[2] = (-along + half) / length;
        }
        let k = slope * (m.end.depth() - m.start.depth()) / length;
        if k.abs() < 1. {
            let u = k * perpendicular / (1. - k * k).sqrt();
            candidates[3] = (u - along) / length;
        }
        for t in candidates {
            if t > 0. && t < 1. {
                removed = removed.max(evaluate(t));
            }
        }
    }
    Ok(removed)
}

/// Conservative air proof against one recorded endmill sweep. Centers may be
/// in cleared space; this tests containment of the entire V-bit footprint.
pub fn vbit_air_in_endmill(
    motion: &Motion,
    vbit: &crate::model::VBit,
    endmill_moves: &[Motion],
    radius: f64,
    reserve: f64,
) -> bool {
    if !radius.is_finite()
        || radius <= 0.
        || !reserve.is_finite()
        || reserve < 0.
        || !motion.start.finite()
        || !motion.end.finite()
        || motion.start.depth().max(motion.end.depth()) > vbit.cutting_height().mm()
    {
        return false;
    }
    let r = vbit.tip_radius().mm()
        + motion.start.depth().max(motion.end.depth()) * vbit.angle().slope()
        + reserve;
    if r >= radius {
        return false;
    }
    endmill_moves
        .iter()
        .filter(|m| m.kind.cutting())
        .any(|prior| {
            let depth = if prior.start.xy() == prior.end.xy() {
                prior.start.depth().max(prior.end.depth())
            } else {
                prior.start.depth().min(prior.end.depth())
            };
            let segment = crate::geometry::Segment {
                start: prior.start.xy(),
                end: prior.end.xy(),
            };
            depth >= motion.start.depth().max(motion.end.depth())
                && [motion.start.xy(), motion.end.xy()]
                    .iter()
                    .all(|&p| segment.distance(p) + r <= radius)
        })
}
