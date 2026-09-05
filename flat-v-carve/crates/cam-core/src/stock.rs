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
