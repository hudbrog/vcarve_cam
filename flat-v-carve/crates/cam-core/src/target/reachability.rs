use super::{Target, error};
use crate::{
    geometry::{Point, Result},
    model::VBit,
};
use serde::Serialize;
use std::{cmp::Ordering, collections::BinaryHeap};

#[derive(Clone, Copy, Debug)]
pub struct ReachabilityOptions {
    pub depth_tolerance_mm: f64,
    pub max_cells: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachabilityStatus {
    Resolved,
    ResourceLimit,
    NumericalLimit,
}

#[derive(Clone, Debug, Serialize)]
pub struct Reachability {
    pub nominal_depth_mm: f64,
    pub reachable_depth_lower_mm: f64,
    pub reachable_depth_upper_mm: f64,
    pub unavoidable_residual_lower_mm: f64,
    pub unavoidable_residual_upper_mm: f64,
    pub status: ReachabilityStatus,
    pub evaluated_cells: usize,
    pub best_tip_center: Point,
    pub input_depth_snap_bound_mm: f64,
}

#[derive(Clone, Copy, Debug)]
struct Cell {
    center: Point,
    half: f64,
    upper: f64,
    id: usize,
}
impl PartialEq for Cell {
    fn eq(&self, other: &Self) -> bool {
        self.upper == other.upper && self.id == other.id
    }
}
impl Eq for Cell {}
impl PartialOrd for Cell {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Cell {
    fn cmp(&self, other: &Self) -> Ordering {
        self.upper
            .total_cmp(&other.upper)
            .then_with(|| other.id.cmp(&self.id))
    }
}

impl Target {
    /// Best achievable removal at a location, allowing the tool center to move.
    /// M = max signed-clearance(q) for |q-p| <= tip_radius. Then d = (M-r)/tan(alpha).
    /// A 1-Lipschitz branch-and-bound encloses M independently of polygon offsets.
    pub fn vbit_reachability(
        &self,
        tool: &VBit,
        p: Point,
        options: ReachabilityOptions,
    ) -> Result<Reachability> {
        self.validate_vbit(tool)?;
        if !options.depth_tolerance_mm.is_finite()
            || options.depth_tolerance_mm <= 0.0
            || options.max_cells == 0
            || options.max_cells > 1_000_000
        {
            return Err(error(
                "INVALID_REACHABILITY_OPTIONS",
                "positive finite depth tolerance and 1..=1000000 cells required",
            ));
        }
        let nominal = self.nominal_depth(p)?.mm();
        let radius = tool.tip_radius().mm();
        if radius > self.region.grid().max_coordinate_mm() {
            return Err(error(
                "CENTER_RANGE",
                "tip radius exceeds supported coordinate range",
            ));
        }
        let slope = self.angle.slope();
        let at_p = self.boundary.sample(p)?;
        let depth_uncertainty = if nominal == 0.0 {
            0.0
        } else {
            (at_p.numerical_reserve_mm / slope).min(self.depth_cap.mm())
        };
        let make_result = |lower: f64, upper: f64, status, evaluated_cells, best_tip_center| {
            let lower = lower.max(0.0).min(nominal);
            let upper = upper.max(lower).min(nominal);
            Reachability {
                nominal_depth_mm: nominal,
                reachable_depth_lower_mm: lower,
                reachable_depth_upper_mm: upper,
                unavoidable_residual_lower_mm: (nominal - upper - depth_uncertainty).max(0.0),
                unavoidable_residual_upper_mm: (nominal - lower + depth_uncertainty)
                    .min(self.depth_cap.mm())
                    .max(0.0),
                status,
                evaluated_cells,
                best_tip_center,
                input_depth_snap_bound_mm: (self.region.grid().snap_bound_mm() / slope)
                    .min(self.depth_cap.mm()),
            }
        };
        if nominal == 0.0 {
            return Ok(make_result(0.0, 0.0, ReachabilityStatus::Resolved, 0, p));
        }
        if radius == 0.0 {
            return Ok(make_result(
                (nominal - depth_uncertainty).max(0.0),
                nominal,
                if depth_uncertainty <= options.depth_tolerance_mm {
                    ReachabilityStatus::Resolved
                } else {
                    ReachabilityStatus::NumericalLimit
                },
                0,
                p,
            ));
        }
        let to_depth = |clearance: f64| ((clearance - radius) / slope).max(0.0).min(nominal);
        let mut best = at_p.signed_distance_mm - at_p.numerical_reserve_mm;
        let mut best_point = p;
        for i in 0..16 {
            let angle = std::f64::consts::TAU * i as f64 / 16.0;
            // Move a few ulps inward, so the lower-bound sample stays within the disk.
            let r = radius * (1.0 - 16.0 * f64::EPSILON);
            let q = Point::new(p.x + r * angle.cos(), p.y + r * angle.sin());
            if q.distance(p) > radius {
                continue;
            }
            let sample = self.boundary.sample(q)?;
            if sample.signed_distance_mm - sample.numerical_reserve_mm > best {
                best = sample.signed_distance_mm - sample.numerical_reserve_mm;
                best_point = q;
            }
        }
        let mut queue = BinaryHeap::new();
        queue.push(Cell {
            center: p,
            half: radius,
            upper: at_p.signed_distance_mm + radius + at_p.numerical_reserve_mm,
            id: 0,
        });
        let mut evaluated = 1;
        loop {
            let upper = queue.peek().map_or(best, |c| c.upper.max(best));
            let (lower_depth, upper_depth) = (to_depth(best), to_depth(upper));
            if upper_depth - lower_depth <= options.depth_tolerance_mm {
                return Ok(make_result(
                    lower_depth,
                    upper_depth,
                    ReachabilityStatus::Resolved,
                    evaluated,
                    best_point,
                ));
            }
            if evaluated + 4 > options.max_cells {
                return Ok(make_result(
                    lower_depth,
                    upper_depth,
                    ReachabilityStatus::ResourceLimit,
                    evaluated,
                    best_point,
                ));
            }
            let Some(cell) = queue.pop() else {
                return Ok(make_result(
                    lower_depth,
                    upper_depth,
                    ReachabilityStatus::NumericalLimit,
                    evaluated,
                    best_point,
                ));
            };
            let half = cell.half / 2.0;
            if half <= f64::EPSILON * cell.center.x.abs().max(cell.center.y.abs()).max(1.0) {
                return Ok(make_result(
                    lower_depth,
                    upper_depth,
                    ReachabilityStatus::NumericalLimit,
                    evaluated,
                    best_point,
                ));
            }
            for (sx, sy) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
                let center = Point::new(cell.center.x + sx * half, cell.center.y + sy * half);
                let closest = Point::new(
                    p.x.clamp(center.x - half, center.x + half),
                    p.y.clamp(center.y - half, center.y + half),
                );
                evaluated += 1;
                if closest.distance(p) > radius {
                    continue;
                }
                let sample = self.boundary.sample(center)?;
                let upper = (sample.signed_distance_mm
                    + half * std::f64::consts::SQRT_2
                    + sample.numerical_reserve_mm)
                    .min(cell.upper);
                let q = if center.distance(p) <= radius {
                    center
                } else {
                    closest
                };
                let value = self.boundary.sample(q)?;
                let lower = value.signed_distance_mm - value.numerical_reserve_mm;
                if lower > best {
                    best = lower;
                    best_point = q;
                }
                if upper > best {
                    queue.push(Cell {
                        center,
                        half,
                        upper,
                        id: evaluated,
                    });
                }
            }
        }
    }
}
