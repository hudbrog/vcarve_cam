use super::error;
use crate::{
    geometry::{Point, Result},
    job::Job,
    motion::Position,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LengthCompensation {
    MacroManaged,
    ToolTable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpindleDirection {
    Clockwise,
    Counterclockwise,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Coolant {
    Off,
    Flood,
    Mist,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZDatum {
    StockTop,
    StockBottom,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolMapping {
    pub tool_id: String,
    pub tool_number: u32,
    pub length_offset_number: Option<u32>,
    pub spindle_direction: SpindleDirection,
}
/// Position of the NEW tool tip in the selected work frame, AFTER the chosen
/// length compensation is active. This is a machine-owned contract, not a
/// claim that M6 or G43 preserves the old tool's displayed position.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum M6Return {
    CallerPosition,
    FixedPosition {
        position_mm: Position,
    },
    /// The machine contract guarantees an unobstructed upward move to this
    /// work-coordinate Z, then clear XY transit there. Initial XYZ is unknown;
    /// this transition is explicitly machine-owned, not geometrically proven.
    SafeRetract {
        z_mm: f64,
        transit_xy_mm: Point,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M6Contract {
    pub reference: String,
    pub reviewed: bool,
    pub return_position: M6Return,
    /// M6 must preserve the stock datum, not use G52/G92 as compensation, and
    /// use no XY tool offsets or work-frame rotation. G43/G43.1 Z is supported.
    pub preserves_work_datum: bool,
    pub local_offsets_unused: bool,
    pub tool_offsets_z_only: bool,
}
/// Separately versioned export configuration. Editable schema-3 job profiles
/// remain portable; their free-text M6 description is never export authority.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxCncProfile {
    pub schema_version: u32,
    pub id: String,
    pub work_offset: String,
    pub z_datum: ZDatum,
    /// Clearance above stock TOP, matching the planner. Emitted Z also
    /// includes stock thickness when the machine datum is stock bottom.
    pub clearance_z_mm: f64,
    pub decimal_places: usize,
    /// Required operator setup, expressed AFTER modal setup and G92.1.
    /// With None, startup is macro-owned; no axis move precedes the first M6.
    pub program_start_position_mm: Option<Position>,
    pub length_compensation: LengthCompensation,
    pub tools: Vec<ToolMapping>,
    pub spindle_spinup_seconds: f64,
    pub coolant: Coolant,
    pub m6: M6Contract,
}
impl LinuxCncProfile {
    pub fn from_json(text: &str) -> Result<Self> {
        if text.len() > 64_000 {
            return Err(error("POST_PROFILE", "profile exceeds 64 KB"));
        }
        serde_json::from_str(text).map_err(|e| error("POST_PROFILE", e.to_string()))
    }
    pub fn validate(&self, job: &Job) -> Result<()> {
        if self.schema_version != 1
            || !crate::preview::valid_id(&self.id)
            || !matches!(
                self.work_offset.as_str(),
                "G54" | "G55" | "G56" | "G57" | "G58" | "G59" | "G59.1" | "G59.2" | "G59.3"
            )
            || self.decimal_places > 9
            || !self.clearance_z_mm.is_finite()
            || self.clearance_z_mm <= 0.
            || !self.spindle_spinup_seconds.is_finite()
            || !(0. ..=3600.).contains(&self.spindle_spinup_seconds)
        {
            return Err(error(
                "POST_PROFILE",
                "invalid profile version, ID, work offset, clearance, precision, or dwell",
            ));
        }
        if !self.m6.reviewed
            || self.m6.reference.trim().is_empty()
            || self.m6.reference.len() > 4000
            || !self.m6.preserves_work_datum
            || !self.m6.local_offsets_unused
            || !self.m6.tool_offsets_z_only
        {
            return Err(error(
                "POST_M6_CONTRACT",
                "a reviewed M6 reference must establish the return position after compensation, preserve the work datum without rotation, leave G52/G92 unused, and use only Z tool offsets",
            ));
        }
        let settings = job
            .endmill_planning
            .as_ref()
            .ok_or_else(|| error("POST_PROFILE", "planning clearance is required"))?;
        if settings.clearance_z_mm != self.clearance_z_mm {
            return Err(error(
                "POST_CLEARANCE",
                "profile clearance differs from the verified plan; regenerate with the intended clearance",
            ));
        }
        // Do not round safety planes or assumed macro/operator positions down.
        let machine_clearance = super::rounded(
            self.clearance_z_mm + self.z_offset(job)?,
            self.decimal_places,
        );
        if super::rounded(self.clearance_z_mm, self.decimal_places) != self.clearance_z_mm
            || self
                .program_start_position_mm
                .is_some_and(|s| s.z != machine_clearance)
        {
            return Err(error(
                "POST_START_POSITION",
                "program must start at the declared clearance plane with known current-tool compensation",
            ));
        }
        if let Some(start) = self.program_start_position_mm {
            self.validate_position(start)?;
        }
        if self.program_start_position_mm.is_none()
            && !matches!(self.m6.return_position, M6Return::SafeRetract { .. })
        {
            return Err(error(
                "POST_START_POSITION",
                "unknown startup coordinates require the declared safe-retract M6 contract",
            ));
        }
        if let M6Return::FixedPosition { position_mm } = self.m6.return_position {
            self.validate_position(position_mm)?;
            if position_mm.z < machine_clearance {
                return Err(error(
                    "POST_M6_CONTRACT",
                    "M6 must return the compensated tool tip at or above clearance",
                ));
            }
        }
        if let M6Return::SafeRetract {
            z_mm,
            transit_xy_mm,
        } = self.m6.return_position
        {
            self.validate_position(Position::new(transit_xy_mm, z_mm))?;
            if z_mm < machine_clearance {
                return Err(error(
                    "POST_M6_CONTRACT",
                    "safe retract Z must be at or above stock-top clearance in the machine work frame",
                ));
            }
        }
        if self.tools.len() != 2 {
            return Err(error(
                "POST_TOOL_MAPPING",
                "map exactly the job's endmill and V-bit",
            ));
        }
        let mut numbers = std::collections::BTreeSet::new();
        let mut ids = std::collections::BTreeSet::new();
        for t in &self.tools {
            if !ids.insert(&t.tool_id)
                || !numbers.insert(t.tool_number)
                || !(1..=99999).contains(&t.tool_number)
                || !job.tools.iter().any(|j| j.id == t.tool_id)
                || match self.length_compensation {
                    LengthCompensation::MacroManaged => t.length_offset_number.is_some(),
                    LengthCompensation::ToolTable => !t
                        .length_offset_number
                        .is_some_and(|h| (1..=99999).contains(&h)),
                }
            {
                return Err(error(
                    "POST_TOOL_MAPPING",
                    "tool IDs/numbers must be unique; tool-table compensation requires a positive H mapping, macro-managed compensation forbids H mappings",
                ));
            }
        }
        if let Some(legacy) = &job.machine_profile {
            let e = self.tool(&job.operation.endmill_id);
            let v = self.tool(&job.operation.vbit_id);
            if legacy.id != self.id
                || legacy
                    .work_offset
                    .as_ref()
                    .is_some_and(|w| w != &self.work_offset)
                || legacy
                    .clearance_z_mm
                    .is_some_and(|z| z != self.clearance_z_mm)
                || legacy
                    .endmill_tool_number
                    .is_some_and(|n| n != e.tool_number)
                || legacy.vbit_tool_number.is_some_and(|n| n != v.tool_number)
            {
                return Err(error(
                    "POST_PROFILE_MISMATCH",
                    "export profile conflicts with machine settings embedded in the saved job",
                ));
            }
        }
        Ok(())
    }
    pub(super) fn z_offset(&self, job: &Job) -> Result<f64> {
        let z = match self.z_datum {
            ZDatum::StockTop => 0.,
            ZDatum::StockBottom => job.stock.thickness_mm.ok_or_else(|| {
                error(
                    "POST_Z_DATUM",
                    "stock-bottom datum requires stock thickness",
                )
            })?,
        };
        if !z.is_finite()
            || !(0. ..=1_000_000.).contains(&z)
            || super::rounded(z, self.decimal_places) != z
        {
            return Err(error(
                "POST_Z_DATUM",
                "stock thickness must be exactly representable at output precision",
            ));
        }
        Ok(z)
    }
    fn validate_position(&self, p: Position) -> Result<()> {
        for value in [p.x, p.y, p.z] {
            if !value.is_finite()
                || value.abs() > 1_000_000.
                || super::rounded(value, self.decimal_places) != value
            {
                return Err(error(
                    "POST_PROFILE_PRECISION",
                    "declared startup/return positions and clearance must be exactly representable at output precision and within +/-1000000 mm",
                ));
            }
        }
        Ok(())
    }
    pub(super) fn tool(&self, id: &str) -> &ToolMapping {
        self.tools
            .iter()
            .find(|t| t.tool_id == id)
            .expect("validated tool mapping")
    }
    pub(super) fn returned_position(&self, caller: Option<Position>) -> Position {
        match self.m6.return_position {
            M6Return::CallerPosition => {
                caller.expect("validated startup; previous stage ends at clearance")
            }
            M6Return::FixedPosition { position_mm } => position_mm,
            M6Return::SafeRetract {
                z_mm,
                transit_xy_mm,
            } => Position::new(transit_xy_mm, z_mm),
        }
    }
}
