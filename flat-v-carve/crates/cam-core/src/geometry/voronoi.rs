use super::{Diagnostic, Point, Region, Result, Segment, backend};
use boostvoronoi::prelude::{Builder, Cell, SourceCategory};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SiteKind {
    Segment,
    SegmentStart,
    SegmentEnd,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Site {
    /// Index into VoronoiDiagram::source_segments, in normalized ring/edge order.
    pub segment: usize,
    pub kind: SiteKind,
}
impl Site {
    pub fn distance_mm(self, p: Point, sources: &[Segment]) -> f64 {
        let s = sources[self.segment];
        match self.kind {
            SiteKind::Segment => s.distance(p),
            SiteKind::SegmentStart => s.start.distance(p),
            SiteKind::SegmentEnd => s.end.distance(p),
        }
    }
    fn point(self, sources: &[Segment]) -> Option<Point> {
        match self.kind {
            SiteKind::Segment => None,
            SiteKind::SegmentStart => Some(sources[self.segment].start),
            SiteKind::SegmentEnd => Some(sources[self.segment].end),
        }
    }
}

/// A finite linear or quadratic curve, reconstructed from the associated sites.
/// A point/line bisector is a parabola, exactly representable as a quadratic Bezier.
#[derive(Clone, Debug, Serialize)]
pub struct Curve {
    start: Point,
    control: Option<Point>,
    end: Point,
    endpoint_reconstruction_error_mm: f64,
    numerical_reserve_mm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Linearization {
    pub points: Vec<Point>,
    pub chord_bound_mm: f64,
    pub endpoint_reconstruction_error_mm: f64,
    pub numerical_reserve_mm: f64,
    pub total_bound_mm: f64,
}

impl Curve {
    pub fn reconstruction_bound_mm(&self) -> f64 {
        self.endpoint_reconstruction_error_mm + self.numerical_reserve_mm
    }
    pub fn is_curved(&self) -> bool {
        self.control.is_some()
    }
    pub fn evaluate(&self, t: f64) -> Result<Point> {
        if !t.is_finite() || !(0.0..=1.0).contains(&t) {
            return Err(Diagnostic::new(
                "CURVE_PARAMETER",
                "curve parameter must be in [0,1]",
            ));
        }
        Ok(self.at(t))
    }
    fn at(&self, t: f64) -> Point {
        match self.control {
            None => self.start.lerp(self.end, t),
            Some(c) => self.start.lerp(c, t).lerp(c.lerp(self.end, t), t),
        }
    }
    fn full_chord_bound(&self) -> f64 {
        self.control
            .map_or(0.0, |c| c.distance(self.start.lerp(self.end, 0.5)) / 2.0)
    }
    /// Quadratic interpolation error is |P0 - 2P1 + P2|/(4*n^2).
    /// This bounds every parameter value, not just the returned sample points.
    pub fn linearize(&self, tolerance_mm: f64, max_segments: usize) -> Result<Linearization> {
        let reserve = self.endpoint_reconstruction_error_mm + self.numerical_reserve_mm;
        if !tolerance_mm.is_finite() || tolerance_mm <= reserve || max_segments == 0 {
            return Err(Diagnostic::new(
                "CURVE_PRECISION",
                "curve tolerance must exceed numerical and reconstruction reserve",
            ));
        }
        let chord = self.full_chord_bound();
        let count = (chord / (tolerance_mm - reserve)).sqrt().ceil().max(1.0);
        if count > max_segments as f64 || count > 65536.0 {
            return Err(Diagnostic::new(
                "CURVE_LIMIT",
                "curve subdivision exceeds the segment limit",
            ));
        }
        let n = count as usize;
        let points: Vec<_> = (0..=n).map(|i| self.at(i as f64 / n as f64)).collect();
        if points.iter().any(|p| !p.finite()) {
            return Err(Diagnostic::new(
                "CURVE_PRECISION",
                "nonfinite curve evaluation",
            ));
        }
        let chord_bound_mm = chord / (n as f64).powi(2);
        Ok(Linearization {
            points,
            chord_bound_mm,
            endpoint_reconstruction_error_mm: self.endpoint_reconstruction_error_mm,
            numerical_reserve_mm: self.numerical_reserve_mm,
            total_bound_mm: chord_bound_mm + reserve,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct VoronoiEdge {
    pub sites: [Site; 2],
    pub primary: bool,
    pub curved: bool,
    /// None denotes an infinite vertex. Infinite edges retain their sites but have no finite curve.
    pub start: Option<Point>,
    pub end: Option<Point>,
    pub curve: Option<Curve>,
}

#[derive(Clone, Debug, Serialize)]
pub struct VoronoiDiagram {
    pub source_segments: Vec<Segment>,
    /// (ring index, edge index), parallel to source_segments.
    pub source_boundaries: Vec<[usize; 2]>,
    pub cells: Vec<Site>,
    /// Each undirected edge appears once. This is the full diagram, not yet a medial axis.
    pub edges: Vec<VoronoiEdge>,
}

impl VoronoiDiagram {
    pub fn build(region: &Region) -> Result<Self> {
        let source_segments = region.segments();
        let source_boundaries = region
            .rings()
            .iter()
            .enumerate()
            .flat_map(|(r, ring)| (0..ring.points().len()).map(move |e| [r, e]))
            .collect();
        let input: Vec<[i32; 4]> = region
            .rings()
            .iter()
            .flat_map(|ring| {
                ring.points()
                    .iter()
                    .zip(ring.points().iter().cycle().skip(1))
                    .map(|(a, b)| [a.x as i32, a.y as i32, b.x as i32, b.y as i32])
            })
            .collect();
        let diagram = Builder::<i32>::default()
            .with_segments(input.iter())
            .map_err(backend)?
            .build()
            .map_err(backend)?;
        let site = |cell: &Cell| -> Result<Site> {
            let kind = match cell.source_category() {
                SourceCategory::Segment => SiteKind::Segment,
                SourceCategory::SegmentStart => SiteKind::SegmentStart,
                SourceCategory::SegmentEnd => SiteKind::SegmentEnd,
                SourceCategory::SinglePoint => {
                    return Err(backend(
                        "unexpected standalone point cell in segment-only input",
                    ));
                }
            };
            let segment = cell.source_index().usize();
            if segment >= source_segments.len() {
                return Err(backend("invalid Voronoi source index"));
            }
            Ok(Site { segment, kind })
        };
        let cells = diagram
            .cells()
            .iter()
            .map(site)
            .collect::<Result<Vec<_>>>()?;
        let mut edges = vec![];
        for edge in diagram.edges() {
            let twin = diagram
                .edge(edge.twin().map_err(backend)?)
                .map_err(backend)?;
            if edge.id().usize() > twin.id().usize() {
                continue;
            }
            let sites = [
                site(
                    diagram
                        .cell(edge.cell().map_err(backend)?)
                        .map_err(backend)?,
                )?,
                site(
                    diagram
                        .cell(twin.cell().map_err(backend)?)
                        .map_err(backend)?,
                )?,
            ];
            let point = |id| -> Result<Point> {
                let v = diagram.vertex(id).map_err(backend)?;
                let p = Point::new(v.x() / region.grid().scale(), v.y() / region.grid().scale());
                if !p.finite() {
                    return Err(backend("nonfinite Voronoi vertex"));
                }
                Ok(p)
            };
            let start = edge.vertex0().map(point).transpose()?;
            let end = twin.vertex0().map(point).transpose()?;
            let curve = match (start, end) {
                (Some(a), Some(b)) => Some(reconstruct(
                    a,
                    b,
                    edge.is_curved(),
                    sites,
                    &source_segments,
                    region.grid().tolerance_mm(),
                )?),
                _ => None,
            };
            edges.push(VoronoiEdge {
                sites,
                primary: edge.is_primary(),
                curved: edge.is_curved(),
                start,
                end,
                curve,
            });
        }
        Ok(Self {
            source_segments,
            source_boundaries,
            cells,
            edges,
        })
    }
}

fn reconstruct(
    start: Point,
    end: Point,
    curved: bool,
    sites: [Site; 2],
    sources: &[Segment],
    tolerance: f64,
) -> Result<Curve> {
    let mut curve = Curve {
        start,
        control: None,
        end,
        endpoint_reconstruction_error_mm: 0.0,
        numerical_reserve_mm: 0.0,
    };
    if curved {
        let (focus, directrix) = match (sites[0].point(sources), sites[1].point(sources)) {
            (Some(p), None) => (p, sources[sites[1].segment]),
            (None, Some(p)) => (p, sources[sites[0].segment]),
            _ => return Err(backend("curved edge must associate a point with a segment")),
        };
        let origin = directrix.start;
        let length = origin.distance(directrix.end);
        let ux = (directrix.end.x - origin.x) / length;
        let uy = (directrix.end.y - origin.y) / length;
        let project = |p: Point| {
            (
                (p.x - origin.x) * ux + (p.y - origin.y) * uy,
                -(p.x - origin.x) * uy + (p.y - origin.y) * ux,
            )
        };
        let (fu, fv) = project(focus);
        if fv.abs() < tolerance * 1e-6 {
            return Err(Diagnostic::new(
                "CURVE_PRECISION",
                "point/directrix separation is numerically unresolved",
            ));
        }
        let parabola = |u: f64| {
            let v = ((u - fu).powi(2) + fv * fv) / (2.0 * fv);
            Point::new(origin.x + u * ux - v * uy, origin.y + u * uy + v * ux)
        };
        let u0 = project(start).0;
        let u1 = project(end).0;
        let a = parabola(u0);
        let b = parabola(u1);
        let m = parabola((u0 + u1) / 2.0);
        let control = Point::new(2.0 * m.x - (a.x + b.x) / 2.0, 2.0 * m.y - (a.y + b.y) / 2.0);
        // Keep library vertices for common endpoint connectivity. Perturbing only the
        // Bezier endpoints moves the entire curve by at most the maximum endpoint error.
        curve.control = Some(control);
        curve.endpoint_reconstruction_error_mm = a.distance(start).max(b.distance(end));
    }
    let magnitude = [start, end, curve.control.unwrap_or(start)]
        .iter()
        .map(|p| p.x.abs().max(p.y.abs()))
        .fold(1.0, f64::max);
    curve.numerical_reserve_mm = 128.0 * f64::EPSILON * magnitude;
    if !curve.endpoint_reconstruction_error_mm.is_finite()
        || curve.control.is_some_and(|p| !p.finite())
        || curve.endpoint_reconstruction_error_mm + curve.numerical_reserve_mm > tolerance / 16.0
    {
        return Err(Diagnostic::new(
            "CURVE_PRECISION",
            "Voronoi curve reconstruction exceeds its numerical budget",
        ));
    }
    Ok(curve)
}
