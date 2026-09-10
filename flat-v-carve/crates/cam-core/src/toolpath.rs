//! Generic planned motions shared by every operation planner.
//!
//! This is the new plan representation from the 2.5D CAM contract: feed
//! motion is separate from material effect, and every motion belongs to
//! exactly one execution stage. The legacy [`crate::motion::Motion`] stays
//! inside the legacy planners and is adapted per operation.
//!
//! Knife motions program the blade holder's pivot in XY (plan section 12.1);
//! the visible blade tip is derived for display from the modeled heading
//! carried on the motion and the tool's blade offset via [`knife_tip`].
use crate::geometry::Point;
use crate::motion::Position;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpolation {
    Rapid,
    LinearFeed,
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
    /// The cutter sweep of this move removes stock under the legacy engine's
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
