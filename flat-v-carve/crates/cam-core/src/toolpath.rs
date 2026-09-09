//! Generic planned motions shared by every operation planner.
//!
//! This is the new plan representation from the 2.5D CAM contract: feed
//! motion is separate from material effect, and every motion belongs to
//! exactly one execution stage. The legacy [`crate::motion::Motion`] stays
//! inside the legacy planners and is adapted per operation.
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
}
impl PlannedMotion {
    pub fn is_cutting(&self) -> bool {
        !matches!(self.effect, MotionEffect::None)
    }
}
