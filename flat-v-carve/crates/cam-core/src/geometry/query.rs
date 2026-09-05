//! Cached, independent distances and containment; no polygon offset or Voronoi calls.
use super::{Diagnostic, Point, Region, Result, Segment};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PointLocation {
    Inside,
    Boundary,
    Outside,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Clearance {
    pub location: PointLocation,
    pub distance_mm: f64,
    pub signed_distance_mm: f64,
    /// Floating-point reserve for these normalized-coordinate distance calculations.
    pub numerical_reserve_mm: f64,
}

#[derive(Clone, Debug)]
pub struct BoundaryQuery {
    rings: Vec<Vec<Point>>,
    segments: Vec<Segment>,
    magnitude: f64,
    query_limit: f64,
}

impl BoundaryQuery {
    pub fn new(region: &Region) -> Self {
        let rings = region.rings_mm();
        let magnitude = rings
            .iter()
            .flatten()
            .map(|p| p.x.abs().max(p.y.abs()))
            .fold(1.0, f64::max);
        Self {
            rings,
            segments: region.segments(),
            magnitude,
            query_limit: 4.0 * region.grid().max_coordinate_mm(),
        }
    }
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }
    /// Signed-distance enclosure over a complete axis-aligned rectangle.
    /// Distance to a segment is convex, so its maximum is at a box vertex.
    /// A boundary intersecting/contained in the box forces a mixed-sign bound.
    pub fn box_signed_distance_bounds(&self, min: Point, max: Point) -> Result<(f64, f64)> {
        if min.x > max.x || min.y > max.y {
            return Err(Diagnostic::new("QUERY_BOX", "ordered finite box required"));
        }
        let center = min.lerp(max, 0.5);
        let sample = self.sample(center)?;
        self.sample(min)?;
        self.sample(max)?;
        let corners = [min, Point::new(max.x, min.y), max, Point::new(min.x, max.y)];
        let mut lower = f64::INFINITY;
        let mut upper = f64::INFINITY;
        for &edge in &self.segments {
            let inside = |p: Point| p.x >= min.x && p.x <= max.x && p.y >= min.y && p.y <= max.y;
            let least = if inside(edge.start) || inside(edge.end) {
                0.
            } else {
                (0..4)
                    .map(|i| {
                        segment_distance(
                            edge,
                            Segment {
                                start: corners[i],
                                end: corners[(i + 1) % 4],
                            },
                        )
                    })
                    .fold(f64::INFINITY, f64::min)
            };
            lower = lower.min(least);
            upper = upper.min(corners.iter().map(|&p| edge.distance(p)).fold(0., f64::max));
        }
        let reserve = sample.numerical_reserve_mm * 4.;
        upper += reserve;
        if lower <= reserve {
            return Ok((-upper, upper));
        }
        lower -= reserve;
        Ok(if sample.location == PointLocation::Inside {
            (lower, upper)
        } else {
            (-upper, -lower)
        })
    }
    pub fn sample(&self, p: Point) -> Result<Clearance> {
        if !p.finite() || p.x.abs() > self.query_limit || p.y.abs() > self.query_limit {
            return Err(Diagnostic::new(
                "QUERY_RANGE",
                "distance query must be finite and within four times the shared grid coordinate range",
            ));
        }
        if self.segments.is_empty() {
            return Err(Diagnostic::new(
                "EMPTY_GEOMETRY",
                "empty region has no finite boundary distance",
            ));
        }
        let mut inside = false;
        let mut boundary = false;
        for ring in &self.rings {
            for (&a, &b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                let orientation = orient(a, b, p);
                if orientation == 0.0
                    && p.x >= a.x.min(b.x)
                    && p.x <= a.x.max(b.x)
                    && p.y >= a.y.min(b.y)
                    && p.y <= a.y.max(b.y)
                {
                    boundary = true;
                }
                if (a.y > p.y) != (b.y > p.y) && ((orientation > 0.0) == (b.y > a.y)) {
                    inside = !inside;
                }
            }
        }
        let distance = self
            .segments
            .iter()
            .map(|s| s.distance(p))
            .fold(f64::INFINITY, f64::min);
        let location = if boundary {
            PointLocation::Boundary
        } else if inside {
            PointLocation::Inside
        } else {
            PointLocation::Outside
        };
        let signed_distance_mm = match location {
            PointLocation::Inside => distance,
            PointLocation::Outside => -distance,
            PointLocation::Boundary => 0.0,
        };
        Ok(Clearance {
            location,
            distance_mm: distance,
            signed_distance_mm,
            numerical_reserve_mm: 128.0
                * f64::EPSILON
                * self.magnitude.max(p.x.abs()).max(p.y.abs()),
        })
    }
    /// Minimum clearance over a whole segment, including intersections between endpoints.
    pub fn segment_distance_mm(&self, segment: Segment) -> Result<f64> {
        self.sample(segment.start)?;
        self.sample(segment.end)?;
        Ok(self
            .segments
            .iter()
            .map(|&edge| segment_distance(segment, edge))
            .fold(f64::INFINITY, f64::min))
    }

    /// Lower margin for a linearly changing disk along a linear XY move.
    /// Split at changes of closest boundary feature, then minimize each squared
    /// distance-minus-radius quadratic over the entire parameter interval.
    pub fn variable_radius_margin_mm(&self, segment: Segment, r0: f64, r1: f64) -> Result<f64> {
        if !r0.is_finite() || !r1.is_finite() || r0 < 0. || r1 < 0. {
            return Err(Diagnostic::new(
                "SWEEP_RADIUS",
                "finite nonnegative radii required",
            ));
        }
        for p in [segment.start, segment.end] {
            if self.sample(p)?.location != PointLocation::Inside {
                return Err(Diagnostic::new(
                    "SWEEP_OUTSIDE",
                    "cutting centers must remain inside the normalized target",
                ));
            }
        }
        let vx = segment.end.x - segment.start.x;
        let vy = segment.end.y - segment.start.y;
        let dr = r1 - r0;
        let mut margin = f64::INFINITY;
        for edge in &self.segments {
            let wx = edge.end.x - edge.start.x;
            let wy = edge.end.y - edge.start.y;
            let ww = wx * wx + wy * wy;
            let cx = segment.start.x - edge.start.x;
            let cy = segment.start.y - edge.start.y;
            let u0 = (cx * wx + cy * wy) / ww;
            let du = (vx * wx + vy * wy) / ww;
            let mut splits = vec![0., 1.];
            if du != 0. {
                for u in [0., 1.] {
                    let t = (u - u0) / du;
                    if t > 0. && t < 1. {
                        splits.push(t);
                    }
                }
            }
            splits.sort_by(f64::total_cmp);
            for pair in splits.windows(2) {
                let [lo, hi] = [pair[0], pair[1]];
                let u = u0 + du * (lo + hi) / 2.;
                let (ax, ay, bx, by) = if u <= 0. {
                    (cx, cy, vx, vy)
                } else if u >= 1. {
                    (cx - wx, cy - wy, vx, vy)
                } else {
                    (cx - u0 * wx, cy - u0 * wy, vx - du * wx, vy - du * wy)
                };
                let aa = bx * bx + by * by - dr * dr;
                let bb = 2. * (ax * bx + ay * by - r0 * dr);
                let cc = ax * ax + ay * ay - r0 * r0;
                let at = |t: f64| (aa * t + bb) * t + cc;
                let mut least = at(lo).min(at(hi));
                if aa > 0. {
                    let t = -bb / (2. * aa);
                    if t > lo && t < hi {
                        least = least.min(at(t));
                    }
                }
                let magnitude =
                    ax.abs() + ay.abs() + bx.abs() + by.abs() + r0 + r1 + self.magnitude;
                let reserve = 256. * f64::EPSILON * magnitude * magnitude;
                let denom = (ax + bx * lo)
                    .hypot(ay + by * lo)
                    .max((ax + bx * hi).hypot(ay + by * hi))
                    + r0.max(r1);
                if !least.is_finite() || !reserve.is_finite() {
                    return Err(Diagnostic::new(
                        "SWEEP_RANGE",
                        "sweep arithmetic exceeds its finite range",
                    ));
                }
                // A negative numerator is a rejection; its quotient is only a diagnostic.
                margin = margin.min((least - reserve) / denom.max(f64::MIN_POSITIVE));
            }
        }
        Ok(margin)
    }
}

fn orient(a: Point, b: Point, c: Point) -> f64 {
    let coord = |p: Point| robust::Coord { x: p.x, y: p.y };
    robust::orient2d(coord(a), coord(b), coord(c))
}
fn segment_distance(a: Segment, b: Segment) -> f64 {
    let opposite = |x: f64, y: f64| (x >= 0.0 && y <= 0.0) || (x <= 0.0 && y >= 0.0);
    let boxes_overlap = a.start.x.min(a.end.x) <= b.start.x.max(b.end.x)
        && b.start.x.min(b.end.x) <= a.start.x.max(a.end.x)
        && a.start.y.min(a.end.y) <= b.start.y.max(b.end.y)
        && b.start.y.min(b.end.y) <= a.start.y.max(a.end.y);
    if boxes_overlap
        && opposite(
            orient(a.start, a.end, b.start),
            orient(a.start, a.end, b.end),
        )
        && opposite(
            orient(b.start, b.end, a.start),
            orient(b.start, b.end, a.end),
        )
    {
        return 0.0;
    }
    a.distance(b.start)
        .min(a.distance(b.end))
        .min(b.distance(a.start))
        .min(b.distance(a.end))
}
