//! Linear XYZ motions; coordinates use stock-top Z=0 and negative cutting Z.
use crate::geometry::Point;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}
impl Position {
    pub fn new(p: Point, z: f64) -> Self {
        Self { x: p.x, y: p.y, z }
    }
    pub fn xy(self) -> Point {
        Point::new(self.x, self.y)
    }
    pub fn depth(self) -> f64 {
        (-self.z).max(0.)
    }
    pub fn finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
    pub fn lerp(self, b: Self, t: f64) -> Self {
        Self {
            x: self.x + (b.x - self.x) * t,
            y: self.y + (b.y - self.y) * t,
            z: self.z + (b.z - self.z) * t,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionKind {
    RapidXY,
    RapidRetract,
    Approach,
    Plunge,
    Ramp,
    Cut,
}
impl MotionKind {
    pub fn cutting(self) -> bool {
        matches!(self, Self::Plunge | Self::Ramp | Self::Cut)
    }
    pub fn rapid(self) -> bool {
        matches!(self, Self::RapidXY | Self::RapidRetract)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Motion {
    pub id: usize,
    pub tool_id: String,
    pub operation_id: String,
    pub layer: usize,
    pub kind: MotionKind,
    pub start: Position,
    pub end: Position,
    pub feed_mm_min: Option<f64>,
}
impl Motion {
    /// Clip the actual linear XYZ move to the part whose tip reaches this slice.
    pub fn at_depth(&self, depth: f64) -> Option<(Point, Point)> {
        if !self.kind.cutting() {
            return None;
        }
        let a = -self.start.z;
        let b = -self.end.z;
        if a < depth && b < depth {
            return None;
        }
        let (lo, hi) = if a >= depth && b >= depth {
            (0., 1.)
        } else if b > a {
            ((depth - a) / (b - a), 1.)
        } else {
            (0., (depth - a) / (b - a))
        };
        Some((
            self.start.lerp(self.end, lo).xy(),
            self.start.lerp(self.end, hi).xy(),
        ))
    }
}
