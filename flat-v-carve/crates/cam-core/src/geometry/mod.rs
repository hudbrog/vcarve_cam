//! Millimeter geometry. Dependency containers and graph handles stay in adapters.
mod polygon;
mod precision;
mod query;
mod snapping;
pub(crate) mod spatial;
pub(crate) mod union;
mod voronoi;

pub use polygon::{BooleanOp, Region, Ring, WindingRule};
pub use precision::{Grid, GridPoint};
pub use query::{BoundaryQuery, Clearance, PointLocation};
use serde::{Deserialize, Serialize};
pub use voronoi::{Curve, Linearization, Site, SiteKind, VoronoiDiagram, VoronoiEdge};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    pub fn distance(self, other: Self) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }
    pub fn lerp(self, other: Self, t: f64) -> Self {
        Self::new(
            self.x + (other.x - self.x) * t,
            self.y + (other.y - self.y) * t,
        )
    }
    pub(crate) fn finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start: Point,
    pub end: Point,
}

impl Segment {
    /// Independent Euclidean distance; does not call either geometry dependency.
    pub fn distance(self, p: Point) -> f64 {
        let dx = self.end.x - self.start.x;
        let dy = self.end.y - self.start.y;
        if dx == 0.0 && dy == 0.0 {
            return self.start.distance(p);
        }
        let t = ((p.x - self.start.x) * dx + (p.y - self.start.y) * dy) / (dx * dx + dy * dy);
        p.distance(self.start.lerp(self.end, t.clamp(0.0, 1.0)))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub stage: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Warning,
    Error,
}

impl Diagnostic {
    pub(crate) fn source(mut self, id: &str) -> Self {
        self.source_id = Some(id.into());
        self
    }
    pub(crate) fn at_stage(mut self, stage: &'static str) -> Self {
        self.stage = stage;
        self
    }
    pub(crate) fn warning(mut self) -> Self {
        self.severity = Severity::Warning;
        self
    }
    pub(crate) fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            severity: if matches!(
                code,
                "DUPLICATE_VERTEX_REMOVED" | "OFFSET_EMPTY_AREA" | "SNAPPED_VERTEX_COALESCED"
            ) {
                Severity::Warning
            } else {
                Severity::Error
            },
            stage: "geometry",
            message: message.into(),
            source_id: None,
        }
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Diagnostic {}
pub type Result<T> = std::result::Result<T, Diagnostic>;

pub(crate) fn backend(e: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::new("GEOMETRY_BACKEND", e.to_string())
}
