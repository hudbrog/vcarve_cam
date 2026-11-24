//! Reusable controller profiles. Untagged schema-2 files are the existing LinuxCNC contract.
use super::{Coolant, HolderSelection, error, sequence::SequenceProfile};
use crate::geometry::Result;
use serde::{Deserialize, Serialize};

/// Controller-facing view of one move. Geometry stays in setup coordinates until output preparation.
#[derive(Clone, Copy)]
pub(super) struct MachineMove {
    pub start: crate::motion::Position,
    pub end: crate::motion::Position,
    pub interpolation: crate::toolpath::Interpolation,
    pub feed_mm_min: Option<f64>,
}

impl From<&crate::toolpath::PlannedMotion> for MachineMove {
    fn from(m: &crate::toolpath::PlannedMotion) -> Self {
        Self {
            start: m.start,
            end: m.end,
            interpolation: m.interpolation,
            feed_mm_min: m.feed_mm_min,
        }
    }
}

/// A borrowed projection that exposes only the move geometry and feed to writers.
#[derive(Clone, Copy)]
pub(super) struct MachineMoves<'a>(&'a [crate::toolpath::PlannedMotion]);

impl<'a> From<&'a [crate::toolpath::PlannedMotion]> for MachineMoves<'a> {
    fn from(motions: &'a [crate::toolpath::PlannedMotion]) -> Self {
        Self(motions)
    }
}

impl MachineMoves<'_> {
    pub(super) fn iter(&self) -> impl Iterator<Item = MachineMove> + '_ {
        self.0.iter().map(MachineMove::from)
    }
}

pub(super) struct StageRun<'a> {
    pub stage: &'a crate::sequence::ExecutionStage,
    pub intent: std::borrow::Cow<'a, crate::sequence::ProcessIntent>,
    pub motions: MachineMoves<'a>,
}

/// Bind process events to their checked motion ranges in one forward pass.
pub(super) fn stage_runs(plan: &dyn crate::sequence::SequencePlan) -> Result<Vec<StageRun<'_>>> {
    use crate::sequence::ExecutionItem;
    let mut stages = plan.stages().iter();
    let mut result = Vec::with_capacity(plan.stages().len());
    let mut pending = None;
    for item in plan.execution() {
        match item {
            ExecutionItem::ToolChange { .. } => {} // Basic checks verify the active tool at every run.
            ExecutionItem::SetProcessIntent { intent } => pending = Some(intent),
            ExecutionItem::RunStage { stage_id } => {
                let stage = stages
                    .next()
                    .filter(|s| &s.stage_id == stage_id)
                    .ok_or_else(|| error("PLAN_EXECUTION_ORDER", "unexpected execution stage"))?;
                let intent = pending
                    .take()
                    .ok_or_else(|| error("PROCESS_SPINDLE_STATE", "missing process intent"))?;
                let motions = plan
                    .motions()
                    .get(stage.motion_range.0..stage.motion_range.1)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| error("POST_EMPTY", "empty stage motion range"))?;
                result.push(StageRun {
                    stage,
                    intent: plan.machine_intent(intent),
                    motions: motions.into(),
                });
            }
        }
    }
    if stages.next().is_some() {
        return Err(error("PLAN_EXECUTION_ORDER", "missing execution stage"));
    }
    Ok(result)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Controller {
    #[default]
    Linuxcnc,
    Grbl11,
}

impl Controller {
    pub fn is_linuxcnc(&self) -> bool {
        *self == Self::Linuxcnc
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrblProfile {
    pub controller: Controller,
    pub schema_version: u32,
    pub id: String,
    pub work_offset: String,
    pub clearance_z_mm: f64,
    pub decimal_places: usize,
    pub spindle_spinup_seconds: f64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub counterclockwise_spindle: bool,
    pub coolant: Coolant,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rapid_rate_mm_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<HolderSelection>,
}

impl GrblProfile {
    // Stock firmware stores a dwell in a 16-bit count of 50 ms ticks.
    pub(super) const MAX_DWELL_SECONDS: f64 = u16::MAX as f64 * 0.05;

    pub fn validate(&self) -> Result<()> {
        if self.controller != Controller::Grbl11
            || self.schema_version != 2
            || !crate::preview::valid_id(&self.id)
            || !matches!(
                self.work_offset.as_str(),
                "G54" | "G55" | "G56" | "G57" | "G58" | "G59"
            )
            || self.decimal_places > 9
            || !self.clearance_z_mm.is_finite()
            || self.clearance_z_mm <= 0.
            || !(0. ..=Self::MAX_DWELL_SECONDS).contains(&self.spindle_spinup_seconds)
            || self
                .rapid_rate_mm_min
                .is_some_and(|v| !v.is_finite() || v <= 0.)
        {
            return Err(error(
                "POST_PROFILE",
                "invalid Grbl 1.1 profile ID, version, work offset, clearance, precision, dwell or rapid rate",
            ));
        }
        if self.coolant == Coolant::Mist {
            return Err(error(
                "PROCESS_COOLANT_STATE",
                "stock Grbl 1.1 does not support mist coolant",
            ));
        }
        if let Some(holder) = &self.holder {
            holder
                .validate()
                .map_err(|msg| error("POST_PROFILE", msg))?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MachineProfile {
    LinuxCnc(SequenceProfile),
    Grbl(GrblProfile),
}

impl MachineProfile {
    pub fn from_json(text: &str) -> Result<Self> {
        if text.len() > 64_000 {
            return Err(error("POST_PROFILE", "profile exceeds 64 KB"));
        }
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| error("POST_PROFILE", e.to_string()))?;
        if value.get("controller").is_none() {
            return SequenceProfile::from_json(text).map(Self::LinuxCnc);
        }
        let profile: GrblProfile =
            serde_json::from_str(text).map_err(|e| error("POST_PROFILE", e.to_string()))?;
        profile.validate()?;
        Ok(Self::Grbl(profile))
    }

    pub fn id(&self) -> &str {
        match self {
            Self::LinuxCnc(p) => &p.id,
            Self::Grbl(p) => &p.id,
        }
    }
    pub fn set_id(&mut self, id: String) {
        match self {
            Self::LinuxCnc(p) => p.id = id,
            Self::Grbl(p) => p.id = id,
        }
    }
    pub fn clearance(&self) -> f64 {
        match self {
            Self::LinuxCnc(p) => p.clearance_z_mm,
            Self::Grbl(p) => p.clearance_z_mm,
        }
    }
}
