//! Ordered LinuxCNC output for sequence plans (plan sections 8.2 and 14).
//!
//! This pipeline is separate from the legacy M5 export in [`super`]: basic
//! checks gate generation, process state is resolved before any G-code, and
//! an independent numeric readback re-derives every motion, tool change and
//! spindle state from the emitted bytes. Detailed stock-quality analysis
//! stays optional and is never run here.
use crate::{
    checks::{BasicCheckReport, require_pass},
    geometry::{Diagnostic, Result},
    operations::drag_knife::replay::ReplayStatus,
    project::{
        CamJob, MillingAssignment, OperationSettings, SpindleDirection as JobSpindleDirection,
    },
    sequence::{
        ExecutionStage, GenerationStatus, ProcessSpindle, SequencePlan, StageRole,
        TrustedSequencePlan,
    },
    toolpath::{Interpolation, PlannedMotion},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    Coolant, LengthCompensation, LinuxCncProfile, M6Contract, PathControl, SpindleDirection,
    ZDatum, error,
};

pub const SEQUENCE_PROFILE_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceToolMapping {
    /// A physical job tool ID, mapped exactly once regardless of how many
    /// operations or assignments use it.
    pub tool_id: String,
    pub tool_number: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_offset_number: Option<u32>,
}

/// Generic LinuxCNC export configuration, schema 2. The Z datum is owned by
/// the job's `setup.work_zero` (never duplicated here) and spindle direction
/// is owned by the job's assignments (never by the T/H mapping).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceProfile {
    pub schema_version: u32,
    pub id: String,
    pub work_offset: String,
    pub clearance_z_mm: f64,
    pub decimal_places: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_start_position_mm: Option<crate::motion::Position>,
    pub length_compensation: LengthCompensation,
    #[serde(default)]
    pub path_control: PathControl,
    pub tools: Vec<SequenceToolMapping>,
    pub spindle_spinup_seconds: f64,
    pub coolant: Coolant,
    pub m6: M6Contract,
}

/// Fully resolved process state for one stage; no unresolved markers remain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PreparedProcess {
    pub spindle: PreparedSpindle,
    pub coolant: Coolant,
    pub path_control: PathControl,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PreparedSpindle {
    Off,
    On {
        rpm: f64,
        direction: SpindleDirection,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PreparedStage {
    pub stage: ExecutionStage,
    pub process: PreparedProcess,
    /// The exact emitted tool number for this stage's tool.
    pub tool_number: u32,
    /// The exact emitted H number when length compensation is tool-table
    /// managed (F3a: the decoder requires `G43 H…` per stage to match it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_offset_number: Option<u32>,
}

/// An immutable plan bound to a profile and fully resolved process state.
/// Only this form can produce G-code (plan section 8.2).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparedExecution {
    pub plan_fingerprint: String,
    pub profile_fingerprint: String,
    pub process_fingerprint: String,
    pub output_decimal_places: usize,
    /// Z shift applied before output formatting for the selected work-zero
    /// datum; the inverse shift is applied after numeric readback.
    pub machine_offset_mm: [f64; 3],
    /// The controller work frame every motion must run under (F3a: decoded
    /// work-offset words are compared against this, not assumed).
    #[serde(default)]
    pub work_offset: String,
    /// The machine-contract start position bridges depart from, when the
    /// profile declares one.
    #[serde(default)]
    pub program_start_position_mm: Option<crate::motion::Position>,
    /// Whether each stage must re-establish `G43 H…` (tool table) or must
    /// never emit it (macro managed).
    pub length_compensation: LengthCompensation,
    pub stages: Vec<PreparedStage>,
    pub basic_checks: BasicCheckReport,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SequenceProgram {
    pub filename: String,
    pub gcode: String,
}

/// Aggregate outcome of the independent replay of the emitted (rounded)
/// knife motions, published with every export that contains knife stages
/// (F3b). Bound evidence with per-sample traces is the separate
/// `drag_knife::evidence` report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnifeReplayStatus {
    Within,
    Exceeded,
    BudgetExhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct KnifeReplaySummary {
    pub status: KnifeReplayStatus,
    pub max_tip_deviation_mm: f64,
    pub max_heading_error_deg: f64,
    pub tip_budget_mm: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SequenceExportReport {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub engine_version: String,
    pub plan_fingerprint: String,
    pub profile_fingerprint: String,
    pub process_fingerprint: String,
    pub output_decimal_places: usize,
    pub machine_offset_mm: [f64; 3],
    pub basic_checks: BasicCheckReport,
    pub program_sha256: String,
    pub motion_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knife_replay: Option<KnifeReplaySummary>,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct SequenceExport {
    pub program: SequenceProgram,
    pub report: SequenceExportReport,
}

/// Ordered output layout of a prepared execution (plan section 14.5): one
/// program, or sequential files split at contiguous tool stages so a
/// recurring tool keeps its execution position instead of being regrouped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum OutputLayout {
    OneProgram,
    SequentialFiles,
}

/// One manifest entry of an export bundle. The manifest describes ordering
/// and stock prerequisites for humans and supervisors; it is never
/// authoritative input to the numeric reader (plan section 14.5).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BundleManifestFile {
    pub order: usize,
    pub filename: String,
    pub sha256: String,
    pub byte_length: usize,
    pub stage_ids: Vec<String>,
    pub tool_number: u32,
    /// This file's global `[first, end)` motion span within the plan.
    pub motion_range: (usize, usize),
    /// The files that must run before this one, in order.
    pub runs_after: Vec<String>,
    /// The last operation covered by this file's stages: after running it,
    /// the stock is through this operation (the starting stock of the next
    /// file in the manifest).
    pub stock_through_operation: String,
}

/// Companion manifest of an export bundle: ordered files, per-file exact
/// byte digests and the stock/ordering prerequisites of each file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SequenceBundleManifest {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub engine_version: String,
    pub layout: OutputLayout,
    pub plan_fingerprint: String,
    pub profile_fingerprint: String,
    pub process_fingerprint: String,
    pub execution_fingerprint: String,
    pub output_decimal_places: usize,
    pub files: Vec<BundleManifestFile>,
}

/// A checked export bundle (H5): the exact bytes of every ordered file, the
/// manifest and the aggregate report. Every file is decoded and compared
/// independently against the plan; `report.program_sha256` binds the exact
/// bytes — the single file's digest for [`OutputLayout::OneProgram`], the
/// digest of the ordered per-file digests otherwise.
#[derive(Clone, Debug)]
pub struct SequenceBundle {
    pub layout: OutputLayout,
    pub files: Vec<SequenceProgram>,
    pub manifest: SequenceBundleManifest,
    pub report: SequenceExportReport,
}

impl SequenceProfile {
    pub fn from_json(text: &str) -> Result<Self> {
        if text.len() > 64_000 {
            return Err(error("POST_PROFILE", "profile exceeds 64 KB"));
        }
        let profile: Self =
            serde_json::from_str(text).map_err(|e| error("POST_PROFILE", e.to_string()))?;
        profile.validate_shape()?;
        Ok(profile)
    }

    pub fn validate_shape(&self) -> Result<()> {
        if self.schema_version != SEQUENCE_PROFILE_SCHEMA_VERSION
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
        let mut ids = std::collections::BTreeSet::new();
        let mut numbers = std::collections::BTreeSet::new();
        for tool in &self.tools {
            if !ids.insert(&tool.tool_id) {
                return Err(error(
                    "POST_TOOL_MAPPING",
                    format!("tool {:?} is mapped more than once", tool.tool_id),
                ));
            }
            if !(1..=99999).contains(&tool.tool_number) || !numbers.insert(tool.tool_number) {
                return Err(error(
                    "POST_TOOL_MAPPING",
                    format!(
                        "tool {:?}: T numbers must be 1..=99999 and unique",
                        tool.tool_id
                    ),
                ));
            }
            match self.length_compensation {
                LengthCompensation::MacroManaged if tool.length_offset_number.is_some() => {
                    return Err(error(
                        "POST_TOOL_MAPPING",
                        format!(
                            "tool {:?}: macro-managed compensation requires length_offset_number to be null",
                            tool.tool_id
                        ),
                    ));
                }
                LengthCompensation::ToolTable
                    if !tool
                        .length_offset_number
                        .is_some_and(|h| (1..=99999).contains(&h)) =>
                {
                    return Err(error(
                        "POST_TOOL_MAPPING",
                        format!(
                            "tool {:?}: tool-table compensation requires an H number between 1 and 99999",
                            tool.tool_id
                        ),
                    ));
                }
                _ => {}
            }
        }
        if let PathControl::Blend {
            tolerance_mm,
            naive_cam_tolerance_mm,
        } = self.path_control
        {
            if !tolerance_mm.is_finite() || !(0.000001..=10.).contains(&tolerance_mm) {
                return Err(error(
                    "POST_PROFILE",
                    "path blending tolerance must be positive, at most 10 mm, and finite",
                ));
            }
            if naive_cam_tolerance_mm
                .is_some_and(|q| !q.is_finite() || !(0.000001..=tolerance_mm).contains(&q))
            {
                return Err(error(
                    "POST_PROFILE",
                    "naive cam tolerance must be positive and no greater than the path blending tolerance, as LinuxCNC requires Q <= P",
                ));
            }
        }
        Ok(())
    }

    fn tool(&self, tool_id: &str) -> Option<&SequenceToolMapping> {
        self.tools.iter().find(|t| t.tool_id == tool_id)
    }
}

/// Apply a legacy schema-1 profile to a canonical job as one document/profile
/// application: the Z datum moves into `setup.work_zero.z` and the per-tool
/// spindle directions move into every matching milling assignment (only
/// where still unset; an explicit job value wins). The job keeps
/// `legacy_machine_profile` as descriptive metadata.
pub fn apply_legacy_profile(profile: &LinuxCncProfile, job: &CamJob) -> Result<CamJob> {
    // When the job is exactly a migratable Flat V-carve document, run the
    // legacy profile validation against its reconstructed legacy job so the
    // old checks (clearance, mapped tools, blend tolerance) still apply.
    if job.operations.len() == 1
        && matches!(job.operations[0].settings, OperationSettings::FlatVcarve(_))
    {
        if let Ok(legacy) = crate::operations::flat_vcarve::to_legacy_job(
            job,
            &job.operations[0].id,
            match &job.operations[0].settings {
                OperationSettings::FlatVcarve(settings) => settings,
                _ => unreachable!(),
            },
        ) {
            profile.validate(&legacy)?;
        }
    } else {
        for mapping in &profile.tools {
            if !job.tools.iter().any(|t| t.id == mapping.tool_id) {
                return Err(error(
                    "POST_TOOL_MAPPING",
                    format!(
                        "profile maps tool {:?} which the job does not define",
                        mapping.tool_id
                    ),
                ));
            }
        }
        if let Some(clearance) = job.setup.clearance_above_stock_mm
            && clearance != profile.clearance_z_mm
        {
            return Err(error(
                "POST_CLEARANCE",
                "profile clearance differs from the job's setup clearance",
            ));
        }
    }
    let mut applied = job.clone();
    applied.setup.work_zero.z = match profile.z_datum {
        ZDatum::StockTop => crate::project::WorkZeroZ::StockTop,
        ZDatum::StockBottom => crate::project::WorkZeroZ::StockBottom,
    };
    for operation in &mut applied.operations {
        let mut assignments: Vec<&mut MillingAssignment> = vec![];
        match &mut operation.settings {
            OperationSettings::FlatVcarve(settings) => {
                assignments.push(&mut settings.endmill);
                assignments.push(&mut settings.vbit);
            }
            OperationSettings::Face(settings) => assignments.push(&mut settings.assignment),
            OperationSettings::Profile(settings) => assignments.push(&mut settings.assignment),
            OperationSettings::DragKnife(_) => {}
        }
        for assignment in assignments {
            if let Some(mapping) = profile
                .tools
                .iter()
                .find(|t| t.tool_id == assignment.tool_id)
                && assignment.spindle_direction.is_none()
            {
                assignment.spindle_direction = Some(match mapping.spindle_direction {
                    SpindleDirection::Clockwise => JobSpindleDirection::Clockwise,
                    SpindleDirection::Counterclockwise => JobSpindleDirection::Counterclockwise,
                });
            }
        }
    }
    applied.validate()?;
    Ok(applied)
}

fn stage_intent(plan: &dyn SequencePlan, stage: &ExecutionStage) -> Result<ProcessSpindle> {
    // Walk execution in order; the intent immediately preceding this stage's
    // RunStage is the one that applies to it.
    let mut pending: Option<ProcessSpindle> = None;
    for item in plan.execution() {
        match item {
            crate::sequence::ExecutionItem::ToolChange { .. } => {}
            crate::sequence::ExecutionItem::SetProcessIntent { intent } => {
                pending = Some(intent.spindle.clone());
            }
            crate::sequence::ExecutionItem::RunStage { stage_id } => {
                if stage_id == &stage.stage_id {
                    return pending.ok_or_else(|| {
                        error(
                            "PROCESS_SPINDLE_STATE",
                            format!("stage '{}' has no process intent", stage.stage_id),
                        )
                    });
                }
            }
        }
    }
    Err(error(
        "PROCESS_SPINDLE_STATE",
        format!("stage '{}' never runs", stage.stage_id),
    ))
}

fn resolve_process(
    stage: &ExecutionStage,
    intent_spindle: &ProcessSpindle,
    profile: &SequenceProfile,
) -> Result<PreparedProcess> {
    let spindle = match intent_spindle {
        ProcessSpindle::Off => PreparedSpindle::Off,
        ProcessSpindle::Milling { rpm, direction } => match direction {
            Some(JobSpindleDirection::Clockwise) => PreparedSpindle::On {
                rpm: *rpm,
                direction: SpindleDirection::Clockwise,
            },
            Some(JobSpindleDirection::Counterclockwise) => PreparedSpindle::On {
                rpm: *rpm,
                direction: SpindleDirection::Counterclockwise,
            },
            None => {
                return Err(error(
                    "PROCESS_SPINDLE_STATE",
                    format!(
                        "milling stage '{}' has no resolved spindle direction; apply a machine profile to the job and replan",
                        stage.stage_id
                    ),
                ));
            }
        },
    };
    Ok(PreparedProcess {
        spindle,
        coolant: match stage.role {
            // Knife stages always run coolant off and exact path (plan
            // sections 12.5 and 14.4): no machine-profile coolant or blend
            // tolerance may leak into swivel geometry.
            StageRole::Knife => Coolant::Off,
            _ => profile.coolant,
        },
        path_control: match stage.role {
            // Knife stages always run exact path; blending could change
            // swivel geometry (plan section 12.5).
            StageRole::Knife => PathControl::ExactPath,
            _ => profile.path_control,
        },
    })
}

impl PreparedExecution {
    /// Bind a checked plan to a profile with fully resolved process state.
    /// Accepts both trusted plan shapes through [`TrustedSequencePlan`]; a
    /// plain plan reference is not an acceptable argument.
    pub fn prepare(plan: &dyn TrustedSequencePlan, profile: &SequenceProfile) -> Result<Self> {
        let plan = plan.trusted();
        let basic_checks = plan.basic_checks()?;
        require_pass(&basic_checks)?;
        profile.validate_shape()?;
        for stage in plan.stages() {
            if profile.tool(&stage.tool_id).is_none() {
                return Err(error(
                    "POST_TOOL_MAPPING",
                    format!(
                        "stage '{}' uses tool '{}' which the profile does not map",
                        stage.stage_id, stage.tool_id
                    ),
                ));
            }
        }
        if let Some(clearance) = plan.setup().clearance_above_stock_mm
            && clearance != profile.clearance_z_mm
        {
            return Err(error(
                "POST_CLEARANCE",
                "profile clearance differs from the plan's setup clearance; regenerate with the intended clearance",
            ));
        }
        // Work-zero output transform (plan section 6.2): the full selected
        // point, XY anchors included. Machine-profile startup/M6 positions
        // are already expressed in the controller work frame and are never
        // transformed again.
        let work_zero = crate::setup::resolve_work_zero_setup(plan.setup())?;
        let machine_offset = work_zero.output_offset();
        for value in [machine_offset.0, machine_offset.1, machine_offset.2] {
            if !value.is_finite() || rounded(value, profile.decimal_places) != value {
                return Err(error(
                    "POST_WORK_ZERO_PRECISION",
                    "work-zero offsets must be exactly representable at output precision",
                ));
            }
        }
        let mut stages = vec![];
        for stage in plan.stages() {
            let process = resolve_process(stage, &stage_intent(plan, stage)?, profile)?;
            stages.push(PreparedStage {
                stage: stage.clone(),
                process,
                tool_number: profile
                    .tool(&stage.tool_id)
                    .expect("validated mapping")
                    .tool_number,
                length_offset_number: if profile.length_compensation
                    == LengthCompensation::ToolTable
                {
                    profile
                        .tool(&stage.tool_id)
                        .and_then(|t| t.length_offset_number)
                } else {
                    None
                },
            });
        }
        let plan_fingerprint = plan.execution_fingerprint().to_string();
        let profile_fingerprint =
            crate::plan_hash::hash(profile).map_err(|e| error("POST_JSON", e.to_string()))?;
        let process_fingerprint =
            crate::plan_hash::hash(&stages).map_err(|e| error("POST_JSON", e.to_string()))?;
        Ok(Self {
            plan_fingerprint,
            profile_fingerprint,
            process_fingerprint,
            output_decimal_places: profile.decimal_places,
            machine_offset_mm: [machine_offset.0, machine_offset.1, machine_offset.2],
            work_offset: profile.work_offset.clone(),
            program_start_position_mm: profile.program_start_position_mm,
            length_compensation: profile.length_compensation,
            stages,
            basic_checks,
        })
    }

    /// Emit the one ordered program for this execution and verify it with an
    /// independent numeric readback. Detailed quality analysis is not run.
    pub fn export(
        &self,
        plan: &dyn TrustedSequencePlan,
        profile: &SequenceProfile,
    ) -> Result<SequenceExport> {
        let bundle = self.export_bundle(plan, profile, OutputLayout::OneProgram)?;
        let Some(program) = bundle.files.into_iter().next() else {
            return Err(error("POST_EMPTY", "no executable motions"));
        };
        Ok(SequenceExport {
            program,
            report: bundle.report,
        })
    }

    /// Stage-index groups of [`PreparedStage`]s per file for a layout: one
    /// group covering everything, or one group per run of contiguous stages
    /// on the same tool. Adjacent same-tool stages share a file; a recurring
    /// tool gets its own later file in execution order (plan section 14.5).
    fn file_groups(&self, layout: OutputLayout) -> Vec<(usize, usize)> {
        let count = self.stages.len();
        match layout {
            OutputLayout::OneProgram => vec![(0, count)],
            OutputLayout::SequentialFiles => {
                let mut groups = vec![];
                let mut start = 0;
                for index in 1..=count {
                    let same_tool = index < count
                        && self.stages[index].stage.tool_id == self.stages[start].stage.tool_id;
                    if !same_tool {
                        groups.push((start, index));
                        start = index;
                    }
                }
                groups
            }
        }
    }

    /// A copy of this preparation restricted to `self.stages[range]`, so a
    /// bundle file containing only those stages decodes against exactly the
    /// bridges, length-compensation and tool expectations it contains.
    fn with_stages(&self, range: std::ops::Range<usize>) -> PreparedExecution {
        let mut subset = self.clone();
        subset.stages = self.stages[range].to_vec();
        subset
    }

    /// Emit and check every ordered file of a layout (H5): each file
    /// independently decoded and compared against the plan at its global
    /// motion offset, the knife replay running over the concatenated
    /// decoded stream. Precision rises through nine places until formatting
    /// preserves every motion and the replay stays within budget; a
    /// required move is never deleted.
    pub fn export_bundle(
        &self,
        plan: &dyn TrustedSequencePlan,
        profile: &SequenceProfile,
        layout: OutputLayout,
    ) -> Result<SequenceBundle> {
        let plan = plan.trusted();
        if plan.motions().is_empty() {
            return Err(error("POST_EMPTY", "no executable motions"));
        }
        for result in plan.operation_results() {
            if matches!(
                result.generation_status,
                GenerationStatus::Incomplete | GenerationStatus::Inconclusive
            ) {
                return Err(error(
                    "PLAN_GENERATION_INCOMPLETE",
                    format!(
                        "operation '{}' did not generate completely",
                        result.operation_id
                    ),
                ));
            }
        }
        let groups = self.file_groups(layout);
        let has_knife = plan
            .stages()
            .iter()
            .any(|stage| stage.role == StageRole::Knife);
        let mut places = profile.decimal_places;
        loop {
            let preserved = motions_preserved(plan, places);
            let mut programs = vec![];
            let mut decoded_stream = DecodedProgram {
                motions: vec![],
                bridges: vec![],
                tool_changes: vec![],
            };
            for &(start, end) in &groups {
                let subset = self.with_stages(start..end);
                let gcode = self.emit_file(plan, profile, places, start..end)?;
                let decoded = subset.decode_program(plan, places, &gcode)?;
                let span = subset_motion_span(&subset);
                let expected = &plan.motions()[span.0..span.1];
                compare_motions(&subset, plan, &gcode, &decoded, expected, span.0)?;
                decoded_stream.motions.extend(decoded.motions);
                decoded_stream.bridges.extend(decoded.bridges);
                decoded_stream.tool_changes.extend(decoded.tool_changes);
                programs.push(SequenceProgram {
                    filename: bundle_filename(programs.len() + 1, &subset.stages[0], layout),
                    gcode,
                });
            }
            let replay = if has_knife {
                Some(crate::operations::drag_knife::evidence::replay_emitted(
                    plan,
                    self,
                    &decoded_stream,
                    crate::operations::drag_knife::evidence::EMITTED_REPLAY_STEP_BUDGET,
                )?)
            } else {
                None
            };
            let replay_ok = replay
                .as_ref()
                .is_none_or(|replay| replay.outcome.status == ReplayStatus::Within);
            if preserved && replay_ok {
                return Ok(self.finish_bundle(
                    plan,
                    layout,
                    programs,
                    &groups,
                    replay.map(|replay| KnifeReplaySummary {
                        status: match replay.outcome.status {
                            ReplayStatus::Within => KnifeReplayStatus::Within,
                            ReplayStatus::Exceeded => KnifeReplayStatus::Exceeded,
                            ReplayStatus::BudgetExhausted => KnifeReplayStatus::BudgetExhausted,
                        },
                        max_tip_deviation_mm: replay.outcome.max_tip_deviation_mm,
                        max_heading_error_deg: replay.outcome.max_heading_error_deg,
                        tip_budget_mm: replay.tip_budget_mm,
                    }),
                    places,
                ));
            }
            if places >= 9 {
                return Err(if !preserved {
                    error(
                        "POST_PRECISION",
                        "nine decimal places cannot represent every required motion; \
                         refusing to delete or collapse a required move",
                    )
                } else {
                    match replay.expect("replay present when !replay_ok") {
                        replay if replay.outcome.status == ReplayStatus::BudgetExhausted => error(
                            "KNIFE_REPLAY_BUDGET",
                            "the independent replay of the emitted knife program exhausted \
                             its integration budget; the result is inconclusive, not truncated",
                        ),
                        replay => error(
                            "KNIFE_TIP_ERROR",
                            format!(
                                "the replayed blade tip of the emitted program deviates {:.4} mm \
                                 from the intended tip path (budget {:.4} mm, heading error {:.3} deg); \
                                 the output precision cannot represent this program",
                                replay.outcome.max_tip_deviation_mm,
                                replay.tip_budget_mm,
                                replay.outcome.max_heading_error_deg
                            ),
                        ),
                    }
                });
            }
            places += 1;
        }
    }

    /// Build the manifest and aggregate report of fully checked files.
    fn finish_bundle(
        &self,
        plan: &dyn SequencePlan,
        layout: OutputLayout,
        programs: Vec<SequenceProgram>,
        groups: &[(usize, usize)],
        knife_replay: Option<KnifeReplaySummary>,
        places: usize,
    ) -> SequenceBundle {
        let mut entries = vec![];
        let mut previous_names = vec![];
        for (index, (program, &(start, end))) in programs.iter().zip(groups).enumerate() {
            let sha256 = format!("{:x}", Sha256::digest(program.gcode.as_bytes()));
            let stages = &self.stages[start..end];
            entries.push(BundleManifestFile {
                order: index + 1,
                filename: program.filename.clone(),
                sha256: sha256.clone(),
                byte_length: program.gcode.len(),
                stage_ids: stages.iter().map(|s| s.stage.stage_id.clone()).collect(),
                tool_number: stages[0].tool_number,
                motion_range: (
                    stages[0].stage.motion_range.0,
                    stages[stages.len() - 1].stage.motion_range.1,
                ),
                runs_after: previous_names.clone(),
                stock_through_operation: stages[stages.len() - 1].stage.operation_id.clone(),
            });
            previous_names.push(program.filename.clone());
        }
        // The aggregate digest binds the exact bytes: a single file is its
        // own digest (byte-identical behavior with the one-program export);
        // several files hash their ordered per-file digests.
        let program_sha256 = match programs.as_slice() {
            [one] => format!("{:x}", Sha256::digest(one.gcode.as_bytes())),
            many => {
                let mut hash = Sha256::new();
                for program in many {
                    hash.update(
                        format!("{:x}", Sha256::digest(program.gcode.as_bytes())).as_bytes(),
                    );
                }
                format!("{:x}", hash.finalize())
            }
        };
        let manifest = SequenceBundleManifest {
            artifact_kind: "sequence_export_manifest".into(),
            schema_version: 1,
            engine_version: env!("CARGO_PKG_VERSION").into(),
            layout,
            plan_fingerprint: self.plan_fingerprint.clone(),
            profile_fingerprint: self.profile_fingerprint.clone(),
            process_fingerprint: self.process_fingerprint.clone(),
            execution_fingerprint: plan.execution_fingerprint().to_string(),
            output_decimal_places: places,
            files: entries,
        };
        let report = SequenceExportReport {
            artifact_kind: "sequence_export_report".into(),
            schema_version: 1,
            engine_version: env!("CARGO_PKG_VERSION").into(),
            plan_fingerprint: self.plan_fingerprint.clone(),
            profile_fingerprint: self.profile_fingerprint.clone(),
            process_fingerprint: self.process_fingerprint.clone(),
            output_decimal_places: places,
            machine_offset_mm: self.machine_offset_mm,
            basic_checks: self.basic_checks.clone(),
            program_sha256,
            motion_count: plan.motions().len(),
            knife_replay,
            diagnostics: vec![],
        };
        SequenceBundle {
            layout,
            files: programs,
            manifest,
            report,
        }
    }

    /// Emit one program file covering `self.stages[range]` (H5): the shared
    /// preamble and modal establishment, the per-stage tool/process groups in
    /// order, and the shutdown tail. A sequential bundle file is a complete
    /// standalone program: its first stage departs from the declared program
    /// start exactly like the first stage of a one-program export.
    fn emit_file(
        &self,
        plan: &dyn SequencePlan,
        profile: &SequenceProfile,
        places: usize,
        range: std::ops::Range<usize>,
    ) -> Result<String> {
        let mut lines =
            vec![
            "(CAM sequence program; basic checks passed; detailed quality analysis is separate)"
                .into(),
            format!("(CAM Engine {}; profile {})", env!("CARGO_PKG_VERSION"), profile.id),
            format!("(CAM Plan {})", self.plan_fingerprint),
            format!("(CAM Process {})", self.process_fingerprint),
            "M5".into(),
            "M9".into(),
        ];
        lines.extend(modal_lines(&profile.work_offset, profile.path_control));
        let mut previous_stage_end = profile.program_start_position_mm;
        for prepared in &self.stages[range] {
            let stage = &prepared.stage;
            lines.push(format!(
                "(CAM Stage {}; tool {})",
                role_name(stage.role),
                stage.tool_id
            ));
            lines.push("M5".into());
            lines.push("M9".into());
            lines.push(format!("T{} M6", prepared.tool_number));
            // Explicit spindle-off and coolant-off state after the tool
            // change (plan section 14.4), then this stage's path-control
            // mode — exact path for knife stages even under a blend profile.
            lines.push("M5".into());
            lines.push("M9".into());
            lines.extend(modal_lines(
                &profile.work_offset,
                prepared.process.path_control,
            ));
            if profile.length_compensation == LengthCompensation::ToolTable {
                lines.push(format!(
                    "G43 H{}",
                    profile
                        .tool(&stage.tool_id)
                        .and_then(|t| t.length_offset_number)
                        .expect("validated H number")
                ));
            }
            match prepared.process.spindle {
                PreparedSpindle::Off => {}
                PreparedSpindle::On { rpm, direction } => {
                    lines.push(format!(
                        "{} S{}",
                        match direction {
                            SpindleDirection::Clockwise => "M3",
                            SpindleDirection::Counterclockwise => "M4",
                        },
                        scalar(rpm)?
                    ));
                    lines.push(format!("G4 P{}", profile.spindle_spinup_seconds));
                }
            }
            lines.push(
                match prepared.process.coolant {
                    Coolant::Off => "M9",
                    Coolant::Flood => "M8",
                    Coolant::Mist => "M7",
                }
                .into(),
            );
            // The M6 bridge is machine-owned; never fabricate a continuous
            // XYZ line through the macro (plan section 8.3). Emission and
            // the decoder's expected bridges share one helper, so only these
            // blocks can ever be classified as bridges.
            let entry = machine_position(
                first_motion(plan, stage).start,
                self.machine_offset_mm,
                places,
            );
            for bridge in &bridge_blocks(previous_stage_end, entry) {
                match *bridge {
                    BridgeBlock::ZOnly(z) => lines.push(format!("G0 Z{:.p$}", z, p = places)),
                    BridgeBlock::Full(p) => lines.push(format!("G0 {}", xyz(p, places))),
                }
            }
            let motions = &plan.motions()[stage.motion_range.0..stage.motion_range.1];
            for motion in motions {
                let end = machine_position(motion.end, self.machine_offset_mm, places);
                let feed = match motion.feed_mm_min {
                    Some(f) => format!(" F{}", scalar(f)?),
                    None => String::new(),
                };
                lines.push(format!(
                    "{} {}{}",
                    match motion.interpolation {
                        Interpolation::Rapid => "G0",
                        Interpolation::LinearFeed => "G1",
                    },
                    xyz(end, places),
                    feed
                ));
            }
            previous_stage_end = motions
                .last()
                .map(|m| machine_position(m.end, self.machine_offset_mm, places));
        }
        lines.extend(["M5".into(), "M9".into(), "M2".into()]);
        if lines.iter().any(|l| l.len() > 240) {
            return Err(error(
                "POST_LINE_LENGTH",
                "an emitted block exceeds the 240-character limit",
            ));
        }
        Ok(lines.join("\n") + "\n")
    }
}

/// Global `[first, end)` motion span of a (subset) preparation's stages.
fn subset_motion_span(prepared: &PreparedExecution) -> (usize, usize) {
    (
        prepared
            .stages
            .first()
            .expect("nonempty preparation")
            .stage
            .motion_range
            .0,
        prepared
            .stages
            .last()
            .expect("nonempty preparation")
            .stage
            .motion_range
            .1,
    )
}

/// Generated filename of a bundle file: order, the role of its first stage
/// and its tool number (plan section 14.5's `01-face-T1.ngc` family). The
/// one-program layout keeps the traditional single name.
fn bundle_filename(order: usize, first: &PreparedStage, layout: OutputLayout) -> String {
    match layout {
        OutputLayout::OneProgram => "sequence.ngc".into(),
        OutputLayout::SequentialFiles => format!(
            "{:02}-{}-T{}.ngc",
            order,
            role_name(first.stage.role).replace('_', "-"),
            first.tool_number
        ),
    }
}

/// Verify supplied output bytes against the prepared execution and plan:
/// altered motion order, coordinates, feeds, tools, modal state or process
/// words are rejected with `POST_SEQUENCE_MISMATCH` (and the specific
/// modal/process codes) — never silently ignored.
pub fn verify_program(
    prepared: &PreparedExecution,
    plan: &dyn TrustedSequencePlan,
    program: &SequenceProgram,
) -> Result<SequenceExportReport> {
    let plan = plan.trusted();
    let decoded = prepared.decode_program(plan, prepared.output_decimal_places, &program.gcode)?;
    compare_with_plan(prepared, plan, &program.gcode, &decoded)?;
    Ok(SequenceExportReport {
        artifact_kind: "sequence_export_report".into(),
        schema_version: 1,
        engine_version: env!("CARGO_PKG_VERSION").into(),
        plan_fingerprint: prepared.plan_fingerprint.clone(),
        profile_fingerprint: prepared.profile_fingerprint.clone(),
        process_fingerprint: prepared.process_fingerprint.clone(),
        output_decimal_places: prepared.output_decimal_places,
        machine_offset_mm: prepared.machine_offset_mm,
        basic_checks: prepared.basic_checks.clone(),
        program_sha256: format!("{:x}", Sha256::digest(program.gcode.as_bytes())),
        motion_count: decoded.motions.len(),
        knife_replay: None,
        diagnostics: vec![],
    })
}

/// Path-control state reconstructed from actual program bytes (F3a). The
/// G61/G61.1/G64 distinction matters: exact path, exact stop and blending
/// are different machine behaviors, and a prepared-state assertion or the
/// writer having emitted a word is not evidence that edited output still
/// carries it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DecodedPathControl {
    ExactPath,
    ExactStop,
    Blend {
        tolerance_mm: Option<f64>,
        naive_cam_tolerance_mm: Option<f64>,
    },
}
impl DecodedPathControl {
    fn matches(&self, expected: &PathControl) -> bool {
        match (*self, *expected) {
            (Self::ExactPath, PathControl::ExactPath) => true,
            (
                Self::Blend {
                    tolerance_mm,
                    naive_cam_tolerance_mm,
                },
                PathControl::Blend {
                    tolerance_mm: p,
                    naive_cam_tolerance_mm: q,
                },
            ) => tolerance_mm == Some(p) && naive_cam_tolerance_mm == q,
            // Exact stop (G61.1) is never a prepared mode in this release.
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DecodedSpindle {
    Off,
    On {
        rpm: Option<f64>,
        direction: SpindleDirection,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodedCoolant {
    Off,
    Flood,
    Mist,
}

/// One motion re-derived from the emitted bytes, with the modal/process
/// state that was in force while it executed and the source line it came
/// from (F3a: decoded motion start/end, feed, tool and state mapping).
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedMotion {
    pub interpolation: Interpolation,
    /// None on the first block after a tool change when no bridge block
    /// established a position: the M6 macro's internal motion is
    /// machine-owned and outside this model (plan section 8.3).
    pub start: Option<crate::motion::Position>,
    pub end: crate::motion::Position,
    pub feed: Option<f64>,
    pub tool_number: u32,
    pub spindle: DecodedSpindle,
    pub coolant: DecodedCoolant,
    pub path_control: DecodedPathControl,
    pub work_offset: String,
    pub line: usize,
}

/// A machine-owned positioning bridge read back from the bytes and checked
/// against the blocks the writer would have produced (F3a: bridges are
/// classified positioning that authorizes nothing, never planned motions).
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedBridge {
    /// `ZOnly(z)` rose to z at the current XY; `Full(p)` moved to p.
    pub block: BridgeBlock,
    pub line: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BridgeBlock {
    ZOnly(f64),
    Full(crate::motion::Position),
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedProgram {
    pub motions: Vec<DecodedMotion>,
    pub bridges: Vec<DecodedBridge>,
    /// One entry per tool change, in order, with its source line.
    pub tool_changes: Vec<(u32, usize)>,
}

/// The machine-owned bridge blocks between one position and a stage entry
/// (plan section 8.3): a Z-only block when the heights differ, then a full
/// XYZ block when the XY differs. Emission and expected-bridge derivation
/// share this so the decoder never classifies a block as a bridge that the
/// writer would not have written.
fn bridge_blocks(
    current: Option<crate::motion::Position>,
    entry: crate::motion::Position,
) -> Vec<BridgeBlock> {
    let Some(current) = current else {
        return vec![];
    };
    let mut blocks = vec![];
    if current.z != entry.z {
        blocks.push(BridgeBlock::ZOnly(entry.z));
    }
    if current.xy() != entry.xy() {
        blocks.push(BridgeBlock::Full(entry));
    }
    blocks
}

impl PreparedExecution {
    /// Expected machine bridge blocks per stage: stage 0 departs from the
    /// declared program start, later stages from the previous stage's last
    /// motion end.
    fn expected_bridges(&self, plan: &dyn SequencePlan, places: usize) -> Vec<Vec<BridgeBlock>> {
        let mut previous = self.program_start_position_mm;
        let mut expected = vec![];
        for stage in &self.stages {
            let entry = machine_position(
                plan.motions()[stage.stage.motion_range.0].start,
                self.machine_offset_mm,
                places,
            );
            expected.push(bridge_blocks(previous, entry));
            previous = Some(machine_position(
                plan.motions()[stage.stage.motion_range.1 - 1].end,
                self.machine_offset_mm,
                places,
            ));
        }
        expected
    }

    /// Decode actual program bytes into motions, bridges, tool changes and
    /// the modal/process state in force at every motion, rejecting every
    /// unsupported or unexpected state change instead of ignoring it (F3a).
    pub fn decode_program(
        &self,
        plan: &dyn SequencePlan,
        places: usize,
        gcode: &str,
    ) -> Result<DecodedProgram> {
        decode_program(gcode, self, &self.expected_bridges(plan, places))
    }

    /// Independently verify the exact bytes of one bundle file against the
    /// plan: the file must contain exactly the prepared stages in
    /// `stage_span` (global stage indices) and decode back to their slice
    /// of the ordered motion stream. Used by export itself and by callers
    /// that re-check retained bytes.
    pub fn verify_program_span(
        &self,
        plan: &dyn TrustedSequencePlan,
        program: &SequenceProgram,
        stage_span: (usize, usize),
    ) -> Result<()> {
        let plan = plan.trusted();
        let subset = self.with_stages(stage_span.0..stage_span.1);
        let decoded = subset.decode_program(plan, self.output_decimal_places, &program.gcode)?;
        let span = subset_motion_span(&subset);
        let expected = &plan.motions()[span.0..span.1];
        compare_motions(&subset, plan, &program.gcode, &decoded, expected, span.0)
    }
}

/// Independent numeric decode of LinuxCNC program bytes (F3a). Re-derives
/// every motion, bridge, tool change, modal group and process word from the
/// bytes alone; comments are skipped and never authorize anything. Any word
/// the writer would not have emitted — or any recognized mode changed to an
/// unsupported value — is rejected with a located error.
fn decode_program(
    gcode: &str,
    prepared: &PreparedExecution,
    expected_bridges: &[Vec<BridgeBlock>],
) -> Result<DecodedProgram> {
    if gcode.len() > 128_000_000 || !gcode.is_ascii() {
        return Err(error(
            "POST_GCODE_SUBSET",
            "program must be ASCII and at most 128 MB",
        ));
    }
    let mut motions: Vec<DecodedMotion> = vec![];
    let mut bridges: Vec<DecodedBridge> = vec![];
    let mut tool_changes: Vec<(u32, usize)> = vec![];
    // Modal groups start unknown; a motion before its group is established
    // is rejected rather than run under assumed defaults.
    let mut units_mm: Option<bool> = None;
    let mut plane_xy: Option<bool> = None;
    let mut distance_absolute: Option<bool> = None;
    let mut feed_per_minute: Option<bool> = None;
    let mut cutter_comp_off: Option<bool> = None;
    let mut cycles_off: Option<bool> = None;
    let mut path_control: Option<DecodedPathControl> = None;
    let mut work_offset: Option<String> = None;
    let mut spindle = DecodedSpindle::Off;
    let mut coolant = DecodedCoolant::Off;
    let mut tool: Option<u32> = None;
    let mut feed: Option<f64> = None;
    let mut length_comp: Option<Option<u32>> = None;
    let mut position: Option<crate::motion::Position> = None;
    let mut program_ended = false;
    // Bridge accounting per stage slot.
    let mut bridges_expected = 0usize;
    let mut bridges_seen = 0usize;
    let mut stage_started = false;

    let reject = |line: usize, code: &str, what: String| -> Diagnostic {
        error(code, format!("line {line}: {what}"))
    };

    for (index, raw) in gcode.lines().enumerate() {
        let line_number = index + 1;
        let line = raw.trim();
        if line.is_empty() || (line.starts_with('(') && line.ends_with(')')) {
            continue;
        }
        if line.len() > 240 {
            return Err(reject(
                line_number,
                "POST_GCODE_SUBSET",
                "line exceeds 240 characters".to_string(),
            ));
        }
        if program_ended {
            return Err(reject(
                line_number,
                "POST_GCODE_SUBSET",
                "block after the program end (M2)".into(),
            ));
        }
        if line.contains('(') {
            return Err(reject(
                line_number,
                "POST_GCODE_SUBSET",
                "inline comments are not part of the supported subset".into(),
            ));
        }
        // Parse the block into words first; only a known vocabulary may
        // continue into state tracking.
        let mut motion: Option<Interpolation> = None;
        let mut coordinates: Option<crate::motion::Position> = None;
        let mut words = 0u8;
        let mut block_feed: Option<f64> = None;
        let mut tool_word: Option<u32> = None;
        let mut change_tool = false;
        let mut spindle_command: Option<(bool, Option<f64>)> = None; // (start, S)
        for token in line.split_ascii_whitespace() {
            let letter = token.as_bytes()[0];
            let value = &token[1..];
            let numeric = || -> Option<f64> {
                value
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && *v >= -1_000_000. && *v <= 1_000_000.)
            };
            match letter {
                b'G' => match value {
                    "0" => motion = Some(Interpolation::Rapid),
                    "1" => motion = Some(Interpolation::LinearFeed),
                    "4" => {
                        // Dwell: P must be present and finite; nothing else.
                        let Some(rest) = line
                            .split_ascii_whitespace()
                            .find(|w| w.starts_with('P'))
                            .and_then(|w| w[1..].parse::<f64>().ok())
                            .filter(|p| p.is_finite() && *p >= 0.)
                        else {
                            return Err(reject(
                                line_number,
                                "POST_GCODE_SUBSET",
                                "G4 requires a finite nonnegative P".into(),
                            ));
                        };
                        let _ = rest;
                    }
                    "17" => plane_xy = Some(true),
                    "18" | "19" => plane_xy = Some(false),
                    "20" => units_mm = Some(false),
                    "21" => units_mm = Some(true),
                    "40" => cutter_comp_off = Some(true),
                    "41" | "42" => cutter_comp_off = Some(false),
                    "43" => {
                        let h = line
                            .split_ascii_whitespace()
                            .find(|w| w.starts_with('H'))
                            .and_then(|w| w[1..].parse::<u32>().ok());
                        if length_comp.is_some() && length_comp != Some(h) {
                            return Err(reject(
                                line_number,
                                "POST_MODAL_STATE",
                                "length compensation changed within a stage".into(),
                            ));
                        }
                        length_comp = Some(h);
                    }
                    "61" => path_control = Some(DecodedPathControl::ExactPath),
                    "61.1" => path_control = Some(DecodedPathControl::ExactStop),
                    "64" => {
                        let tolerance = line
                            .split_ascii_whitespace()
                            .find(|w| w.starts_with('P'))
                            .and_then(|w| w[1..].parse::<f64>().ok())
                            .filter(|v| v.is_finite() && *v > 0.);
                        let naive = line
                            .split_ascii_whitespace()
                            .find(|w| w.starts_with('Q'))
                            .and_then(|w| w[1..].parse::<f64>().ok())
                            .filter(|v| v.is_finite() && *v > 0.);
                        path_control = Some(DecodedPathControl::Blend {
                            tolerance_mm: tolerance,
                            naive_cam_tolerance_mm: naive,
                        });
                    }
                    "80" => cycles_off = Some(true),
                    "90" => distance_absolute = Some(true),
                    "91" => distance_absolute = Some(false),
                    "92.1" => {} // clears leftover G92 offsets; the writer emits it
                    "94" => feed_per_minute = Some(true),
                    "93" => feed_per_minute = Some(false),
                    "54" | "55" | "56" | "57" | "58" | "59" | "59.1" | "59.2" | "59.3" => {
                        work_offset = Some(format!("G{value}"));
                    }
                    _ => {
                        return Err(reject(
                            line_number,
                            "POST_MODAL_STATE",
                            format!("unsupported G-code {token} changes an unsupported mode"),
                        ));
                    }
                },
                b'M' => match value {
                    "2" => program_ended = true,
                    "3" | "4" => {
                        let rpm = line
                            .split_ascii_whitespace()
                            .find(|w| w.starts_with('S'))
                            .and_then(|w| w[1..].parse::<f64>().ok())
                            .filter(|v| v.is_finite() && *v > 0.);
                        spindle_command = Some((true, rpm));
                        if value == "3" {
                            spindle = DecodedSpindle::On {
                                rpm,
                                direction: SpindleDirection::Clockwise,
                            };
                        } else {
                            spindle = DecodedSpindle::On {
                                rpm,
                                direction: SpindleDirection::Counterclockwise,
                            };
                        }
                    }
                    "5" => spindle = DecodedSpindle::Off,
                    "6" => change_tool = true,
                    "7" => coolant = DecodedCoolant::Mist,
                    "8" => coolant = DecodedCoolant::Flood,
                    "9" => coolant = DecodedCoolant::Off,
                    _ => {
                        return Err(reject(
                            line_number,
                            "POST_GCODE_SUBSET",
                            format!("unsupported M-code {token}"),
                        ));
                    }
                },
                b'T' => match value.parse::<u32>() {
                    Ok(number) if (1..=99999).contains(&number) => tool_word = Some(number),
                    _ => {
                        return Err(reject(
                            line_number,
                            "POST_GCODE_SUBSET",
                            format!("invalid tool number {token}"),
                        ));
                    }
                },
                b'S' => {
                    // S is only meaningful on this release's M3/M4 blocks.
                    if spindle_command.is_none() {
                        return Err(reject(
                            line_number,
                            "POST_GCODE_SUBSET",
                            "a lone S word is not part of the supported subset".into(),
                        ));
                    }
                }
                b'F' => match numeric() {
                    Some(value) if value > 0. => block_feed = Some(value),
                    _ => {
                        return Err(reject(
                            line_number,
                            "POST_GCODE_SUBSET",
                            format!("invalid feed {token}"),
                        ));
                    }
                },
                b'X' | b'Y' | b'Z' => {
                    let Some(number) = numeric() else {
                        return Err(reject(
                            line_number,
                            "POST_GCODE_SUBSET",
                            format!("invalid coordinate {token}"),
                        ));
                    };
                    if motion.is_none() {
                        return Err(reject(
                            line_number,
                            "POST_GCODE_SUBSET",
                            "coordinates outside a G0/G1 block".into(),
                        ));
                    }
                    words |= match letter {
                        b'X' => 1,
                        b'Y' => 2,
                        _ => 4,
                    };
                    let slot = coordinates.get_or_insert(crate::motion::Position::new(
                        crate::geometry::Point::new(0., 0.),
                        0.,
                    ));
                    match letter {
                        b'X' => slot.x = number,
                        b'Y' => slot.y = number,
                        _ => slot.z = number,
                    }
                }
                b'P' | b'Q' | b'H' => {} // consumed by their G words above
                _ => {
                    return Err(reject(
                        line_number,
                        "POST_GCODE_SUBSET",
                        format!("unsupported word {token}"),
                    ));
                }
            }
        }
        if let Some(value) = block_feed {
            feed = Some(value);
        }
        if change_tool {
            let number = tool_word.ok_or_else(|| {
                reject(
                    line_number,
                    "POST_GCODE_SUBSET",
                    "M6 without a T number".into(),
                )
            })?;
            if stage_started && bridges_seen < bridges_expected {
                return Err(reject(
                    line_number,
                    "POST_SEQUENCE_MISMATCH",
                    format!(
                        "stage {} wrote {} of {} machine bridge blocks",
                        tool_changes.len(),
                        bridges_seen,
                        bridges_expected
                    ),
                ));
            }
            tool_changes.push((number, line_number));
            tool = Some(number);
            // The M6 bridge itself is machine-owned: modal position and
            // process state survive it, and this model never fabricates a
            // continuous line through the macro.
            position = None;
            length_comp = None;
            stage_started = false;
            bridges_seen = 0;
            bridges_expected = expected_bridges
                .get(tool_changes.len() - 1)
                .map_or(0, Vec::len);
        }
        let Some(interpolation) = motion else {
            if let Some((true, rpm)) = spindle_command
                && rpm.is_none()
            {
                return Err(reject(
                    line_number,
                    "POST_MODAL_STATE",
                    "spindle start without a spindle speed".into(),
                ));
            }
            continue;
        };
        let Some(end) = coordinates else {
            return Err(reject(
                line_number,
                "POST_GCODE_SUBSET",
                "motion block without coordinates".into(),
            ));
        };
        if words != 7 {
            // Only the writer's Z-only bridge form may carry fewer than all
            // three axes, and only where a bridge is expected.
            let slot = tool_changes.len().checked_sub(1).ok_or_else(|| {
                reject(
                    line_number,
                    "POST_SEQUENCE_MISMATCH",
                    "motion before any tool change".into(),
                )
            })?;
            let expected = expected_bridges.get(slot).map_or(&[][..], Vec::as_slice);
            let is_z_bridge = words == 4
                && !stage_started
                && bridges_seen < expected.len()
                && matches!(
                    expected[bridges_seen],
                    BridgeBlock::ZOnly(z) if end.z == z
                );
            if !is_z_bridge {
                return Err(reject(
                    line_number,
                    "POST_GCODE_SUBSET",
                    "partial-coordinate motion block".into(),
                ));
            }
        }
        // Every modal group a motion depends on must already be established
        // in a supported state (F3a: no assumed defaults from preparation).
        let mut missing = vec![];
        if units_mm != Some(true) {
            missing.push("G21 mm units");
        }
        if plane_xy != Some(true) {
            missing.push("G17 XY plane");
        }
        if distance_absolute != Some(true) {
            missing.push("G90 absolute distance");
        }
        if feed_per_minute != Some(true) {
            missing.push("G94 units-per-minute feed");
        }
        if cutter_comp_off != Some(true) {
            missing.push("G40 cutter compensation off");
        }
        if cycles_off != Some(true) {
            missing.push("G80 canned cycles off");
        }
        if path_control.is_none() {
            missing.push("a path-control mode (G61/G64)");
        }
        let decoded_work = work_offset.clone().ok_or_else(|| {
            reject(
                line_number,
                "POST_MODAL_STATE",
                "motion before a work frame (G54..G59.3) is selected".into(),
            )
        })?;
        if !missing.is_empty() {
            return Err(reject(
                line_number,
                "POST_MODAL_STATE",
                format!("motion before {} established", missing.join(", ")),
            ));
        }
        // The path-control group is established here (the missing check
        // above already rejected the unset case).
        let decoded_path = path_control
            .ok_or_else(|| reject(line_number, "POST_MODAL_STATE", "no path control".into()))?;
        if decoded_work != prepared.work_offset {
            return Err(reject(
                line_number,
                "POST_MODAL_STATE",
                format!(
                    "work frame {decoded_work} does not match the prepared {}",
                    prepared.work_offset
                ),
            ));
        }
        let tool_number = tool.ok_or_else(|| {
            reject(
                line_number,
                "POST_SEQUENCE_MISMATCH",
                "motion before any tool change".into(),
            )
        })?;
        // Machine-owned bridges: only immediately after the M6 group, only
        // G0, only exactly the blocks the writer would have produced, in
        // order, and only before this stage's first planned motion. A rapid
        // that happens to end at a stage entry excuses nothing by itself.
        if !stage_started && interpolation == Interpolation::Rapid {
            let slot = tool_changes.len() - 1;
            let expected = expected_bridges.get(slot).map_or(&[][..], Vec::as_slice);
            if bridges_seen < expected.len() {
                let matches = match expected[bridges_seen] {
                    BridgeBlock::ZOnly(z) => words == 4 && end.z == z,
                    BridgeBlock::Full(p) => words == 7 && end == p,
                };
                if matches {
                    bridges.push(DecodedBridge {
                        block: expected[bridges_seen],
                        line: line_number,
                    });
                    bridges_seen += 1;
                    let carried_xy = position.map_or((0., 0.), |p| (p.x, p.y));
                    position = Some(match expected[bridges_seen - 1] {
                        BridgeBlock::ZOnly(z) => crate::motion::Position {
                            x: carried_xy.0,
                            y: carried_xy.1,
                            z,
                        },
                        BridgeBlock::Full(p) => p,
                    });
                    continue;
                }
            }
        }
        // Length compensation expectations for the stage this motion runs in.
        let stage_slot = stage_slot_of(prepared, motions.len());
        let stage = prepared.stages.get(stage_slot).ok_or_else(|| {
            reject(
                line_number,
                "POST_SEQUENCE_MISMATCH",
                "more motions than the stages account for".into(),
            )
        })?;
        match prepared.length_compensation {
            LengthCompensation::ToolTable => {
                if length_comp != Some(stage.length_offset_number) {
                    return Err(reject(
                        line_number,
                        "POST_MODAL_STATE",
                        format!(
                            "stage '{}' must run under its G43 H{} length compensation",
                            stage.stage.stage_id,
                            stage
                                .length_offset_number
                                .map_or_else(|| "?".into(), |h| h.to_string())
                        ),
                    ));
                }
            }
            LengthCompensation::MacroManaged => {
                if length_comp.is_some() {
                    return Err(reject(
                        line_number,
                        "POST_MODAL_STATE",
                        "the macro-managed length-compensation contract forbids G43 in the program"
                            .into(),
                    ));
                }
            }
        }
        // Process state at the motion (F3a): decoded words against the
        // prepared stage, never the other way around.
        let expected_spindle = stage.process.spindle;
        match (spindle, expected_spindle) {
            (DecodedSpindle::Off, PreparedSpindle::Off) => {}
            (
                DecodedSpindle::On { rpm, direction },
                PreparedSpindle::On {
                    rpm: expected_rpm,
                    direction: expected_direction,
                },
            ) => {
                if rpm != Some(expected_rpm) || direction != expected_direction {
                    return Err(reject(
                        line_number,
                        "PROCESS_SPINDLE_STATE",
                        format!(
                            "spindle runs {:?} at S{} while stage '{}' prepared {} rpm",
                            direction,
                            rpm.map_or_else(|| "?".into(), |v| v.to_string()),
                            stage.stage.stage_id,
                            expected_rpm,
                        ),
                    ));
                }
            }
            (decoded, _) => {
                return Err(reject(
                    line_number,
                    "PROCESS_SPINDLE_STATE",
                    format!(
                        "spindle {} while cutting stage '{}'",
                        if matches!(decoded, DecodedSpindle::Off) {
                            "is off"
                        } else {
                            "runs"
                        },
                        stage.stage.stage_id
                    ),
                ));
            }
        }
        let coolant_ok = (coolant, stage.process.coolant) == (DecodedCoolant::Off, Coolant::Off)
            || (coolant, stage.process.coolant) == (DecodedCoolant::Flood, Coolant::Flood)
            || (coolant, stage.process.coolant) == (DecodedCoolant::Mist, Coolant::Mist);
        if !coolant_ok {
            return Err(reject(
                line_number,
                "PROCESS_COOLANT_STATE",
                format!(
                    "coolant {:?} during stage '{}'",
                    coolant, stage.stage.stage_id
                ),
            ));
        }
        if !decoded_path.matches(&stage.process.path_control) {
            return Err(reject(
                line_number,
                "POST_PATH_CONTROL_STATE",
                format!(
                    "path-control state {:?} at stage '{}' does not match the prepared mode",
                    decoded_path, stage.stage.stage_id
                ),
            ));
        }
        motions.push(DecodedMotion {
            interpolation,
            start: position,
            end,
            feed,
            tool_number,
            spindle,
            coolant,
            path_control: decoded_path,
            work_offset: decoded_work,
            line: line_number,
        });
        position = Some(end);
        stage_started = true;
    }
    if !program_ended {
        return Err(error("POST_GCODE_SUBSET", "program does not end with M2"));
    }
    if stage_started && bridges_seen < bridges_expected {
        return Err(error(
            "POST_SEQUENCE_MISMATCH",
            format!(
                "stage {} wrote {} of {} machine bridge blocks",
                tool_changes.len() - 1,
                bridges_seen,
                bridges_expected
            ),
        ));
    }
    // Tool changes must match the stages in order, one per stage.
    let expected_tools: Vec<u32> = prepared.stages.iter().map(|s| s.tool_number).collect();
    let decoded_tools: Vec<u32> = tool_changes.iter().map(|(t, _)| *t).collect();
    if decoded_tools != expected_tools {
        return Err(error(
            "POST_SEQUENCE_MISMATCH",
            format!(
                "tool changes {decoded_tools:?} do not match the ordered stage tools {expected_tools:?}"
            ),
        ));
    }
    Ok(DecodedProgram {
        motions,
        bridges,
        tool_changes,
    })
}

/// Which prepared stage a motion belongs to, by cumulative motion counts;
/// motions of one stage are contiguous in the emitted stream.
fn stage_slot_of(prepared: &PreparedExecution, motions_so_far: usize) -> usize {
    let mut consumed = 0usize;
    for (index, stage) in prepared.stages.iter().enumerate() {
        consumed += stage.stage.motion_range.1 - stage.stage.motion_range.0;
        if motions_so_far < consumed {
            return index;
        }
    }
    prepared.stages.len()
}

/// Compare decoded motions with the plan's motions in order: positions at
/// output precision, interpolation, feed and per-stage tool ownership.
/// Modal/process state was already verified per motion during decoding.
fn compare_with_plan(
    prepared: &PreparedExecution,
    plan: &dyn SequencePlan,
    gcode: &str,
    decoded: &DecodedProgram,
) -> Result<()> {
    compare_motions(prepared, plan, gcode, decoded, plan.motions(), 0)
}

/// Compare decoded motions with an expected slice of the plan's motions in
/// order: positions at output precision, interpolation, feed and per-stage
/// tool ownership. `offset` is the global motion index the decoded stream
/// starts at, so a sequential bundle file compares against exactly its own
/// slice of the plan while every check stays identical.
fn compare_motions(
    prepared: &PreparedExecution,
    plan: &dyn SequencePlan,
    gcode: &str,
    decoded: &DecodedProgram,
    expected: &[crate::toolpath::PlannedMotion],
    offset: usize,
) -> Result<()> {
    let motions = &decoded.motions;
    if motions.len() != expected.len() {
        return Err(error(
            "POST_SEQUENCE_MISMATCH",
            format!(
                "read back {} motions for {} planned (from {} bytes)",
                motions.len(),
                expected.len(),
                gcode.len()
            ),
        ));
    }
    for (index, (read, planned)) in motions.iter().zip(expected.iter()).enumerate() {
        let global = offset + index;
        let expected_end = machine_position(
            planned.end,
            prepared.machine_offset_mm,
            prepared.output_decimal_places,
        );
        if read.end != expected_end {
            return Err(error(
                "POST_SEQUENCE_MISMATCH",
                format!(
                    "motion {global} readback end {:?} differs from planned {expected_end:?}",
                    read.end
                ),
            ));
        }
        if read.interpolation != planned.interpolation {
            return Err(error(
                "POST_SEQUENCE_MISMATCH",
                format!("motion {global} interpolation differs from the plan"),
            ));
        }
        // Feeds are modal words that rapids do not consume; only linear feed
        // motions carry an authoritative per-motion feed to compare.
        if read.interpolation == Interpolation::LinearFeed && read.feed != planned.feed_mm_min {
            return Err(error(
                "POST_SEQUENCE_MISMATCH",
                format!("motion {global} feed differs from the plan"),
            ));
        }
        let stage = plan
            .stages()
            .iter()
            .find(|s| global >= s.motion_range.0 && global < s.motion_range.1)
            .ok_or_else(|| {
                error(
                    "POST_SEQUENCE_MISMATCH",
                    format!("motion {global} outside all stages"),
                )
            })?;
        let tool_number = prepared
            .stages
            .iter()
            .find(|s| s.stage.stage_id == stage.stage_id)
            .map(|s| s.tool_number)
            .expect("prepared stage exists");
        if read.tool_number != tool_number {
            return Err(error(
                "POST_SEQUENCE_MISMATCH",
                format!(
                    "motion {global} runs with T{} under stage '{}' (T{tool_number})",
                    read.tool_number, stage.stage_id
                ),
            ));
        }
    }
    Ok(())
}

fn role_name(role: StageRole) -> &'static str {
    match role {
        StageRole::Face => "face",
        StageRole::ProfileRough => "profile_rough",
        StageRole::ProfileFinish => "profile_finish",
        StageRole::VcarveRough => "vcarve_rough",
        StageRole::VcarveFinish => "vcarve_finish",
        StageRole::Knife => "knife",
    }
}

fn modal_lines(work_offset: &str, path_control: PathControl) -> Vec<String> {
    let path_mode = match path_control {
        PathControl::ExactPath => "G61".to_string(),
        PathControl::Blend {
            tolerance_mm,
            naive_cam_tolerance_mm,
        } => match naive_cam_tolerance_mm {
            Some(q) => format!("G64 P{tolerance_mm} Q{q}"),
            None => format!("G64 P{tolerance_mm}"),
        },
    };
    vec![
        format!("G21 G17 G90 G94 G40 G80 {path_mode}"),
        work_offset.to_string(),
        "G92.1".into(),
    ]
}

fn first_motion<'a>(plan: &'a dyn SequencePlan, stage: &ExecutionStage) -> &'a PlannedMotion {
    &plan.motions()[stage.motion_range.0]
}

fn rounded_position(p: crate::motion::Position, places: usize) -> crate::motion::Position {
    crate::motion::Position {
        x: rounded(p.x, places),
        y: rounded(p.y, places),
        z: rounded(p.z, places),
    }
}
/// Setup-to-machine output position: work-zero Z datum shift, then rounding.
fn machine_position(
    p: crate::motion::Position,
    offset: [f64; 3],
    places: usize,
) -> crate::motion::Position {
    crate::motion::Position {
        x: rounded(p.x - offset[0], places),
        y: rounded(p.y - offset[1], places),
        z: rounded(p.z - offset[2], places),
    }
}
fn rounded(v: f64, places: usize) -> f64 {
    format!("{v:.places$}")
        .parse()
        .expect("finite formatted number")
}
fn xyz(p: crate::motion::Position, places: usize) -> String {
    format!("X{:.p$} Y{:.p$} Z{:.p$}", p.x, p.y, p.z, p = places)
}
fn scalar(v: f64) -> Result<String> {
    if !v.is_finite() || !(0.000001..=1_000_000.).contains(&v) {
        return Err(error(
            "POST_NUMBER_RANGE",
            "feeds and spindle speeds must be within 0.000001..=1000000",
        ));
    }
    Ok(v.to_string())
}

fn motions_preserved(plan: &dyn SequencePlan, places: usize) -> bool {
    plan.motions().iter().all(|m| {
        let a = rounded_position(m.start, places);
        let b = rounded_position(m.end, places);
        // A motion must survive formatting as a real move; pure-Z and pure-XY
        // moves only need their changing axis to survive.
        a != b
    }) && plan.motions().windows(2).all(|w| {
        // A zero-length formatted block cannot be distinguished from collapse;
        // require distinct endpoints for every motion at this precision.
        rounded_position(w[0].start, places) != rounded_position(w[0].end, places)
            && rounded_position(w[1].start, places) != rounded_position(w[1].end, places)
    })
}
