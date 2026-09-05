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
