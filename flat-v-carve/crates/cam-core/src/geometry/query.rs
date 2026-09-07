//! Cached, independent distances and containment; no polygon offset or Voronoi calls.
use super::spatial::{Aabb, SpatialIndex};
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
    segments: Vec<Segment>,
    index: SpatialIndex,
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
        let segments = region.segments();
        let index = SpatialIndex::new(segments.iter().map(|s| Aabb::new(s.start, s.end)).collect());
        Self {
            segments,
            index,
            magnitude,
            query_limit: 4.0 * region.grid().max_coordinate_mm(),
        }
    }
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }
    /// Visit boundary edges whose boxes intersect a caller's conservative search box.
    /// Exact geometric predicates, including any proof for omitted edges, remain
    /// the caller's responsibility.
    pub(crate) fn visit_segments(
        &self,
        min: Point,
        max: Point,
        mut visitor: impl FnMut(&Segment),
    ) -> Result<()> {
        self.index.visit(Aabb::new(min, max), |i| {
            visitor(&self.segments[i]);
            Ok(())
        })
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
        self.box_bounds_with_center_sample(min, max, sample)
    }
    /// Reuse an exact sample of min.lerp(max, 0.5), taken from this same query.
    pub(crate) fn box_bounds_with_center_sample(
        &self,
        min: Point,
        max: Point,
        sample: Clearance,
    ) -> Result<(f64, f64)> {
        if min.x > max.x || min.y > max.y {
            return Err(Diagnostic::new("QUERY_BOX", "ordered finite box required"));
        }
        self.check_point(min)?;
        self.check_point(max)?;
        let corners = [min, Point::new(max.x, min.y), max, Point::new(min.x, max.y)];
        let bounds = Aabb::new(min, max);
        let mut lower = self.index.minimum(bounds, |i| {
            let edge = self.segments[i];
            let inside = |p: Point| p.x >= min.x && p.x <= max.x && p.y >= min.y && p.y <= max.y;
            if inside(edge.start) || inside(edge.end) {
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
            }
        });
        let mut upper = self.index.minimum(bounds, |i| {
            corners
                .iter()
                .map(|&p| self.segments[i].distance(p))
                .fold(0., f64::max)
        });
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
    fn check_point(&self, p: Point) -> Result<()> {
        if !p.finite() || p.x.abs() > self.query_limit || p.y.abs() > self.query_limit {
            return Err(Diagnostic::new(
                "QUERY_RANGE",
                "distance query must be finite and within four times the shared grid coordinate range",
            ));
        }
        Ok(())
    }
    fn check_sample_point(&self, p: Point) -> Result<()> {
        self.check_point(p)?;
        if self.segments.is_empty() {
            return Err(Diagnostic::new(
                "EMPTY_GEOMETRY",
                "empty region has no finite boundary distance",
            ));
        }
        Ok(())
    }
    /// Containment without an unused nearest-boundary distance search.
    pub fn location(&self, p: Point) -> Result<PointLocation> {
        self.check_sample_point(p)?;
        let mut inside = false;
        let mut boundary = false;
        self.index
            .visit(Aabb::new(p, Point::new(self.query_limit, p.y)), |i| {
                let Segment { start: a, end: b } = self.segments[i];
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
                Ok(())
            })?;
        Ok(if boundary {
            PointLocation::Boundary
        } else if inside {
            PointLocation::Inside
        } else {
            PointLocation::Outside
        })
    }
    pub fn sample(&self, p: Point) -> Result<Clearance> {
        self.check_sample_point(p)?;
        let mut nearest = 0;
        let mut best = f64::INFINITY;
        let distance = self.index.minimum(Aabb::new(p, p), |i| {
            let distance = self.segments[i].distance(p);
            if distance < best {
                best = distance;
                nearest = i;
            }
            distance
        });
        let Segment { start: a, end: b } = self.segments[nearest];
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let length2 = dx * dx + dy * dy;
        let projection = (p.x - a.x) * dx + (p.y - a.y) * dy;
        let reserve = 128.
            * f64::EPSILON
            * self.magnitude.max(p.x.abs()).max(p.y.abs())
            * (dx.abs() + dy.abs());
        // Normalized outer boundaries are CCW and holes are CW: material is
        // always on the left. A nearest point strictly inside an edge therefore
        // determines the sign. Vertex and numerically ambiguous projections
        // retain the independent ray test, including touching components.
        let location = if projection > reserve && projection < length2 - reserve {
            let orientation = orient(a, b, p);
            if orientation > 0. {
                PointLocation::Inside
            } else if orientation < 0. {
                PointLocation::Outside
            } else {
                PointLocation::Boundary
            }
        } else {
            self.location(p)?
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
        self.check_sample_point(segment.start)?;
        self.check_point(segment.end)?;
        Ok(self
            .index
            .minimum(Aabb::new(segment.start, segment.end), |i| {
                segment_distance(segment, self.segments[i])
            }))
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
            if self.location(p)? != PointLocation::Inside {
                return Err(Diagnostic::new(
                    "SWEEP_OUTSIDE",
                    "cutting centers must remain inside the normalized target",
                ));
            }
        }
        let vx = segment.end.x - segment.start.x;
        let vy = segment.end.y - segment.start.y;
        let dr = r1 - r0;
        // An edge outside this expanded motion box is at least r_max + 1 mm
        // from every center. Its clearance therefore exceeds 1 mm. Round the
        // search outward; retain 1 mm as a conservative bound for skipped edges.
        // Nearby edges still use the same continuous quadratic proof below.
        let magnitude = self
            .magnitude
            .max(segment.start.x.abs())
            .max(segment.start.y.abs())
            .max(segment.end.x.abs())
            .max(segment.end.y.abs());
        let padding = r0.max(r1) + 1. + 1024. * f64::EPSILON * (magnitude + r0 + r1 + 1.);
        let bounds = Aabb::new(segment.start, segment.end);
        let bounds = Aabb::new(
            Point::new(bounds.min.x - padding, bounds.min.y - padding),
            Point::new(bounds.max.x + padding, bounds.max.y + padding),
        );
        if !bounds.min.finite() || !bounds.max.finite() {
            return Err(Diagnostic::new(
                "SWEEP_RANGE",
                "sweep bounds exceed finite range",
            ));
        }
        let mut margin: f64 = 1.;
        self.index.visit(bounds, |i| {
            let edge = &self.segments[i];
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
            Ok(())
        })?;
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

#[cfg(test)]
mod sign_tests {
    use super::*;
    use crate::geometry::{BooleanOp, Grid};

    #[test]
    fn nearest_edge_sign_matches_ray_crossings_and_full_scan_distances() {
        let p = Point::new;
        let rectangle = |x, y, w, h| vec![p(x, y), p(x + w, y), p(x + w, y + h), p(x, y + h)];
        for scale in [1., 1000.] {
            let grid = Grid::new(0.001 * scale, 100. * scale).unwrap();
            let ring = |v: Vec<Point>| {
                v.into_iter()
                    .map(|p| Point::new(p.x * scale, p.y * scale))
                    .collect()
            };
            let region = Region::from_rings(
                grid,
                &[
                    ring(rectangle(0., 0., 20., 20.)),
                    ring(rectangle(4., 4., 12., 12.)),
                    ring(rectangle(7., 7., 6., 6.)),
                    ring(vec![
                        p(25., 0.),
                        p(40., 0.),
                        p(40., 20.),
                        p(35., 20.),
                        p(35., 5.),
                        p(30., 5.),
                        p(30., 20.),
                        p(25., 20.),
                    ]),
                ],
            )
            .unwrap();
            // The extra square touches the first at one vertex after union.
            let touching = Region::from_rings(grid, &[ring(rectangle(-5., -5., 5., 5.))]).unwrap();
            let region = region.boolean(BooleanOp::Union, &touching).unwrap();
            let query = BoundaryQuery::new(&region);
            let mut points = vec![];
            for x in -24..=170 {
                for y in -24..=90 {
                    points.push(p(x as f64 * scale / 4., y as f64 * scale / 4.));
                }
            }
            for edge in region.segments() {
                for t in [0., f64::EPSILON, 0.5, 1. - f64::EPSILON, 1.] {
                    let at = edge.start.lerp(edge.end, t);
                    for epsilon in [-1e-12, 0., 1e-12] {
                        points.push(p(at.x + epsilon * scale, at.y - epsilon * scale));
                    }
                }
            }
            for p in points {
                let sample = query.sample(p).unwrap();
                assert_eq!(sample.location, query.location(p).unwrap(), "{p:?}");
                let distance = query
                    .segments()
                    .iter()
                    .map(|s| s.distance(p))
                    .fold(f64::INFINITY, f64::min);
                assert_eq!(sample.distance_mm, distance, "{p:?}");
            }
        }
    }
}
