use super::{Diagnostic, Point, Result};
use serde::Serialize;

/// Conservative common domain for i32 Voronoi and i64 Clipper input.
pub const MAX_GRID_COORD: i64 = 1_000_000_000;
pub(crate) const MAX_EDGES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct GridPoint {
    pub x: i64,
    pub y: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Grid {
    ticks_per_mm: f64,
    geometry_tolerance_mm: f64,
}

impl Grid {
    /// Select the smallest decimal scale assigning at most 1/8 of tolerance to snapping.
    pub fn new(tolerance_mm: f64, max_abs_coordinate_mm: f64) -> Result<Self> {
        if !tolerance_mm.is_finite()
            || tolerance_mm < 1e-9
            || !max_abs_coordinate_mm.is_finite()
            || max_abs_coordinate_mm < 0.0
        {
            return Err(Diagnostic::new(
                "INVALID_PRECISION",
                "finite tolerance >= 1e-9 mm and nonnegative coordinate extent required",
            ));
        }
        let required = 4.0 * std::f64::consts::SQRT_2 / tolerance_mm;
        let scale = 10f64.powf(required.log10().ceil()).max(1.0);
        Self::with_scale(tolerance_mm, max_abs_coordinate_mm, scale)
    }

    pub fn with_scale(
        tolerance_mm: f64,
        max_abs_coordinate_mm: f64,
        ticks_per_mm: f64,
    ) -> Result<Self> {
        if !tolerance_mm.is_finite()
            || tolerance_mm < 1e-9
            || !ticks_per_mm.is_finite()
            || !(1.0..=1e12).contains(&ticks_per_mm)
            || !max_abs_coordinate_mm.is_finite()
            || max_abs_coordinate_mm < 0.0
        {
            return Err(Diagnostic::new(
                "INVALID_PRECISION",
                "invalid precision parameters",
            ));
        }
        let grid = Self {
            ticks_per_mm,
            geometry_tolerance_mm: tolerance_mm,
        };
        if grid.snap_bound_mm() > tolerance_mm / 8.0
            || max_abs_coordinate_mm * ticks_per_mm > MAX_GRID_COORD as f64
        {
            return Err(Diagnostic::new(
                "PRECISION_RANGE",
                "requested tolerance and coordinate range do not fit the shared integer grid",
            ));
        }
        Ok(grid)
    }

    pub fn scale(self) -> f64 {
        self.ticks_per_mm
    }
    pub fn tolerance_mm(self) -> f64 {
        self.geometry_tolerance_mm
    }
    pub fn snap_bound_mm(self) -> f64 {
        std::f64::consts::SQRT_2 / (2.0 * self.ticks_per_mm)
    }
    pub fn arc_tolerance_mm(self) -> f64 {
        self.geometry_tolerance_mm / 4.0
    }
    pub fn max_coordinate_mm(self) -> f64 {
        MAX_GRID_COORD as f64 / self.ticks_per_mm
    }

    pub fn quantize(self, p: Point) -> Result<GridPoint> {
        if !p.finite() {
            return Err(Diagnostic::new(
                "NONFINITE_COORDINATE",
                "point must be finite",
            ));
        }
        let x = p.x * self.ticks_per_mm;
        let y = p.y * self.ticks_per_mm;
        if x.abs() > MAX_GRID_COORD as f64 || y.abs() > MAX_GRID_COORD as f64 {
            return Err(Diagnostic::new(
                "COORDINATE_RANGE",
                "point exceeds the shared integer range",
            ));
        }
        Ok(GridPoint {
            x: x.round() as i64,
            y: y.round() as i64,
        })
    }

    pub fn point(self, p: GridPoint) -> Point {
        Point::new(
            p.x as f64 / self.ticks_per_mm,
            p.y as f64 / self.ticks_per_mm,
        )
    }
}

pub(crate) fn cross(a: GridPoint, b: GridPoint, c: GridPoint) -> i128 {
    (b.x - a.x) as i128 * (c.y - a.y) as i128 - (b.y - a.y) as i128 * (c.x - a.x) as i128
}

pub(crate) fn twice_area(p: &[GridPoint]) -> i128 {
    p.iter()
        .zip(p.iter().cycle().skip(1))
        .map(|(a, b)| a.x as i128 * b.y as i128 - a.y as i128 * b.x as i128)
        .sum()
}

pub(crate) fn on_segment(p: GridPoint, a: GridPoint, b: GridPoint) -> bool {
    cross(a, b, p) == 0
        && (a.x.min(b.x)..=a.x.max(b.x)).contains(&p.x)
        && (a.y.min(b.y)..=a.y.max(b.y)).contains(&p.y)
}

pub(crate) fn intersects(a: GridPoint, b: GridPoint, c: GridPoint, d: GridPoint) -> bool {
    let (ab_c, ab_d, cd_a, cd_b) = (
        cross(a, b, c),
        cross(a, b, d),
        cross(c, d, a),
        cross(c, d, b),
    );
    (ab_c.signum() * ab_d.signum() < 0 && cd_a.signum() * cd_b.signum() < 0)
        || on_segment(a, c, d)
        || on_segment(b, c, d)
        || on_segment(c, a, b)
        || on_segment(d, a, b)
}

pub(crate) fn inside(p: GridPoint, ring: &[GridPoint]) -> bool {
    let mut result = false;
    for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        if (a.y > p.y) != (b.y > p.y) && ((cross(a, b, p) > 0) == (b.y > a.y)) {
            result = !result;
        }
    }
    result
}
