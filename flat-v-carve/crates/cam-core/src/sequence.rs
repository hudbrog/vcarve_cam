//! Ordered operation plans: stages, execution items and process state.
//!
//! `plan_job` assembles the enabled operations of a [`crate::project::CamJob`]
//! in document order into one executable plan. Slice A2 supports the Flat
//! V-carve compatibility adapter against original stock-top geometry; later
//! slices add stock history, face/profile/knife planners and export.
//! `plan_job_v5` (H3) assembles schema-5 collection documents the same way
//! through cross-source resolution, and carries the semantic machining
//! identity of plan section 22.8 instead of a whole-job receipt.
use crate::{
    geometry::{Diagnostic, Result},
    job::PlanningTolerances,
    project::{
        CamJob, OperationSettings, SpindleDirection,
        v5::{
            self, CamJobV5, GeometryRef, KnifeAssignmentV5, MillingAssignmentV5,
            OperationSettingsV5, OperationV5, ReadinessScope, artwork,
            references::planning_readiness, resolve,
        },
    },
    toolpath::{Interpolation, MotionEffect, MotionPurpose, PlannedMotion},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const OPERATION_PLAN_ARTIFACT_KIND: &str = "operation_plan";
pub const OPERATION_PLAN_SCHEMA_VERSION: u32 = 1;
/// Schema 2 of the operation-plan artifact carries an embedded collection
/// document and semantic identities (plan sections 22.2 and 22.8).
pub const OPERATION_PLAN_V5_SCHEMA_VERSION: u32 = 2;

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

/// One resolved tab placement in the plan: the protected bridge interval on
/// the compensated centerline (arc length along the executed travel) and the
/// tab top, so previews show exactly what was generated (plan section 10.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct TabPlacementOutput {
    pub contour_id: String,
    /// Arc-length interval `[start_mm, end_mm]` of the minimum full-height
    /// bridge along the resolved loop.
    pub bridge_start_mm: f64,
    pub bridge_end_mm: f64,
    /// The wider centerline interval where the cutter must stay at or above
    /// the tab top (bridge dilated by the cutter radius and margin).
    pub restricted_start_mm: f64,
    pub restricted_end_mm: f64,
    pub top_z_mm: f64,
    /// Setup-space quad of the bridge's protected cross-section (the cutter
    /// corridor over the bridge) for exact preview overlays, independent of
    /// display-grid resolution (plan section 15.3). Empty in older plans.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub footprint_mm: Vec<(f64, f64)>,
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
    /// Profile tab payload: the resolved placements of one profile operation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tab_placements: Vec<TabPlacementOutput>,
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

/// The schema-5 collection counterpart of [`TrustedPlan`]: only
/// [`OperationPlanV5::plan_job_v5`] creates it in-process, so export
/// preparation over collection documents accepts nothing a client serialized.
pub struct TrustedPlanV5(OperationPlanV5);
impl std::fmt::Debug for TrustedPlanV5 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustedPlanV5")
            .field("execution_fingerprint", &self.0.execution_fingerprint)
            .finish_non_exhaustive()
    }
}
impl TrustedPlanV5 {
    pub fn plan(&self) -> &OperationPlanV5 {
        &self.0
    }
    /// Bind a collection plan this engine just generated in this process.
    pub fn from_generated(plan: OperationPlanV5) -> Self {
        Self(plan)
    }
}

/// The plan surface the post pipeline and emitted-output evidence consume
/// (H4): both the schema-4 and schema-5 plan shapes expose it, so preparation,
/// export, decoding, comparison and knife replay run identical algorithms over
/// collection documents instead of a second pipeline.
pub trait SequencePlan {
    fn setup(&self) -> &crate::project::SetupSettings;
    fn tolerances(&self) -> &PlanningTolerances;
    /// Geometry snapshot of a job tool, if the tool exists and carries one.
    fn tool_geometry(&self, tool_id: &str) -> Option<&crate::project::ToolGeometry>;
    fn stages(&self) -> &[ExecutionStage];
    fn motions(&self) -> &[PlannedMotion];
    fn execution(&self) -> &[ExecutionItem];
    fn operation_results(&self) -> &[OperationResult];
    fn execution_fingerprint(&self) -> &str;
    /// The automatic basic checks of this plan shape.
    fn basic_checks(&self) -> Result<crate::checks::BasicCheckReport>;
}

impl SequencePlan for OperationPlan {
    fn setup(&self) -> &crate::project::SetupSettings {
        &self.job_snapshot.setup
    }
    fn tolerances(&self) -> &PlanningTolerances {
        &self.job_snapshot.tolerances
    }
    fn tool_geometry(&self, tool_id: &str) -> Option<&crate::project::ToolGeometry> {
        self.job_snapshot
            .tools
            .iter()
            .find(|tool| tool.id == tool_id)
            .and_then(|tool| tool.geometry.as_ref())
    }
    fn stages(&self) -> &[ExecutionStage] {
        &self.stages
    }
    fn motions(&self) -> &[PlannedMotion] {
        &self.motions
    }
    fn execution(&self) -> &[ExecutionItem] {
        &self.execution
    }
    fn operation_results(&self) -> &[OperationResult] {
        &self.operation_results
    }
    fn execution_fingerprint(&self) -> &str {
        &self.execution_fingerprint
    }
    fn basic_checks(&self) -> Result<crate::checks::BasicCheckReport> {
        crate::checks::check_plan(self)
    }
}

impl SequencePlan for OperationPlanV5 {
    fn setup(&self) -> &crate::project::SetupSettings {
        &self.job_snapshot.setup
    }
    fn tolerances(&self) -> &PlanningTolerances {
        &self.job_snapshot.tolerances
    }
    fn tool_geometry(&self, tool_id: &str) -> Option<&crate::project::ToolGeometry> {
        self.job_snapshot
            .tools
            .iter()
            .find(|tool| tool.id == tool_id)
            .and_then(|tool| tool.geometry.as_ref())
    }
    fn stages(&self) -> &[ExecutionStage] {
        &self.stages
    }
    fn motions(&self) -> &[PlannedMotion] {
        &self.motions
    }
    fn execution(&self) -> &[ExecutionItem] {
        &self.execution
    }
    fn operation_results(&self) -> &[OperationResult] {
        &self.operation_results
    }
    fn execution_fingerprint(&self) -> &str {
        &self.execution_fingerprint
    }
    fn basic_checks(&self) -> Result<crate::checks::BasicCheckReport> {
        crate::checks::check_plan_v5(self)
    }
}

/// Trust boundary of the post pipeline: only wrappers this engine itself
/// constructed can hand a [`SequencePlan`] to preparation and export. A
/// serialized plan, a client-built struct or a plain reference cannot.
pub trait TrustedSequencePlan {
    fn trusted(&self) -> &dyn SequencePlan;
}
impl TrustedSequencePlan for TrustedPlan {
    fn trusted(&self) -> &dyn SequencePlan {
        self.plan()
    }
}
impl TrustedSequencePlan for TrustedPlanV5 {
    fn trusted(&self) -> &dyn SequencePlan {
        self.plan()
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
                    | OperationSettings::DragKnife(_)
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
        let tools = &self.job_snapshot.tools;
        compose_stock_history(
            thickness,
            self.job_snapshot.setup.stock.xy,
            &self.stages,
            &self.motions,
            &|tool_id| {
                tools
                    .iter()
                    .find(|tool| tool.id == tool_id)
                    .and_then(|tool| tool.geometry.clone())
            },
            through,
        )
    }
}

/// Shared removal-history composition over an executed plan prefix: the
/// schema-4 and schema-5 plan shapes feed the same stages/motions through
/// their own tool lookups.
fn compose_stock_history(
    thickness: f64,
    xy: Option<crate::project::RectXY>,
    stages: &[ExecutionStage],
    motions: &[PlannedMotion],
    tool_geometry: &dyn Fn(&str) -> Option<crate::project::ToolGeometry>,
    through: Option<&str>,
) -> Result<crate::stock::history::StockHistory> {
    let mut history = crate::stock::history::StockHistory::new(thickness, xy)?;
    for stage in stages {
        // Knife traces never enter the milling-stock model (plan section
        // 9.1); knife stages contribute no sweeps and need no cutter.
        if stage.role == StageRole::Knife {
            if through == Some(stage.operation_id.as_str()) {
                break;
            }
            continue;
        }
        let cutter = tool_geometry(&stage.tool_id).and_then(|geometry| match &geometry {
            crate::project::ToolGeometry::Endmill(g) => {
                Some(crate::stock::history::SweepCutter::FlatEndmill {
                    radius_mm: g.diameter_mm / 2.,
                })
            }
            crate::project::ToolGeometry::Vbit(spec) => {
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
        let sweeps: Vec<crate::stock::history::SweepMotion> = motions
            [stage.motion_range.0..stage.motion_range.1]
            .iter()
            .filter(|motion| motion.effect == crate::toolpath::MotionEffect::MillingSweep)
            .map(|motion| crate::stock::history::SweepMotion {
                start: motion.start,
                end: motion.end,
            })
            .collect();
        if !sweeps.is_empty() {
            history.push(crate::stock::history::SweepBatch {
                stage_id: stage.stage_id.clone(),
                operation_id: stage.operation_id.clone(),
                cutter,
                motions: sweeps,
            });
        }
        if through == Some(stage.operation_id.as_str()) {
            break;
        }
    }
    Ok(history)
}

// ---------------------------------------------------------------------------
// Schema-5 collection plans (H3, plan sections 22.4 and 22.8): the same
// ordered assembly over resolved cross-source geometry, with semantic
// identities replacing the whole-job receipt.
// ---------------------------------------------------------------------------

/// The ordered operation plan of a schema-5 collection document. The
/// `machining_identity` is the semantic scope identity of plan section 22.8:
/// it excludes display names, provenance, artwork row order, unreferenced
/// items/tools and the work-zero selection, so those edits keep a plan
/// current while used-source and assignment edits do not.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationPlanV5 {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub engine_version: String,
    pub job_snapshot: CamJobV5,
    pub machining_identity: String,
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

/// Enabled operations of `job` inside `scope`, in document order.
pub(crate) fn scoped_enabled_operations_v5<'a>(
    job: &'a CamJobV5,
    scope: &ReadinessScope,
) -> Result<Vec<&'a OperationV5>> {
    Ok(match scope {
        ReadinessScope::AllEnabled => job.operations.iter().filter(|op| op.enabled).collect(),
        ReadinessScope::ThroughOperation { operation_id } => {
            let Some(end) = job.operations.iter().position(|op| op.id == *operation_id) else {
                return Err(error(
                    "READINESS_SCOPE",
                    format!("prefix scope references unknown operation '{operation_id}'"),
                ));
            };
            job.operations[..=end]
                .iter()
                .filter(|op| op.enabled)
                .collect()
        }
    })
}

/// Tool IDs referenced by one collection operation's settings, in
/// assignment order (mirrors the schema-4 `tool_ids_of`). Shared with the
/// H4 machine-configuration resolver for active-scope mapping validation.
pub(crate) fn tool_ids_of_v5(settings: &OperationSettingsV5) -> Vec<&str> {
    match settings {
        OperationSettingsV5::FlatVcarve(s) => {
            vec![s.endmill.tool_id.as_str(), s.vbit.tool_id.as_str()]
        }
        OperationSettingsV5::Face(s) => vec![s.assignment.tool_id.as_str()],
        OperationSettingsV5::Profile(s) => vec![s.assignment.tool_id.as_str()],
        OperationSettingsV5::DragKnife(s) => vec![s.assignment.tool_id.as_str()],
    }
}

/// Every artwork item an operation's settings address (selections and
/// anchors). Unreferenced items never enter a machining identity.
fn referenced_item_ids(settings: &OperationSettingsV5) -> Vec<&v5::ArtworkItemId> {
    fn anchors(start: &v5::StartSelectionV5) -> Vec<&GeometryRef> {
        match start {
            v5::StartSelectionV5::Automatic => vec![],
            v5::StartSelectionV5::Anchor(anchor) => vec![&anchor.geometry],
        }
    }
    fn tab_anchors(tabs: &Option<v5::TabSettingsV5>) -> Vec<&GeometryRef> {
        match tabs {
            Some(v5::TabSettingsV5 {
                placement: v5::TabPlacementV5::Manual { anchors },
                ..
            }) => anchors.iter().map(|a| &a.geometry).collect(),
            _ => vec![],
        }
    }
    match settings {
        OperationSettingsV5::FlatVcarve(s) => {
            s.components.iter().map(|r| &r.artwork_item_id).collect()
        }
        OperationSettingsV5::Face(_) => vec![],
        OperationSettingsV5::Profile(s) => s
            .contours
            .iter()
            .map(|c| &c.geometry.artwork_item_id)
            .chain(anchors(&s.start).into_iter().map(|r| &r.artwork_item_id))
            .chain(tab_anchors(&s.tabs).into_iter().map(|r| &r.artwork_item_id))
            .collect(),
        OperationSettingsV5::DragKnife(s) => s
            .chains
            .iter()
            .map(|r| &r.artwork_item_id)
            .chain(anchors(&s.start).into_iter().map(|r| &r.artwork_item_id))
            .collect(),
    }
}

/// The machining-relevant projection of one assignment: copied cutting
/// values with provenance stripped (plan section 22.8 excludes applied
/// profile baselines from machining identity).
fn semantic_milling(assignment: &MillingAssignmentV5) -> MillingAssignmentV5 {
    MillingAssignmentV5 {
        tool_id: assignment.tool_id.clone(),
        spindle_rpm: assignment.spindle_rpm,
        spindle_direction: assignment.spindle_direction,
        cutting_feed_mm_min: assignment.cutting_feed_mm_min,
        plunge_feed_mm_min: assignment.plunge_feed_mm_min,
        max_stepdown_mm: assignment.max_stepdown_mm,
        stepover_mm: assignment.stepover_mm,
        applied_profile: None,
    }
}

fn semantic_knife(assignment: &KnifeAssignmentV5) -> KnifeAssignmentV5 {
    KnifeAssignmentV5 {
        tool_id: assignment.tool_id.clone(),
        cutting_feed_mm_min: assignment.cutting_feed_mm_min,
        plunge_feed_mm_min: assignment.plunge_feed_mm_min,
        swivel_feed_mm_min: assignment.swivel_feed_mm_min,
        max_stepdown_mm: assignment.max_stepdown_mm,
        applied_profile: None,
    }
}

fn semantic_settings(settings: &OperationSettingsV5) -> OperationSettingsV5 {
    match settings {
        OperationSettingsV5::FlatVcarve(s) => {
            OperationSettingsV5::FlatVcarve(v5::FlatVcarveSettingsV5 {
                components: s.components.clone(),
                mode: s.mode,
                endmill: semantic_milling(&s.endmill),
                vbit: semantic_milling(&s.vbit),
                top: s.top.clone(),
                max_depth_mm: s.max_depth_mm,
                wall_allowance_mm: s.wall_allowance_mm,
                max_floor_ridge_mm: s.max_floor_ridge_mm,
                max_detail_residual_mm: s.max_detail_residual_mm,
                rough: s.rough.clone(),
                finish: if s.mode == crate::project::FlatVcarveMode::Combined {
                    s.finish.clone()
                } else {
                    None
                },
            })
        }
        OperationSettingsV5::Face(s) => OperationSettingsV5::Face(v5::FaceSettingsV5 {
            area: s.area.clone(),
            margins: s.margins,
            entry_overrun_mm: s.entry_overrun_mm,
            exit_overrun_mm: s.exit_overrun_mm,
            top: s.top.clone(),
            bottom: s.bottom.clone(),
            stepdown_mm: s.stepdown_mm,
            stepover_mm: s.stepover_mm,
            pass_angle_deg: s.pass_angle_deg,
            pattern: s.pattern,
            assignment: semantic_milling(&s.assignment),
        }),
        OperationSettingsV5::Profile(s) => OperationSettingsV5::Profile(v5::ProfileSettingsV5 {
            contours: s.contours.clone(),
            assignment: semantic_milling(&s.assignment),
            top: s.top.clone(),
            bottom: s.bottom.clone(),
            stepdown_mm: s.stepdown_mm,
            through_cut_allowance_mm: s.through_cut_allowance_mm,
            direction: s.direction,
            order: s.order,
            start: s.start.clone(),
            finish: s.finish.clone(),
            entry: s.entry.clone(),
            lead_in: s.lead_in.clone(),
            lead_out: s.lead_out.clone(),
            tabs: s.tabs.clone(),
        }),
        OperationSettingsV5::DragKnife(s) => {
            OperationSettingsV5::DragKnife(v5::DragKnifeSettingsV5 {
                chains: s.chains.clone(),
                assignment: semantic_knife(&s.assignment),
                top: s.top.clone(),
                bottom: s.bottom.clone(),
                stepdown_mm: s.stepdown_mm,
                swivel_depth_mm: s.swivel_depth_mm,
                corner_threshold_deg: s.corner_threshold_deg,
                through_cut_allowance_mm: s.through_cut_allowance_mm,

                start: s.start.clone(),
                closure_overlap_mm: s.closure_overlap_mm,
                alignment: s.alignment.clone(),
            })
        }
    }
}

impl OperationPlanV5 {
    /// The semantic machining identity of one scope (plan section 22.8):
    /// engine and artifact contracts, setup material/travel (the work-zero
    /// selection is deliberately absent — it changes output, never motions),
    /// tolerances, the artwork items the scope actually selects (sorted by
    /// stable item ID with their content revision, interpretation and
    /// placement — never row order), the enabled operation order with their
    /// provenance-stripped settings, and the used tools' geometry snapshots.
    pub fn machining_identity(job: &CamJobV5, scope: &ReadinessScope) -> Result<String> {
        let enabled = scoped_enabled_operations_v5(job, scope)?;
        let (artwork_inputs, tool_snapshots) = semantic_scope_inputs(job, &enabled)?;
        let operations: Vec<(&str, OperationSettingsV5)> = enabled
            .iter()
            .map(|op| (op.id.as_str(), semantic_settings(&op.settings)))
            .collect();
        crate::plan_hash::hash(&(
            "machining-identity",
            env!("CARGO_PKG_VERSION"),
            OPERATION_PLAN_ARTIFACT_KIND,
            OPERATION_PLAN_V5_SCHEMA_VERSION,
            &job.setup.stock,
            &job.setup.clearance_above_stock_mm,
            &job.setup.start_xy_mm,
            &job.tolerances,
            &artwork_inputs,
            &tool_snapshots,
            &operations,
        ))
        .map_err(|e| error("PLAN_JSON", e.to_string()))
    }

    /// Plan every enabled collection operation inside `scope` in document
    /// order through cross-source resolution (plan sections 22.4 and 22.5).
    /// Reference-blocked and numerically incomplete operations become
    /// incomplete results with located issues; they never fail the whole
    /// plan, and their dependents see the missing published outputs.
    pub fn plan_job_v5(
        job: &CamJobV5,
        scope: &ReadinessScope,
        limits: &PlanLimits,
    ) -> Result<Self> {
        job.validate_structure()?;
        let enabled = scoped_enabled_operations_v5(job, scope)?;
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
        let readiness = planning_readiness(job, scope)?;
        let combined = artwork::inspect_artwork(job)?;
        let catalogue = resolve::assembled_catalogue(&combined)?;
        let ctx = crate::operations::PlanContext::from_v5(job);
        let mut operation_results = vec![];
        let mut stages = vec![];
        let mut motions = vec![];
        let mut execution = vec![];
        let mut generation_diagnostics = vec![];
        let mut preparation_requirements = vec![];
        let mut planned_ids = BTreeSet::new();
        let mut published_faces = crate::operations::PublishedFaceMap::new();
        let mut current_tool: Option<String> = None;
        // Semantic prefix stock identities (plan section 9.2 over collection
        // inputs): the initial view hashes stock/travel/tolerances, the
        // referenced artwork (sorted by item ID) and the used tool snapshots;
        // each operation extends it with its semantic settings and actual
        // execution, so editing an earlier operation stales every later id.
        let mut stock_before = semantic_initial_stock(job, &enabled)?;
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
            // Invalid references cannot reach planners (plan section 22.5):
            // the blocked operation turns incomplete with its located issues.
            let planned = match readiness
                .operations
                .iter()
                .find(|entry| entry.operation_id == op.id)
            {
                Some(entry) if !entry.ready => PlannedOperation {
                    status: GenerationStatus::Incomplete,
                    stages: vec![],
                    motions: vec![],
                    stage_evidence: vec![],
                    pass_evidence: vec![],
                    issues: entry
                        .blockers
                        .iter()
                        .map(|d| PlanIssue {
                            code: d.code.clone(),
                            message: format!(
                                "{} ({})",
                                d.message,
                                d.field_path.as_deref().unwrap_or("")
                            ),
                            operation_id: d.operation_id.clone(),
                            stage_id: None,
                        })
                        .collect(),
                    preparation: vec![],
                    named_outputs: vec![],
                },
                _ => resolve::plan_operation_v5(
                    job,
                    &ctx,
                    op,
                    &combined,
                    &catalogue,
                    &published_faces,
                    &motions,
                )?,
            };
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
            let used_tool_snapshots = semantic_tool_snapshots(job, &op.settings);
            let stock_after = crate::plan_hash::hash(&(
                &stock_before,
                &op.id,
                &semantic_settings(&op.settings),
                &used_tool_snapshots,
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
        let machining_identity = Self::machining_identity(job, scope)?;
        let execution_fingerprint = crate::plan_hash::hash(&(
            &machining_identity,
            &stages,
            &motions,
            &execution,
            &operation_results,
        ))
        .map_err(|e| error("PLAN_JSON", e.to_string()))?;
        Ok(Self {
            artifact_kind: OPERATION_PLAN_ARTIFACT_KIND.into(),
            schema_version: OPERATION_PLAN_V5_SCHEMA_VERSION,
            engine_version,
            job_snapshot: job.clone(),
            machining_identity,
            execution_fingerprint,
            operation_results,
            stages,
            motions,
            execution,
            preparation_requirements,
            generation_diagnostics,
        })
    }

    /// Compose the ordered removal history of the executed prefix ending at
    /// `through` (all operations when None); knife traces never enter.
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
        let tools = &self.job_snapshot.tools;
        compose_stock_history(
            thickness,
            self.job_snapshot.setup.stock.xy,
            &self.stages,
            &self.motions,
            &|tool_id| {
                tools
                    .iter()
                    .find(|tool| tool.id == tool_id)
                    .and_then(|tool| tool.geometry.clone())
            },
            through,
        )
    }
}

/// A used tool as it enters a machining identity: geometry and capabilities
/// only — identity strings, display names and provenance never enter a
/// machining identity (plan section 22.8).
type SemanticToolSnapshot<'a> = (
    &'a str,
    Option<&'a crate::project::ToolGeometry>,
    &'a crate::project::ToolCapabilities,
);

/// A referenced artwork item as it enters a machining identity: content
/// revision, interpretation and placement.
type SemanticArtworkInput<'a> = (
    &'a str,
    v5::SourceRevision,
    &'a v5::SvgInterpretation,
    &'a crate::svg::Placement,
);

fn semantic_tool_snapshots<'a>(
    job: &'a CamJobV5,
    settings: &'a OperationSettingsV5,
) -> Vec<SemanticToolSnapshot<'a>> {
    tool_ids_of_v5(settings)
        .into_iter()
        .filter_map(|id| job.tools.iter().find(|tool| tool.id == id))
        .map(|tool| (tool.id.as_str(), tool.geometry.as_ref(), &tool.capabilities))
        .collect()
}

/// Artwork inputs and tool snapshots referenced by one scope, sorted by
/// stable IDs: content revision, interpretation and placement enter the
/// identity; display names, provenance and row order never do.
fn semantic_scope_inputs<'a>(
    job: &'a CamJobV5,
    enabled: &[&'a OperationV5],
) -> Result<(Vec<SemanticArtworkInput<'a>>, Vec<SemanticToolSnapshot<'a>>)> {
    let mut item_ids = BTreeSet::new();
    let mut tool_ids = BTreeSet::new();
    for operation in enabled {
        for id in referenced_item_ids(&operation.settings) {
            item_ids.insert(id.0.as_str());
        }
        for id in tool_ids_of_v5(&operation.settings) {
            tool_ids.insert(id);
        }
    }
    let artwork_inputs = item_ids
        .iter()
        .filter_map(|id| job.artwork.iter().find(|item| item.id.0 == *id))
        .map(|item| {
            Ok((
                item.id.0.as_str(),
                item.source_revision()
                    .map_err(|e| error("PLAN_JSON", e.message))?,
                &item.import_settings,
                &item.placement,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let tool_snapshots = tool_ids
        .iter()
        .filter_map(|id| job.tools.iter().find(|tool| tool.id == *id))
        .map(|tool| (tool.id.as_str(), tool.geometry.as_ref(), &tool.capabilities))
        .collect();
    Ok((artwork_inputs, tool_snapshots))
}

/// Initial semantic stock identity for a collection scope: stock/travel/
/// tolerances plus the referenced artwork (sorted by item ID) and used tool
/// snapshots. Display names, provenance, artwork row order and unreferenced
/// content never enter it (plan section 22.8).
fn semantic_initial_stock(job: &CamJobV5, enabled: &[&OperationV5]) -> Result<String> {
    let (artwork_inputs, tool_snapshots) = semantic_scope_inputs(job, enabled)?;
    crate::plan_hash::hash(&(
        "stock-initial",
        OPERATION_PLAN_ARTIFACT_KIND,
        OPERATION_PLAN_V5_SCHEMA_VERSION,
        env!("CARGO_PKG_VERSION"),
        &job.setup.stock,
        &job.setup.clearance_above_stock_mm,
        &job.setup.start_xy_mm,
        &job.tolerances,
        &artwork_inputs,
        &tool_snapshots,
    ))
    .map_err(|e| error("PLAN_JSON", e.to_string()))
}
