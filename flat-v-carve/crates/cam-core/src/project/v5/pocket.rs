//! Portable settings for constant-section pockets. Missing cutting values are
//! editable draft state; supplied invalid values are structural errors.
use super::{GeometryRef, GeometryRefKind, MillingAssignmentV5, error, number};
use crate::{
    geometry::Result,
    project::{CutDirection, HeightRef, LeadSpec},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PocketEntry {
    #[default]
    Plunge,
    Ramp {
        max_angle_deg: Option<f64>,
        feed_mm_min: Option<f64>,
    },
    Helix {
        /// Radius of the cutter center path, not the swept hole radius.
        radius_mm: Option<f64>,
        max_angle_deg: Option<f64>,
        feed_mm_min: Option<f64>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PocketLimits {
    pub max_layers: usize,
    pub max_loops_per_layer: usize,
    pub max_motions: usize,
}
impl Default for PocketLimits {
    fn default() -> Self {
        Self {
            max_layers: 256,
            max_loops_per_layer: 1024,
            max_motions: 100_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PocketSettingsV5 {
    pub components: Vec<GeometryRef>,
    pub assignment: MillingAssignmentV5,
    pub top: HeightRef,
    pub bottom: HeightRef,
    pub direction: Option<CutDirection>,
    pub wall_allowance_mm: Option<f64>,
    #[serde(default)]
    pub finish_walls: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_feed_mm_min: Option<f64>,
    #[serde(default)]
    pub entry: PocketEntry,
    #[serde(default)]
    pub lead_in: LeadSpec,
    #[serde(default)]
    pub lead_out: LeadSpec,
    #[serde(default)]
    pub limits: PocketLimits,
}

impl PocketSettingsV5 {
    pub(super) fn validate(&self) -> Result<()> {
        self.assignment.validate("pocket.assignment")?;
        for component in &self.components {
            component.validate()?;
            if component.kind != GeometryRefKind::FilledComponent {
                return Err(error(
                    "PROJECT_PARAMETER",
                    "pocket selection requires filled components",
                ));
            }
        }
        number(self.wall_allowance_mm, "pocket.wall_allowance_mm", false)?;
        number(self.finish_feed_mm_min, "pocket.finish_feed_mm_min", true)?;
        self.lead_in.validate("pocket.lead_in")?;
        self.lead_out.validate("pocket.lead_out")?;
        if let PocketEntry::Helix { radius_mm, .. } = self.entry {
            number(radius_mm, "pocket.entry.radius_mm", true)?;
        }
        if let PocketEntry::Ramp {
            max_angle_deg,
            feed_mm_min,
        }
        | PocketEntry::Helix {
            max_angle_deg,
            feed_mm_min,
            ..
        } = self.entry
        {
            number(max_angle_deg, "pocket.entry.max_angle_deg", true)?;
            number(feed_mm_min, "pocket.entry.feed_mm_min", true)?;
            if max_angle_deg.is_some_and(|angle| angle >= 90.) {
                return Err(error(
                    "PROJECT_PARAMETER",
                    "pocket entry angle must be below 90 degrees",
                ));
            }
        }
        if !(1..=256).contains(&self.limits.max_layers)
            || !(1..=1024).contains(&self.limits.max_loops_per_layer)
            || !(1..=100_000).contains(&self.limits.max_motions)
        {
            return Err(error(
                "PROJECT_PARAMETER",
                "pocket limits must be 1..256 layers, 1..1024 loops/layer and 1..100000 motions",
            ));
        }
        Ok(())
    }
}
