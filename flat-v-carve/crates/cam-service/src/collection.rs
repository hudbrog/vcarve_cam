//! ui-9 collection DTOs (H4, plan sections 22.6, 22.7 and 22.9): schema-5
//! artwork-collection documents, cutting-profile and tool-assignment
//! commands, the one applied machine configuration, scope-aware planning and
//! read-only inspection — one shared projection for the HTTP service and the
//! WebAssembly worker.
//!
//! The ui-8 sequence transport keeps serving schema-4 jobs unchanged; this
//! transport owns collection documents and refuses to flatten them. Resource
//! commands take the library or configuration document as an explicit input
//! and return independent copied values: reopening the result needs neither
//! file, and later edits to them cannot change a saved job.
use crate::document::{ENGINE_VERSION, UiDiagnostic};
use cam_core::{
    checks::check_plan_v5,
    geometry::{Diagnostic, Result},
    operations::drag_knife::evidence::{KNIFE_EVIDENCE_MAX_SAMPLES, build_evidence},
    post::sequence::{OutputLayout, PreparedExecution, SequenceProfile},
    project::v5::{
        self, CAM_JOB_V5_SCHEMA_VERSION, CamJobV5,
        commands::AffectedEntity,
        inspection::{self, DocumentInspection, PlanInspection},
        machine,
        migrate::migrate_json,
        references::ReadinessScope,
        resources::{self, AssignmentRole},
    },
    sequence::{OperationPlanV5, PlanLimits, TrustedPlanV5},
    tool_library::{self, ToolLibrary},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const COLLECTION_API_VERSION: &str = "ui-9";

pub(crate) fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("collection")
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollectionRequest {
    pub api_version: String,
    pub request_id: String,
    pub revision: u64,
    pub command: CollectionCommand,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "camelCase", deny_unknown_fields)]
pub enum CollectionCommand {
    /// Open any supported job document (schema 1-5); older schemas migrate
    /// once, irreversibly, into the collection model.
    Open { json: String },
    /// Plan the enabled collection operations, all of them or a prefix.
    Plan { job: Value, scope: CollectionScope },
    /// One bounded page of a scope's ordered motion stream. Replanning per
    /// page is the documented pre-H5 limitation; retained plan handles
    /// arrive with the output-artifact slice.
    Motions {
        job: Value,
        scope: CollectionScope,
        offset: usize,
    },
    /// Read-only document inspection: machining order, used-by index,
    /// assignment statuses and the machine readout.
    Inspect { job: Value },
    /// Apply one named cutting profile to exactly one assignment (plan
    /// section 22.6): every applicable preset field is copied — unset ones
    /// included — and stored as the baseline Reset/Modified use.
    ApplyCuttingProfile {
        job: Value,
        library: Value,
        library_id: String,
        operation_id: String,
        role: AssignmentRole,
        library_tool_id: String,
        preset_id: String,
    },
    /// Restore one assignment's copied baseline without any library file.
    ResetAssignment {
        job: Value,
        operation_id: String,
        role: AssignmentRole,
    },
    /// Reapply the stored provenance from a supplied current library
    /// revision; the current values become the new baseline.
    ReapplyProfile {
        job: Value,
        library: Value,
        library_id: String,
        operation_id: String,
        role: AssignmentRole,
    },
    /// Bind a library tool's geometry to one assignment ("Use in
    /// operation"): an independent snapshot with copied provenance.
    ApplyTool {
        job: Value,
        library: Value,
        library_id: String,
        operation_id: String,
        role: AssignmentRole,
        library_tool_id: String,
    },
    /// Copy a reusable configuration file into the job's one applied machine
    /// snapshot (plan section 22.7). Rows for tools this job does not carry
    /// are dropped, never transplanted.
    ApplyMachineConfiguration {
        job: Value,
        profile: Value,
        configuration_name: String,
    },
    /// Set one job tool's controller mapping exactly; `None` values are
    /// absolute and both `None` removes the row.
    SetToolMapping {
        job: Value,
        job_tool_id: String,
        tool_number: Option<u32>,
        length_offset_number: Option<u32>,
    },
    /// Resolve the applied snapshot into the complete validated schema-2
    /// profile export needs, validating mappings for the selected scope.
    ResolveProfile { job: Value, scope: CollectionScope },
    /// Bounded emitted-output knife evidence (F3b) over a collection plan:
    /// the applied machine configuration supplies the process profile — no
    /// separate profile parameter exists for collection documents.
    KnifeEvidence {
        job: Value,
        scope: CollectionScope,
        sample_limit: Option<usize>,
    },
    /// Retained planning task (H5, plan section 22.8): registers a
    /// cancellable generation of the submitted document snapshot for the
    /// scope, answered by `TaskStatus` with a service-owned plan handle.
    /// Needs the retained runtime; the stateless entry refuses it.
    Generate { job: Value, scope: CollectionScope },
    /// State of one retained task; a succeeded generation carries its plan
    /// handle, a succeeded preparation its bundle handle.
    TaskStatus { task_id: String },
    /// Cancel one retained task. A completion arriving afterwards is
    /// discarded and can never issue a handle.
    CancelTask { task_id: String },
    /// One bounded page of a retained plan's ordered motions. Never replans:
    /// the page is read from the retained generated execution.
    ReadMotions { plan_handle: String, offset: usize },
    /// The bounded plan inspection of a retained plan.
    ReadInspection { plan_handle: String },
    /// Register a preparation task for one retained plan: the submitted
    /// current document supplies the applied machine configuration and must
    /// still match the plan's machining identity for the plan's scope; the
    /// output scope is the plan's scope and can never exceed it.
    PrepareOutput {
        plan_handle: String,
        job: Value,
        layout: OutputLayout,
    },
    /// A finished preparation's bundle handle, report and ordered manifest.
    PreparedOutput { task_id: String },
    /// The exact checked bytes of one file of a retained bundle — identical
    /// on every read, including retries after a failed save.
    ReadPreparedBytes {
        bundle_handle: String,
        filename: String,
    },
    /// Advertised kinds, features and limits.
    Capabilities,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CollectionScope {
    AllEnabled,
    ThroughOperation { operation_id: String },
}

impl CollectionScope {
    pub(crate) fn readiness(&self) -> ReadinessScope {
        match self {
            Self::AllEnabled => ReadinessScope::AllEnabled,
            Self::ThroughOperation { operation_id } => ReadinessScope::ThroughOperation {
                operation_id: operation_id.clone(),
            },
        }
    }
}

/// Document receipt for collection documents; deliberately distinct from the
/// ui-8 sequence receipt and from machining identities.
pub fn fingerprint(job: &CamJobV5) -> String {
    let mut hash = Sha256::new();
    hash.update(b"ui-collection-v1\0");
    hash.update(ENGINE_VERSION.as_bytes());
    hash.update(job.to_json().expect("validated job serializes").as_bytes());
    format!("{:x}", hash.finalize())
}

pub(crate) fn parse_job(value: &Value) -> Result<CamJobV5> {
    let raw = value.to_string();
    if raw.len() > crate::document::JOB_BYTES {
        return Err(error("JOB_RESOURCE_LIMIT", "job exceeds 64 MB"));
    }
    let version: Value =
        serde_json::from_str(&raw).map_err(|e| error("COLLECTION_JOB_JSON", e.to_string()))?;
    let schema = version.get("schema_version").and_then(Value::as_u64);
    if schema > Some(u64::from(CAM_JOB_V5_SCHEMA_VERSION)) {
        return Err(error(
            "COLLECTION_SCHEMA_UNSUPPORTED",
            format!(
                "schema {} documents need a newer client; this ui-9 client refuses them",
                schema.unwrap_or_default()
            ),
        ));
    }
    migrate_json(&raw)
}

fn parse_library(value: &Value) -> Result<ToolLibrary> {
    let raw = value.to_string();
    if raw.len() > tool_library::MAX_LIBRARY_BYTES {
        return Err(error("LIBRARY_RESOURCE_LIMIT", "tool library exceeds 8 MB"));
    }
    ToolLibrary::from_json(&raw)
}

fn parse_profile(value: &Value) -> Result<SequenceProfile> {
    let raw = value.to_string();
    if raw.len() > crate::export::PROFILE_BYTES {
        return Err(error("COLLECTION_PROFILE_LIMIT", "profile exceeds 64 KB"));
    }
    SequenceProfile::from_json(&raw)
}

fn affected_value(entity: &AffectedEntity) -> Value {
    match entity {
        AffectedEntity::ArtworkItem(id) => json!({"artworkItem": id.0}),
        AffectedEntity::Operation(id) => json!({"operation": id}),
        AffectedEntity::Setup => json!({"setup": true}),
        AffectedEntity::JobTool(id) => json!({"jobTool": id}),
        AffectedEntity::MachineConfiguration => json!({"machineConfiguration": true}),
    }
}

/// The artwork tree with owner-qualified geometry entries (plan section
/// 22.9): one projection shared by Open and Inspect.
fn artwork_projection(job: &CamJobV5) -> Result<Value> {
    let combined = v5::artwork::inspect_artwork(job)?;
    Ok(json!(
        combined
            .items
            .iter()
            .map(|item| {
                json!({
                    "id": item.id.0,
                    "name": item.name,
                    "revision": item.revision,
                    "importError": item.import_error,
                    "entries": item.entries.iter().map(|entry| json!({
                        "wireId": entry.wire_id,
                        "reference": entry.reference,
                        "kind": entry.kind,
                        "role": entry.role,
                        "closed": entry.closed,
                        "suggestedSide": entry.suggested_side,
                        "sourceFingerprint": entry.source_fingerprint,
                        "bounds": entry.bounds,
                        "perimeterMm": entry.perimeter_mm,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>()
    ))
}

fn issues_value(job: &CamJobV5) -> Result<Value> {
    Ok(json!(v5::references::inspect_references(job)?.issues))
}

fn document_projection(job: &CamJobV5, migrated: bool) -> Result<Value> {
    let inspection: DocumentInspection = inspection::inspect_document(job);
    Ok(json!({
        "job": serde_json::to_value(job).expect("validated job serializes"),
        "migrated": migrated,
        "artwork": artwork_projection(job)?,
        "inspection": inspection,
        "issues": issues_value(job)?,
        "documentFingerprint": fingerprint(job),
    }))
}

/// The shared projection after one resource/machine command: the updated
/// document, the located issues of the new state, stable affected IDs and
/// refreshed assignment statuses.
fn command_projection(
    job: &CamJobV5,
    issues: &[cam_core::operations::LocatedDiagnostic],
    affected: &[AffectedEntity],
) -> Result<Value> {
    Ok(json!({
        "job": serde_json::to_value(job).expect("validated job serializes"),
        "issues": issues,
        "affected": affected.iter().map(affected_value).collect::<Vec<_>>(),
        "assignments": resources::assignment_statuses(job),
        "documentFingerprint": fingerprint(job),
    }))
}

pub(crate) fn plan_summary(
    plan: &OperationPlanV5,
    checks: &cam_core::checks::BasicCheckReport,
) -> Value {
    json!({
        "engineVersion": ENGINE_VERSION,
        "machiningIdentity": plan.machining_identity,
        "executionFingerprint": plan.execution_fingerprint,
        "motionCount": plan.motions.len(),
        "cuttingMotionCount": plan.motions.iter().filter(|m| m.is_cutting()).count(),
        "operations": plan.operation_results.iter().map(|result| {
            json!({
                "operationId": result.operation_id,
                "generationStatus": result.generation_status,
                "stageIds": result.stage_ids,
                "stockBeforeId": result.stock_before_id,
                "stockAfterId": result.stock_after_id,
                "namedOutputs": result.named_outputs.iter().map(|output| json!({
                    "kind": output.kind,
                    "zMm": output.z_mm,
                    "covered": output.covered,
                    "tabPlacements": output.tab_placements,
                })).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
        "stages": plan.stages.iter().map(|stage| {
            json!({
                "stageId": stage.stage_id,
                "operationId": stage.operation_id,
                "toolId": stage.tool_id,
                "role": stage.role,
                "motionCount": stage.motion_range.1 - stage.motion_range.0,
            })
        }).collect::<Vec<_>>(),
        "basicChecks": {
            "status": checks.status,
            "findings": checks.findings,
            "exportReady": checks.export_ready,
        },
        "preparationRequirements": plan.preparation_requirements,
        "diagnostics": plan.generation_diagnostics,
    })
}

pub(crate) fn motion_page(plan: &OperationPlanV5, offset: usize) -> Result<Value> {
    if offset > plan.motions.len() {
        return Err(error(
            "COLLECTION_MOTION_OFFSET",
            format!(
                "motion offset {offset} exceeds the plan's {} motions",
                plan.motions.len()
            ),
        ));
    }
    let end = (offset + crate::task::PAGE_MOTIONS).min(plan.motions.len());
    Ok(json!({
        "offset": offset,
        "count": end - offset,
        "total": plan.motions.len(),
        "motions": &plan.motions[offset..end],
    }))
}

pub(crate) fn inspect_plan_value(plan: &OperationPlanV5) -> Result<PlanInspection> {
    inspection::inspect_plan(plan)
}

fn open(raw: &str) -> Result<Value> {
    if raw.len() > crate::document::JOB_BYTES {
        return Err(error("JOB_RESOURCE_LIMIT", "job exceeds 64 MB"));
    }
    let version: Value =
        serde_json::from_str(raw).map_err(|e| error("COLLECTION_JOB_JSON", e.to_string()))?;
    let schema = version.get("schema_version").and_then(Value::as_u64);
    if schema > Some(u64::from(CAM_JOB_V5_SCHEMA_VERSION)) {
        return Err(error(
            "COLLECTION_SCHEMA_UNSUPPORTED",
            format!(
                "schema {} documents need a newer client; this ui-9 client refuses them",
                schema.unwrap_or_default()
            ),
        ));
    }
    let migrated = schema != Some(u64::from(CAM_JOB_V5_SCHEMA_VERSION));
    let job = migrate_json(raw)?;
    document_projection(&job, migrated)
}

/// Execute one stateless ui-9 command. Planning recomputes from the
/// submitted document on every call; the retained-artifact commands (H5)
/// own service-side state and are refused here — route them through
/// [`crate::retained::Retained::execute`], which delegates every stateless
/// command back to this function unchanged.
pub fn execute(command: CollectionCommand) -> Result<Value> {
    match command {
        CollectionCommand::Open { json } => open(&json),
        CollectionCommand::Plan { job, scope } => {
            let job = parse_job(&job)?;
            let readiness = scope.readiness();
            let plan = OperationPlanV5::plan_job_v5(&job, &readiness, &PlanLimits::default())?;
            let checks = check_plan_v5(&plan)?;
            Ok(json!({
                "summary": plan_summary(&plan, &checks),
                "inspection": inspect_plan_value(&plan)?,
                "motions": motion_page(&plan, 0)?,
                "scope": scope,
            }))
        }
        CollectionCommand::Motions { job, scope, offset } => {
            let job = parse_job(&job)?;
            let plan =
                OperationPlanV5::plan_job_v5(&job, &scope.readiness(), &PlanLimits::default())?;
            Ok(json!({
                "motions": motion_page(&plan, offset)?,
                "scope": scope,
            }))
        }
        CollectionCommand::Inspect { job } => {
            let job = parse_job(&job)?;
            Ok(json!({
                "artwork": artwork_projection(&job)?,
                "inspection": inspection::inspect_document(&job),
                "issues": issues_value(&job)?,
                "documentFingerprint": fingerprint(&job),
            }))
        }
        CollectionCommand::ApplyCuttingProfile {
            job,
            library,
            library_id,
            operation_id,
            role,
            library_tool_id,
            preset_id,
        } => {
            let job = parse_job(&job)?;
            let library = parse_library(&library)?;
            let outcome = resources::apply_cutting_profile(
                &job,
                &operation_id,
                role,
                &library,
                &library_id,
                &library_tool_id,
                &preset_id,
            )?;
            command_projection(&outcome.job, &outcome.issues, &outcome.affected)
        }
        CollectionCommand::ResetAssignment {
            job,
            operation_id,
            role,
        } => {
            let job = parse_job(&job)?;
            let outcome = resources::reset_assignment(&job, &operation_id, role)?;
            command_projection(&outcome.job, &outcome.issues, &outcome.affected)
        }
        CollectionCommand::ReapplyProfile {
            job,
            library,
            library_id,
            operation_id,
            role,
        } => {
            let job = parse_job(&job)?;
            let library = parse_library(&library)?;
            let outcome =
                resources::reapply_profile(&job, &operation_id, role, &library, &library_id)?;
            command_projection(&outcome.job, &outcome.issues, &outcome.affected)
        }
        CollectionCommand::ApplyTool {
            job,
            library,
            library_id,
            operation_id,
            role,
            library_tool_id,
        } => {
            let job = parse_job(&job)?;
            let library = parse_library(&library)?;
            let outcome = resources::apply_tool_to_assignment(
                &job,
                &operation_id,
                role,
                &library,
                &library_id,
                &library_tool_id,
            )?;
            command_projection(&outcome.job, &outcome.issues, &outcome.affected)
        }
        CollectionCommand::ApplyMachineConfiguration {
            job,
            profile,
            configuration_name,
        } => {
            let job = parse_job(&job)?;
            let profile = parse_profile(&profile)?;
            let outcome =
                machine::apply_machine_configuration(&job, &profile, &configuration_name)?;
            command_projection(&outcome.job, &outcome.issues, &outcome.affected)
        }
        CollectionCommand::SetToolMapping {
            job,
            job_tool_id,
            tool_number,
            length_offset_number,
        } => {
            let job = parse_job(&job)?;
            let outcome =
                machine::set_tool_mapping(&job, &job_tool_id, tool_number, length_offset_number)?;
            command_projection(&outcome.job, &outcome.issues, &outcome.affected)
        }
        CollectionCommand::ResolveProfile { job, scope } => {
            let job = parse_job(&job)?;
            let profile = machine::resolve_sequence_profile(&job, &scope.readiness())?;
            Ok(json!({
                "profile": profile,
                "scope": scope,
            }))
        }
        CollectionCommand::KnifeEvidence {
            job,
            scope,
            sample_limit,
        } => {
            let job = parse_job(&job)?;
            let readiness = scope.readiness();
            let profile = machine::resolve_sequence_profile(&job, &readiness)?;
            let plan = OperationPlanV5::plan_job_v5(&job, &readiness, &PlanLimits::default())?;
            let trusted = TrustedPlanV5::from_generated(plan);
            let prepared = PreparedExecution::prepare(&trusted, &profile)?;
            let export = prepared.export(&trusted, &profile)?;
            if export.program.gcode.len() > crate::export::PROGRAM_BYTES {
                return Err(error(
                    "COLLECTION_PROGRAM_LIMIT",
                    "exported program exceeds the 8 MB service limit",
                ));
            }
            let decoded = prepared.decode_program(
                trusted.plan(),
                prepared.output_decimal_places,
                &export.program.gcode,
            )?;
            let sample_limit = sample_limit
                .unwrap_or(2_048)
                .clamp(1, KNIFE_EVIDENCE_MAX_SAMPLES);
            let report = build_evidence(
                trusted.plan(),
                &prepared,
                &decoded,
                &export.report.program_sha256,
                sample_limit,
            )?;
            Ok(json!({
                "report": report,
                "stock": {
                    "thicknessMm": job.setup.stock.thickness_mm,
                    "xy": job.setup.stock.xy,
                    "hasKnifeStages": trusted.plan().stages.iter()
                        .any(|stage| stage.role == cam_core::sequence::StageRole::Knife),
                },
                "summary": plan_summary(trusted.plan(), &export.report.basic_checks),
                "inspection": inspect_plan_value(trusted.plan())?,
            }))
        }
        CollectionCommand::Capabilities => Ok(json!({
            "apiVersion": COLLECTION_API_VERSION,
            "engineVersion": ENGINE_VERSION,
            "operationKinds": ["flat_vcarve", "face", "profile", "drag_knife"],
            "planScopes": ["all_enabled", "through_operation"],
            "outputLayouts": ["one_program", "sequential_files"],
            "features": {
                "artworkCollection": true,
                "cuttingProfiles": true,
                "appliedMachineConfiguration": true,
                "scopeMappingValidation": true,
                "usedByIndex": true,
                "planInspection": true,
                "knifeEmittedEvidence": true,
                "knifeContactChecks": true,
                "legacyJobMigration": true,
                "motionPaging": true,
                "retainedExecution": true,
                "retainedPaging": true,
                "preparedBundles": true,
                "sequentialFiles": true,
                "exactByteRetry": true,
            },
            "limits": {
                "pageMotions": crate::task::PAGE_MOTIONS,
                "jobBytes": crate::document::JOB_BYTES,
                "libraryBytes": tool_library::MAX_LIBRARY_BYTES,
                "programBytes": crate::export::PROGRAM_BYTES,
                "retainedPlans": crate::retained::RETAINED_PLANS,
                "retainedBundles": crate::retained::RETAINED_BUNDLES,
                "retainedTasks": crate::retained::RETAINED_TASKS,
            },
        })),
        CollectionCommand::Generate { .. }
        | CollectionCommand::TaskStatus { .. }
        | CollectionCommand::CancelTask { .. }
        | CollectionCommand::ReadMotions { .. }
        | CollectionCommand::ReadInspection { .. }
        | CollectionCommand::PrepareOutput { .. }
        | CollectionCommand::PreparedOutput { .. }
        | CollectionCommand::ReadPreparedBytes { .. } => Err(error(
            "RETAINED_STATE_REQUIRED",
            "retained-artifact commands need the retained runtime; route them through the stateful entry",
        )),
    }
}

/// Admission for ui-9 requests: same identity rules as ui-8, its own version.
pub fn validate_identity(
    api_version: &str,
    instance_id: &str,
    request_id: &str,
    revision: u64,
    expected_instance: &str,
) -> std::result::Result<(), crate::admission::Failure> {
    if api_version != COLLECTION_API_VERSION || instance_id != expected_instance {
        return Err(crate::admission::Failure::new(
            409,
            "TASK_INSTANCE",
            "The service changed. Reconnect; previous tasks are not replayed.",
        ));
    }
    if request_id.is_empty()
        || request_id.len() > 128
        || !request_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || revision > 9_007_199_254_740_991
    {
        return Err(crate::admission::Failure::new(
            400,
            "REQUEST_IDENTITY",
            "A short request ID and a safe revision are required.",
        ));
    }
    Ok(())
}

/// The ui-9 envelope shared by both adapters.
pub fn envelope(request_id: &str, revision: u64, data: Result<Value>) -> Value {
    let mut envelope = json!({
        "apiVersion": COLLECTION_API_VERSION,
        "engineVersion": ENGINE_VERSION,
        "requestId": request_id,
        "revision": revision,
    });
    match data {
        Ok(data) => {
            envelope["data"] = data;
        }
        Err(diagnostic) => {
            envelope["diagnostic"] = json!(UiDiagnostic::from(diagnostic));
        }
    }
    envelope
}
