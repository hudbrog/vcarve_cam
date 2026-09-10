//! Ordered operation plans: stages, execution items and process state.
//!
//! `plan_job` assembles the enabled operations of a [`crate::project::CamJob`]
//! in document order into one executable plan. Slice A2 supports the Flat
//! V-carve compatibility adapter against original stock-top geometry; later
//! slices add stock history, face/profile/knife planners and export.
use crate::{
    geometry::{Diagnostic, Result},
    project::{CamJob, OperationSettings, SpindleDirection},
    toolpath::{Interpolation, MotionEffect, MotionPurpose, PlannedMotion},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const OPERATION_PLAN_ARTIFACT_KIND: &str = "operation_plan";
pub const OPERATION_PLAN_SCHEMA_VERSION: u32 = 1;

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("sequence")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageRole {
    Face,
    ProfileRough,
    ProfileFinish,
    VcarveRough,
    VcarveFinish,
    Knife,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProcessSpindle {
    Off,
    Milling {
        rpm: f64,
        /// Unset for migrated legacy assignments until a machine profile is
        /// applied; export preparation must resolve it before emitting M3/M4.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        direction: Option<SpindleDirection>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoolantIntent {
    Off,
    UseMachineProfile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathControlIntent {
    UseMachineProfile,
    ExactPath,
}

/// Requested process state for a stage. Unresolved legacy fields are legal
/// here; only a prepared (fully resolved) state may reach the writer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIntent {
    pub spindle: ProcessSpindle,
    pub coolant: CoolantIntent,
    pub path_control: PathControlIntent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ExecutionItem {
    ToolChange { tool_id: String },
    SetProcessIntent { intent: ProcessIntent },
    RunStage { stage_id: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ExecutionStage {
    pub stage_id: String,
    pub operation_id: String,
    pub tool_id: String,
    pub role: StageRole,
    /// Global motion index range `[first, end)`.
    pub motion_range: (usize, usize),
    pub entry_position: crate::motion::Position,
    pub exit_position: crate::motion::Position,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationStatus {
    Complete,
    Empty,
    Incomplete,
    Inconclusive,
}

/// Names of outputs an operation publishes for later height references
/// (e.g. a face plane). Payloads arrive with the face milestone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct NamedOutput {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_operation_id: Option<String>,
    /// Face plane payload: the established Z and the covered rectangle
    /// (plan section 6.3). Other kinds add their own payloads later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub covered: Option<crate::project::RectXY>,
}

/// Legacy planner evidence retained per adapted operation so candidate and
/// execution data stay inspectable without colliding global motion IDs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LegacyStageEvidence {
    pub stage_id: String,
    pub legacy_first_motion_id: usize,
    pub legacy_end_motion_id: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct LegacyPassEvidence {
    pub pass_id: usize,
    pub first_motion_id: usize,
    pub end_motion_id: usize,
    pub pass_depth_mm: f64,
    pub final_finish: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct OperationResult {
    pub operation_id: String,
    pub generation_status: GenerationStatus,
    pub stage_ids: Vec<String>,
    /// Identity of the stock view this operation was planned against and the
    /// view its recorded sweeps produce. Prefix history arrives in B2.
    pub stock_before_id: String,
    pub stock_after_id: String,
    #[serde(default)]
    pub named_outputs: Vec<NamedOutput>,
    #[serde(default)]
    pub legacy_stage_evidence: Vec<LegacyStageEvidence>,
    #[serde(default)]
    pub legacy_pass_evidence: Vec<LegacyPassEvidence>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PlanIssue {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
}

/// A process field that must be resolved before export preparation.
/// Requirements are facts about the plan, not executable state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PreparationRequirement {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationPlan {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub engine_version: String,
    pub job_snapshot: CamJob,
    pub input_fingerprint: String,
    pub execution_fingerprint: String,
    pub operation_results: Vec<OperationResult>,
    pub stages: Vec<ExecutionStage>,
    pub motions: Vec<PlannedMotion>,
    pub execution: Vec<ExecutionItem>,
    #[serde(default)]
    pub preparation_requirements: Vec<PreparationRequirement>,
    #[serde(default)]
    pub generation_diagnostics: Vec<PlanIssue>,
}

/// Aggregate job resource limits (plan section 17). Checked before allocation
/// so a limit failure cannot leave a truncated "successful" plan behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanLimits {
    pub max_operations: usize,
    pub max_stages: usize,
    pub max_motions: usize,
}
impl Default for PlanLimits {
    fn default() -> Self {
        Self {
            max_operations: 64,
            max_stages: 256,
            max_motions: 1_000_000,
        }
    }
}

/// A plan whose execution this engine has re-established. Only [`OperationPlan::plan_job`]
/// and verified imports create it; export preparation accepts nothing else, so
/// a serialized plan or receipt cannot grant itself export eligibility
/// (plan section 14.6).
pub struct TrustedPlan(OperationPlan);
impl std::fmt::Debug for TrustedPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustedPlan")
            .field("execution_fingerprint", &self.0.execution_fingerprint)
            .finish_non_exhaustive()
    }
}
impl TrustedPlan {
    pub fn plan(&self) -> &OperationPlan {
        &self.0
    }
    /// Import a serialized operation plan by reloading its embedded job and
    /// replanning with the current engine. Any edit to motions, stages,
    /// execution order or fingerprints is rejected; old engine versions are
    /// rejected because their evidence cannot be reconstructed.
    pub fn import(json: &str) -> Result<Self> {
        if json.len() > 512_000_000 {
            return Err(error(
                "PLAN_RESOURCE_LIMIT",
                "plan exceeds the 512 MB input limit",
            ));
        }
        let raw: OperationPlan =
            serde_json::from_str(json).map_err(|e| error("PLAN_JSON", e.to_string()))?;
        if raw.artifact_kind != OPERATION_PLAN_ARTIFACT_KIND
            || raw.schema_version != OPERATION_PLAN_SCHEMA_VERSION
        {
            return Err(error(
                "PLAN_IMPORT",
                "not an operation plan of the supported schema",
            ));
        }
        if raw.engine_version != env!("CARGO_PKG_VERSION") {
            return Err(error(
                "PLAN_IMPORT_ENGINE",
                "plans from other engine versions must be regenerated; their fingerprints and evidence are not transferable",
            ));
        }
        let replanned = OperationPlan::plan_job(&raw.job_snapshot, &PlanLimits::default())?;
        if raw.input_fingerprint != replanned.input_fingerprint
            || raw.execution_fingerprint != replanned.execution_fingerprint
            || raw.motions != replanned.motions
            || raw.stages != replanned.stages
            || raw.execution != replanned.execution
            || raw.operation_results != replanned.operation_results
        {
            return Err(error(
                "PLAN_IMPORT_MISMATCH",
                "the supplied plan does not match a regeneration of its own job; it may be inspected but cannot be exported",
            ));
        }
        Ok(Self(replanned))
    }
    /// Verify an in-process plan the same way an import is verified.
    pub fn from_plan(plan: &OperationPlan) -> Result<Self> {
        let replanned = OperationPlan::plan_job(&plan.job_snapshot, &PlanLimits::default())?;
        if plan.execution_fingerprint != replanned.execution_fingerprint
            || plan.motions != replanned.motions
            || plan.stages != replanned.stages
            || plan.execution != replanned.execution
        {
            return Err(error(
                "PLAN_IMPORT_MISMATCH",
                "the plan does not match a regeneration of its own job",
            ));
        }
        Ok(Self(replanned))
    }
    /// Bind a plan this engine just generated in this process. Provenance is
    /// the trust anchor; generation completeness and export readiness are
    /// checked where they matter (basic checks, export preparation).
    pub fn from_generated(plan: OperationPlan) -> Self {
        Self(plan)
    }
}

/// One operation's locally-indexed planning output before global assembly.
pub(crate) struct LocalStage {
    pub stage_id: String,
    pub role: StageRole,
    pub tool_id: String,
    /// Local motion range `[first, end)` within the operation.
    pub motion_range: (usize, usize),
    pub intent: ProcessIntent,
}

pub(crate) struct PlannedOperation {
    pub status: GenerationStatus,
    pub stages: Vec<LocalStage>,
    pub motions: Vec<PlannedMotion>,
    pub named_outputs: Vec<NamedOutput>,
    pub stage_evidence: Vec<LegacyStageEvidence>,
    pub pass_evidence: Vec<LegacyPassEvidence>,
    pub issues: Vec<PlanIssue>,
    pub preparation: Vec<PreparationRequirement>,
}

impl OperationPlan {
    /// Plan every enabled operation of `job` in document order.
    pub fn plan_job(job: &CamJob, limits: &PlanLimits) -> Result<Self> {
        job.validate()?;
        let enabled: Vec<_> = job.operations.iter().filter(|op| op.enabled).collect();
        if enabled.len() > limits.max_operations {
            return Err(error(
                "PLAN_RESOURCE_LIMIT",
                format!(
                    "{} enabled operations exceed the limit of {}",
                    enabled.len(),
                    limits.max_operations
                ),
            ));
        }
        if let Some(op) = enabled.iter().find(|op| {
            !matches!(
                op.settings,
                OperationSettings::FlatVcarve(_)
                    | OperationSettings::Face(_)
                    | OperationSettings::Profile(_)
            )
        }) {
            return Err(error(
                "OPERATION_PLANNER_UNAVAILABLE",
                format!(
                    "operation '{}' uses a '{}' operation whose planner ships in a later slice; \
                     remove or disable it instead of silently skipping it",
                    op.id,
                    kind_name(&op.settings)
                ),
            ));
        }
        let mut operation_results = vec![];
        let mut stages = vec![];
        let mut motions = vec![];
        let mut execution = vec![];
        let mut generation_diagnostics = vec![];
        let mut preparation_requirements = vec![];
        let mut planned_ids = BTreeSet::new();
        let mut published_faces = std::collections::BTreeMap::new();
        let mut current_tool: Option<String> = None;
        // Prefix stock identities (plan section 9.2): the initial view hashes
        // stock/artwork/planning inputs once; each operation extends it with
        // its settings, the tool snapshots it used and its actual execution,
        // so editing an earlier operation stales every later stock id.
        let mut stock_before = crate::plan_hash::hash(&(
            "stock-initial",
            OPERATION_PLAN_ARTIFACT_KIND,
            OPERATION_PLAN_SCHEMA_VERSION,
            env!("CARGO_PKG_VERSION"),
            &job.setup.stock,
            job.source.as_ref().map(|s| s.svg.as_str()),
            &job.import,
            &job.tolerances,
            &job.tools
                .iter()
                .map(|tool| (&tool.id, &tool.geometry, &tool.capabilities))
                .collect::<Vec<_>>(),
        ))
        .map_err(|e| error("PLAN_JSON", e.to_string()))?;
        for op in enabled {
            if !planned_ids.insert(op.id.clone()) {
                return Err(error(
                    "PLAN_OPERATION_ID",
                    "enabled operation IDs must be unique",
                ));
            }
            if stages.len() + 1 > limits.max_stages || motions.len() >= limits.max_motions {
                return Err(error(
                    "PLAN_RESOURCE_LIMIT",
                    "job exceeds the stage or motion budget before completing all operations",
                ));
            }
            let planned = crate::operations::plan_operation(job, op, &published_faces, &motions)?;
            let base = motions.len();
            if base + planned.motions.len() > limits.max_motions {
                return Err(error(
                    "PLAN_RESOURCE_LIMIT",
                    format!("operation '{}' exceeds the remaining motion budget", op.id),
                ));
            }
            let executed_snapshot: Vec<(
                usize,
                crate::motion::Position,
                crate::motion::Position,
                crate::toolpath::Interpolation,
                crate::toolpath::MotionEffect,
                Option<f64>,
            )> = motions[base..]
                .iter()
                .map(|m: &crate::toolpath::PlannedMotion| {
                    (
                        m.id,
                        m.start,
                        m.end,
                        m.interpolation,
                        m.effect,
                        m.feed_mm_min,
                    )
                })
                .collect();
            let stage_snapshot: Vec<_> = planned
                .stages
                .iter()
                .map(|s| (&s.stage_id, s.role, &s.tool_id))
                .collect();
            let stock_after = crate::plan_hash::hash(&(
                &stock_before,
                &op.id,
                &op.settings,
                &job.tools
                    .iter()
                    .filter(|tool| tool_ids_of(&op.settings).contains(&tool.id.as_str()))
                    .map(|tool| (&tool.id, &tool.geometry, &tool.capabilities))
                    .collect::<Vec<_>>(),
                &stage_snapshot,
                &executed_snapshot,
            ))
            .map_err(|e| error("PLAN_JSON", e.to_string()))?;
            for mut motion in planned.motions {
                motion.id += base;
                motions.push(motion);
            }
            // Spindle direction stays unresolved for migrated assignments;
            // preparing executable process state requires resolving it first.
            for stage in &planned.stages {
                if let ProcessSpindle::Milling {
                    direction: None, ..
                } = &stage.intent.spindle
                {
                    preparation_requirements.push(PreparationRequirement {
                        code: "PROCESS_SPINDLE_DIRECTION".into(),
                        message: format!(
                            "stage '{}' needs an explicit spindle direction before export",
                            stage.stage_id
                        ),
                        operation_id: Some(op.id.clone()),
                        stage_id: Some(stage.stage_id.clone()),
                    });
                }
            }
            for stage in planned.stages {
                let entry = motions[base + stage.motion_range.0].start;
                let exit = motions[base + stage.motion_range.1 - 1].end;
                let stage_id = stage.stage_id.clone();
                if current_tool.as_ref() != Some(&stage.tool_id) {
                    execution.push(ExecutionItem::ToolChange {
                        tool_id: stage.tool_id.clone(),
                    });
                    current_tool = Some(stage.tool_id.clone());
                }
                execution.push(ExecutionItem::SetProcessIntent {
                    intent: stage.intent.clone(),
                });
                execution.push(ExecutionItem::RunStage {
                    stage_id: stage_id.clone(),
                });
                stages.push(ExecutionStage {
                    stage_id: stage_id.clone(),
                    operation_id: op.id.clone(),
                    tool_id: stage.tool_id,
                    role: stage.role,
                    motion_range: (base + stage.motion_range.0, base + stage.motion_range.1),
                    entry_position: entry,
                    exit_position: exit,
                });
            }
            for issue in &planned.preparation {
                preparation_requirements.push(issue.clone());
            }
            for mut issue in planned.issues {
                issue.operation_id.get_or_insert_with(|| op.id.clone());
                generation_diagnostics.push(issue);
            }
            operation_results.push(OperationResult {
                operation_id: op.id.clone(),
                generation_status: planned.status,
                stage_ids: stages
                    .iter()
                    .filter(|s| s.operation_id == op.id)
                    .map(|s| s.stage_id.clone())
                    .collect(),
                stock_before_id: stock_before.clone(),
                stock_after_id: stock_after.clone(),
                named_outputs: planned.named_outputs,
                legacy_stage_evidence: planned.stage_evidence,
                legacy_pass_evidence: planned.pass_evidence,
            });
            stock_before = stock_after;
            for output in &operation_results.last().expect("just pushed").named_outputs {
                if output.kind == "face_plane"
                    && let (Some(z_mm), Some(covered)) = (output.z_mm, output.covered)
                {
                    published_faces.insert(
                        op.id.clone(),
                        crate::operations::PublishedFace { z_mm, covered },
                    );
                }
            }
        }
        let engine_version = env!("CARGO_PKG_VERSION").to_string();
        let input_fingerprint = crate::plan_hash::hash(&(
            &engine_version,
            OPERATION_PLAN_ARTIFACT_KIND,
            OPERATION_PLAN_SCHEMA_VERSION,
            job,
        ))
        .map_err(|e| error("PLAN_JSON", e.to_string()))?;
        let execution_fingerprint = crate::plan_hash::hash(&(
            &input_fingerprint,
            &stages,
            &motions,
            &execution,
            &operation_results,
        ))
        .map_err(|e| error("PLAN_JSON", e.to_string()))?;
        Ok(Self {
            artifact_kind: OPERATION_PLAN_ARTIFACT_KIND.into(),
            schema_version: OPERATION_PLAN_SCHEMA_VERSION,
            engine_version,
            job_snapshot: job.clone(),
            input_fingerprint,
            execution_fingerprint,
            operation_results,
            stages,
            motions,
            execution,
            preparation_requirements,
            generation_diagnostics,
        })
    }
}

pub(crate) fn kind_name(settings: &OperationSettings) -> &'static str {
    match settings {
        OperationSettings::FlatVcarve(_) => "flat_vcarve",
        OperationSettings::Face(_) => "face",
        OperationSettings::Profile(_) => "profile",
        OperationSettings::DragKnife(_) => "drag_knife",
    }
}

/// Map a legacy motion kind onto the new interpolation/purpose/effect triple.
/// The old writer emits G0 exactly for `rapid()` kinds; feed moves are G1.
pub(crate) fn legacy_motion_mapping(
    kind: crate::motion::MotionKind,
    finishing: bool,
) -> (Interpolation, MotionPurpose, MotionEffect) {
    use crate::motion::MotionKind;
    match kind {
        MotionKind::RapidXY | MotionKind::RapidRetract => (
            Interpolation::Rapid,
            MotionPurpose::Clearance,
            MotionEffect::None,
        ),
        MotionKind::Approach => (
            Interpolation::LinearFeed,
            MotionPurpose::Approach,
            MotionEffect::None,
        ),
        MotionKind::Plunge | MotionKind::Ramp => (
            Interpolation::LinearFeed,
            MotionPurpose::Entry,
            MotionEffect::MillingSweep,
        ),
        MotionKind::Cut => (
            Interpolation::LinearFeed,
            if finishing {
                MotionPurpose::Finish
            } else {
                MotionPurpose::Rough
            },
            MotionEffect::MillingSweep,
        ),
    }
}

/// Tool IDs referenced by one operation's settings, in assignment order.
fn tool_ids_of(settings: &OperationSettings) -> Vec<&str> {
    match settings {
        OperationSettings::FlatVcarve(s) => {
            vec![s.endmill.tool_id.as_str(), s.vbit.tool_id.as_str()]
        }
        OperationSettings::Face(s) => vec![s.assignment.tool_id.as_str()],
        OperationSettings::Profile(s) => vec![s.assignment.tool_id.as_str()],
        OperationSettings::DragKnife(s) => vec![s.assignment.tool_id.as_str()],
    }
}

impl OperationPlan {
    /// Compose the ordered removal history of the executed prefix ending at
    /// `through` (all operations when None). Knife traces never enter the
    /// model; only motions with a milling effect become sweeps.
    pub fn stock_history(
        &self,
        through: Option<&str>,
    ) -> Result<crate::stock::history::StockHistory> {
        let thickness = self.job_snapshot.setup.stock.thickness_mm.ok_or_else(|| {
            error(
                "SETUP_STOCK_THICKNESS_REQUIRED",
                "stock history requires the stock thickness",
            )
        })?;
        let mut history =
            crate::stock::history::StockHistory::new(thickness, self.job_snapshot.setup.stock.xy)?;
        for stage in &self.stages {
            let cutter = self
                .job_snapshot
                .tools
                .iter()
                .find(|tool| tool.id == stage.tool_id)
                .and_then(|tool| match &tool.geometry {
                    Some(crate::project::ToolGeometry::Endmill(g)) => {
                        Some(crate::stock::history::SweepCutter::FlatEndmill {
                            radius_mm: g.diameter_mm / 2.,
                        })
                    }
                    Some(crate::project::ToolGeometry::Vbit(spec)) => {
                        Some(crate::stock::history::SweepCutter::VBit { spec: spec.clone() })
                    }
                    _ => None,
                });
            let Some(cutter) = cutter else {
                return Err(error(
                    "STOCK_HISTORY_TOOL",
                    format!(
                        "stage '{}' uses a tool without milling geometry",
                        stage.stage_id
                    ),
                ));
            };
            let motions: Vec<crate::stock::history::SweepMotion> = self.motions
                [stage.motion_range.0..stage.motion_range.1]
                .iter()
                .filter(|motion| motion.effect == crate::toolpath::MotionEffect::MillingSweep)
                .map(|motion| crate::stock::history::SweepMotion {
                    start: motion.start,
                    end: motion.end,
                })
                .collect();
            if !motions.is_empty() {
                history.push(crate::stock::history::SweepBatch {
                    stage_id: stage.stage_id.clone(),
                    operation_id: stage.operation_id.clone(),
                    cutter,
                    motions,
                });
            }
            if through == Some(stage.operation_id.as_str()) {
                break;
            }
        }
        Ok(history)
    }
}
