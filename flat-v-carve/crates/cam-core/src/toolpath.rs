//! Generic planned motions shared by every operation planner.
//!
//! This is the new plan representation from the 2.5D CAM contract: feed
//! motion is separate from material effect, and every motion belongs to
//! exactly one execution stage. The legacy [`crate::motion::Motion`] stays
//! inside the V-carve engine and is adapted per operation.
//!
//! Knife motions program the blade holder's pivot in XY (plan section 12.1);
//! the visible blade tip is derived for display from the modeled heading
//! carried on the motion and the tool's blade offset via [`knife_tip`].
use crate::geometry::Point;
use crate::motion::Position;
use serde::{Deserialize, Serialize};

/// One circular feed move in the XY plane, at constant or linearly varying Z.
///
/// The arc is the circle of `center` through the motion's start and end, run
/// in `clockwise` order; Z interpolates linearly over the same parameter. The
/// plan states the centre rather than a radius because that is what the
/// program carries (`G2/G3` with `I`/`J`) and what a reader can reconstruct
/// without choosing between the two arcs that share a chord.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArcMove {
    pub center: Point,
    pub clockwise: bool,
}

impl ArcMove {
    /// Radius of the arc through `from`, or `None` when the centre is
    /// degenerate.
    pub fn radius(self, from: Point) -> Option<f64> {
        let radius = from.distance(self.center);
        (radius.is_finite() && radius > 0.).then_some(radius)
    }

    /// Angular sweep in radians over `from` -> `to`, in the arc's direction:
    /// always positive, and `TAU` for a closed circle.
    pub fn sweep_rad(self, from: Point, to: Point) -> Option<f64> {
        self.radius(from)?;
        let start = (from.y - self.center.y).atan2(from.x - self.center.x);
        let end = (to.y - self.center.y).atan2(to.x - self.center.x);
        let mut sweep = if self.clockwise {
            start - end
        } else {
            end - start
        };
        if sweep < 0. {
            sweep += std::f64::consts::TAU;
        }
        if sweep <= 1e-12 {
            // Coincident endpoints: a full circle, never a zero-length arc.
            sweep = std::f64::consts::TAU;
        }
        Some(sweep)
    }

    /// XY length of the arc, for feed timing and motion shape reports.
    pub fn length(self, from: Point, to: Point) -> Option<f64> {
        Some(self.radius(from)? * self.sweep_rad(from, to)?)
    }

    /// The point at `fraction` of the arc's sweep, for sampling and replay.
    pub fn point_at(self, from: Point, to: Point, fraction: f64) -> Option<Point> {
        let start = (from.y - self.center.y).atan2(from.x - self.center.x);
        let radius = self.radius(from)?;
        let sweep = self.sweep_rad(from, to)?;
        let angle = start + if self.clockwise { -1. } else { 1. } * sweep * fraction;
        Some(Point::new(
            self.center.x + radius * angle.cos(),
            self.center.y + radius * angle.sin(),
        ))
    }

    /// Distance from `p` to the arc from `from` to `to`: the radial distance
    /// when `p`'s angle falls inside the sweep, otherwise the nearer endpoint.
    /// Degenerate arcs fall back to the chord so a caller never has to choose
    /// between "no answer" and an invented one.
    pub fn distance(self, from: Point, to: Point, p: Point) -> f64 {
        let (Some(radius), Some(sweep)) = (self.radius(from), self.sweep_rad(from, to)) else {
            return distance_to_segment(p, from, to);
        };
        let start_angle = (from.y - self.center.y).atan2(from.x - self.center.x);
        let point_angle = (p.y - self.center.y).atan2(p.x - self.center.x);
        let mut along = if self.clockwise {
            start_angle - point_angle
        } else {
            point_angle - start_angle
        };
        along = along.rem_euclid(std::f64::consts::TAU);
        if along <= sweep {
            return (p.distance(self.center) - radius).abs();
        }
        p.distance(from).min(p.distance(to))
    }
}

/// Distance from a point to a segment; the arc helper's fallback.
pub fn distance_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let length_squared = dx * dx + dy * dy;
    if length_squared == 0. {
        return p.distance(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / length_squared).clamp(0., 1.);
    p.distance(Point::new(a.x + t * dx, a.y + t * dy))
}

/// Distance from a point to a motion's axis path: its arc when it has one,
/// its segment otherwise.
pub fn motion_distance(motion: &PlannedMotion, p: Point) -> f64 {
    let (from, to) = (motion.start.xy(), motion.end.xy());
    match motion.interpolation {
        Interpolation::ArcFeed(arc) => arc.distance(from, to, p),
        _ => distance_to_segment(p, from, to),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    Rapid,
    LinearFeed,
    /// Circular feed: the machine runs `G2`/`G3` for this motion.
    ArcFeed(ArcMove),
    /// The axes halt with the spindle turning (`G4 P`), in seconds. A dwell
    /// motion has no movement: `start` and `end` are the same position.
    Dwell {
        seconds: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionPurpose {
    Clearance,
    Approach,
    Entry,
    Rough,
    Finish,
    TabTransition,
    LeadIn,
    LeadOut,
    KnifeCut,
    KnifeAlign,
    KnifeSwivel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionEffect {
    /// No material is removed or marked by this move.
    None,
    /// The cutter sweep of this move removes stock under the engine's
    /// cutting semantics.
    MillingSweep,
    /// The blade traces or scores material without milled volume.
    KnifeTrace,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PlannedMotion {
    /// Global id, unique across the whole ordered plan.
    pub id: usize,
    pub operation_id: String,
    pub stage_id: String,
    pub tool_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contour_id: Option<String>,
    /// Operation-local pass index; layers are local to an operation/pass.
    pub pass_id: usize,
    pub layer: usize,
    pub interpolation: Interpolation,
    pub purpose: MotionPurpose,
    pub effect: MotionEffect,
    pub start: Position,
    pub end: Position,
    /// Required for every linear feed motion; checked before export.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feed_mm_min: Option<f64>,
    /// Knife stages only: the modeled blade heading in degrees CCW from +X —
    /// the direction the blade points from the holder pivot toward the tip —
    /// at this motion's start and end. The programmed XY of a knife motion
    /// is the pivot; the tip shown to the user derives from the heading and
    /// the knife geometry's blade offset (plan sections 12.1 and 15.3). A
    /// lifted rapid carries the retained heading at both ends. Never present
    /// on milling motions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blade_heading_deg: Option<(f64, f64)>,
}
impl PlannedMotion {
    pub fn is_cutting(&self) -> bool {
        !matches!(self.effect, MotionEffect::None)
    }
}

/// The blade-tip position for a pivot point and heading (plan section 12.5:
/// `tip = q + d * u(theta)` with `u` the unit vector the blade points along,
/// pivot toward tip). For travel tangent `t` the blade trails the holder, so
/// the heading is `t` rotated by 180 degrees. This is the pivot/tip display
/// contract; the knife planner owns producing the headings.
pub fn knife_tip(pivot: Point, heading_deg: f64, blade_offset_mm: f64) -> Point {
    let (s, c) = heading_deg.to_radians().sin_cos();
    Point::new(pivot.x + blade_offset_mm * c, pivot.y + blade_offset_mm * s)
}
