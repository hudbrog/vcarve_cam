//! ui-8 sequence DTOs: canonical CamJob documents, operation-list editing,
//! sequence planning and sequence export through one shared projection for
//! the native HTTP service and the WebAssembly worker.
//!
//! The legacy ui-7 transport keeps serving legacy jobs unchanged; these
//! commands never silently plan a subset of a multi-operation document
//! (plan scopes are explicit).
use crate::document::{ENGINE_VERSION, UiDiagnostic};
use cam_core::{
    checks::{BasicCheckReport, check_plan},
    geometry::{Diagnostic, Result},
    operations,
    post::sequence::{PreparedExecution, SequenceProfile, SequenceProgram, apply_legacy_profile},
    project::{CamJob, OperationSettings, migrate::migrate_legacy_json},
    sequence::{ExecutionItem, OperationPlan, PlanLimits, TrustedPlan},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const SEQUENCE_API_VERSION: &str = "ui-8";

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("sequence")
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SequenceRequest {
    pub api_version: String,
    pub request_id: String,
    pub revision: u64,
    pub command: SequenceCommand,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "camelCase", deny_unknown_fields)]
pub enum SequenceCommand {
    /// Open a legacy (schema 1-3) or canonical (schema 4) job document.
    Open { json: String },
    /// Apply operation-list edits to a canonical job.
    Edit {
        job: Value,
        edits: Vec<OperationEdit>,
    },
    /// Apply a legacy schema-1 machine profile to a canonical job.
    ApplyProfile { job: Value, profile: Value },
    /// Replace one operation's settings (strict engine parsing; the UI never
    /// edits the canonical job in place).
    UpdateSettings {
        job: Value,
        operation_id: String,
        settings: Value,
    },
    /// Plan the enabled operations, all of them or a prefix.
    Plan { job: Value, scope: PlanScope },
    /// One page of the complete ordered motion stream of a scope (plan
    /// section 15.2: a bounded preview page is not a complete simulation).
    Motions {
        job: Value,
        scope: PlanScope,
        offset: usize,
    },
    /// The contour catalogue of the job's SVG source (plan section 7.1) for
    /// explicit per-contour selection in profile editing.
    Contours { job: Value },
    /// Export the plan with a schema-2 sequence profile.
    Export { job: Value, profile: Value },
    /// Apply a drag-knife tool (and optionally one typed preset) from a
    /// supplied library document to exactly one knife operation's
    /// assignment (F3d): sibling assignments and copied fields elsewhere
    /// are preserved, and the job gains an independent tool snapshot.
    ApplyKnifeTool {
        job: Value,
        library: Value,
        operation_id: String,
        tool_id: String,
        preset_id: Option<String>,
    },
    /// Bounded emitted-output knife evidence (F3b/F3d): plan, prepare,
    /// export, decode the actual bytes and replay them independently. The
    /// report binds to the exact program SHA-256; detail samples honor an
    /// explicit bounded page size.
    KnifeEvidence {
        job: Value,
        profile: Value,
        sample_limit: Option<usize>,
    },
    /// Advertised operation kinds and optional features.
    Capabilities,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PlanScope {
    AllEnabled,
    /// Plan the enabled prefix ending at this operation; later operations
    /// are excluded without implying they can run alone.
    ThroughOperation {
        operation_id: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "edit",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum OperationEdit {
    Rename {
        id: String,
        name: String,
    },
    SetEnabled {
        id: String,
        enabled: bool,
    },
    Move {
        id: String,
        to_index: usize,
    },
    Delete {
        id: String,
    },
    Duplicate {
        id: String,
        new_id: String,
    },
    /// Append a face or profile operation with default settings bound to an
    /// explicit existing tool. Nothing is invented: every machining value the
    /// defaults leave unset is reported through missing-field resolution.
    Add {
        id: String,
        name: String,
        kind: AddOperationKind,
        tool_id: String,
    },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddOperationKind {
    Face,
    Profile,
    DragKnife,
}

/// Document receipt for canonical jobs, deliberately distinct from planner
/// fingerprints and from ui-7 document receipts.
pub fn fingerprint(job: &CamJob) -> String {
    let mut hash = Sha256::new();
    hash.update(b"ui-sequence-v1\0");
    hash.update(ENGINE_VERSION.as_bytes());
    hash.update(job.to_json().expect("validated job serializes").as_bytes());
    format!("{:x}", hash.finalize())
}

fn parse_job(value: &Value) -> Result<CamJob> {
    let raw = value.to_string();
    if raw.len() > crate::document::JOB_BYTES {
        return Err(error("JOB_RESOURCE_LIMIT", "job exceeds 64 MB"));
    }
    CamJob::from_json(&raw)
}

fn kind_name(settings: &OperationSettings) -> &'static str {
    match settings {
        OperationSettings::FlatVcarve(_) => "flat_vcarve",
        OperationSettings::Face(_) => "face",
        OperationSettings::Profile(_) => "profile",
        OperationSettings::DragKnife(_) => "drag_knife",
    }
}

fn depth_summary(settings: &OperationSettings) -> Value {
    match settings {
        OperationSettings::FlatVcarve(s) => json!({
            "maxDepthMm": s.max_depth_mm,
            "mode": s.mode,
        }),
        OperationSettings::Face(s) => json!({"bottom": s.bottom, "top": s.top}),
        OperationSettings::Profile(s) => json!({"bottom": s.bottom, "top": s.top}),
        OperationSettings::DragKnife(s) => json!({"bottom": s.bottom, "top": s.top}),
    }
}

fn tool_ids(settings: &OperationSettings) -> Vec<String> {
    match settings {
        OperationSettings::FlatVcarve(s) => {
            vec![s.endmill.tool_id.clone(), s.vbit.tool_id.clone()]
        }
        OperationSettings::Face(s) => vec![s.assignment.tool_id.clone()],
        OperationSettings::Profile(s) => vec![s.assignment.tool_id.clone()],
        OperationSettings::DragKnife(s) => vec![s.assignment.tool_id.clone()],
    }
}

/// Missing editable machining values per operation, located by operation ID.
pub fn missing_by_operation(job: &CamJob) -> serde_json::Map<String, Value> {
    let mut missing = serde_json::Map::new();
    for operation in &job.operations {
        let entries: Vec<Value> = match &operation.settings {
            OperationSettings::FlatVcarve(settings) => {
                operations::flat_vcarve::missing_fields(job, &operation.id, settings)
                    .iter()
                    .map(|d| {
                        json!({
                            "fieldPath": d.field_path,
                            "message": d.message,
                            "toolId": d.tool_id,
                        })
                    })
                    .collect()
            }
            // Planners for these kinds ship in later slices; their settings
            // are reported as a whole until each slice defines resolution.
            OperationSettings::Face(settings) => {
                operations::face::missing_fields(job, &operation.id, settings)
                    .iter()
                    .map(|d| {
                        json!({
                            "fieldPath": d.field_path,
                            "message": d.message,
                            "toolId": d.tool_id,
                        })
                    })
                    .collect()
            }
            OperationSettings::Profile(settings) => {
                operations::profile::missing_fields(job, &operation.id, settings)
                    .iter()
                    .map(|d| {
                        json!({
                            "fieldPath": d.field_path,
                            "message": d.message,
                            "toolId": d.tool_id,
                        })
                    })
                    .collect()
            }
            OperationSettings::DragKnife(settings) => {
                operations::drag_knife::missing_fields(job, &operation.id, settings)
                    .iter()
                    .map(|d| {
                        json!({
                            "fieldPath": d.field_path,
                            "message": d.message,
                            "toolId": d.tool_id,
                        })
                    })
                    .collect()
            }
        };
        missing.insert(operation.id.clone(), Value::Array(entries));
    }
    missing
}

fn operations_projection(job: &CamJob) -> Value {
    json!(
        job.operations
            .iter()
            .map(|op| {
                json!({
                    "id": op.id,
                    "name": op.name,
                    "enabled": op.enabled,
                    "kind": kind_name(&op.settings),
                    "toolIds": tool_ids(&op.settings),
                    "depth": depth_summary(&op.settings),
                })
            })
            .collect::<Vec<_>>()
    )
}

fn document_projection(job: &CamJob, migrated: bool) -> Result<Value> {
    let stock_xy = job.setup.stock.xy.map(|rect| {
        json!({
            "minXmm": rect.min_x_mm, "minYmm": rect.min_y_mm,
            "widthMm": rect.width_mm, "lengthMm": rect.length_mm,
        })
    });
    Ok(json!({
        "job": serde_json::to_value(job).expect("validated job serializes"),
        "migrated": migrated,
        "operations": operations_projection(job),
        "missingByOperation": missing_by_operation(job),
        "setup": {
            "stock": {
                "thicknessMm": job.setup.stock.thickness_mm,
                // None keeps legacy unknown-XY behavior explicit (plan 9.3).
                "xy": stock_xy,
                "physicalXy": job.setup.stock.xy.is_some(),
            },
            "workZero": {"xy": job.setup.work_zero.xy, "z": job.setup.work_zero.z},
            "clearanceAboveStockMm": job.setup.clearance_above_stock_mm,
        },
        "documentFingerprint": fingerprint(job),
    }))
}

fn open(raw: &str) -> Result<Value> {
    if raw.len() > crate::document::JOB_BYTES {
        return Err(error("JOB_RESOURCE_LIMIT", "job exceeds 64 MB"));
    }
    let version: Value =
        serde_json::from_str(raw).map_err(|e| error("SEQUENCE_JOB_JSON", e.to_string()))?;
    let migrated = version.get("schema_version").and_then(Value::as_u64)
        != Some(cam_core::project::CAM_JOB_SCHEMA_VERSION as u64);
    let job = if migrated {
        migrate_legacy_json(raw)?
    } else {
        CamJob::from_json(raw)?
    };
    document_projection(&job, migrated)
}

fn apply_edits(mut job: CamJob, edits: &[OperationEdit]) -> Result<CamJob> {
    for edit in edits {
        match edit {
            OperationEdit::Rename { id, name } => {
                let Some(operation) = job.operations.iter_mut().find(|op| &op.id == id) else {
                    return Err(error(
                        "SEQUENCE_OPERATION_ID",
                        format!("cannot rename unknown operation '{id}'"),
                    ));
                };
                operation.name = name.clone();
            }
            OperationEdit::SetEnabled { id, enabled } => {
                let Some(operation) = job.operations.iter_mut().find(|op| &op.id == id) else {
                    return Err(error(
                        "SEQUENCE_OPERATION_ID",
                        format!("cannot toggle unknown operation '{id}'"),
                    ));
                };
                operation.enabled = *enabled;
            }
            OperationEdit::Move { id, to_index } => {
                let index = job
                    .operations
                    .iter()
                    .position(|op| op.id == *id)
                    .ok_or_else(|| {
                        error(
                            "SEQUENCE_OPERATION_ID",
                            format!("cannot move unknown operation '{id}'"),
                        )
                    })?;
                if *to_index >= job.operations.len() {
                    return Err(error(
                        "SEQUENCE_OPERATION_INDEX",
                        format!("move target {to_index} is outside the operation list"),
                    ));
                }
                let operation = job.operations.remove(index);
                job.operations.insert(*to_index, operation);
            }
            OperationEdit::Delete { id } => {
                let before = job.operations.len();
                job.operations.retain(|op| &op.id != id);
                if job.operations.len() == before {
                    return Err(error(
                        "SEQUENCE_OPERATION_ID",
                        format!("cannot delete unknown operation '{id}'"),
                    ));
                }
            }
            OperationEdit::Duplicate { id, new_id } => {
                let Some(source) = job.operations.iter().find(|op| &op.id == id) else {
                    return Err(error(
                        "SEQUENCE_OPERATION_ID",
                        format!("cannot duplicate unknown operation '{id}'"),
                    ));
                };
                let mut copy = source.clone();
                copy.id = new_id.clone();
                copy.name = format!("{} copy", source.name);
                let position = job
                    .operations
                    .iter()
                    .position(|op| op.id == *id)
                    .expect("source exists");
                job.operations.insert(position + 1, copy);
            }
            OperationEdit::Add {
                id,
                name,
                kind,
                tool_id,
            } => {
                if job.operations.iter().any(|op| op.id == *id) {
                    return Err(error(
                        "SEQUENCE_OPERATION_ID",
                        format!("operation ID '{id}' is already in use"),
                    ));
                }
                // Defaults leave every machining value unset (heights rest at
                // their zero-offset references until the editor supplies
                // them); the tool reference is the only supplied binding.
                let zero_top = cam_core::project::HeightRef {
                    reference: Default::default(),
                    offset_mm: 0.,
                };
                let settings = match kind {
                    AddOperationKind::Face => {
                        OperationSettings::Face(cam_core::project::FaceSettings {
                            area: cam_core::project::FaceArea::EntireStock,
                            margins: Default::default(),
                            entry_overrun_mm: None,
                            exit_overrun_mm: None,
                            top: zero_top.clone(),
                            bottom: zero_top,
                            stepdown_mm: None,
                            stepover_mm: None,
                            pass_angle_deg: None,
                            pattern: Default::default(),
                            assignment: cam_core::project::MillingAssignment {
                                tool_id: tool_id.clone(),
                                spindle_rpm: None,
                                spindle_direction: None,
                                cutting_feed_mm_min: None,
                                plunge_feed_mm_min: None,
                                max_stepdown_mm: None,
                                stepover_mm: None,
                            },
                        })
                    }
                    AddOperationKind::Profile => {
                        OperationSettings::Profile(cam_core::project::ProfileSettings {
                            contours: vec![],
                            assignment: cam_core::project::MillingAssignment {
                                tool_id: tool_id.clone(),
                                spindle_rpm: None,
                                spindle_direction: None,
                                cutting_feed_mm_min: None,
                                plunge_feed_mm_min: None,
                                max_stepdown_mm: None,
                                stepover_mm: None,
                            },
                            top: zero_top,
                            bottom: cam_core::project::HeightRef {
                                reference: cam_core::project::HeightReference::OperationTop,
                                offset_mm: 0.,
                            },
                            stepdown_mm: None,
                            through_cut_allowance_mm: None,
                            direction: None,
                            order: Default::default(),
                            start: Default::default(),
                            finish: Default::default(),
                            entry: Default::default(),
                            lead_in: Default::default(),
                            lead_out: Default::default(),
                            tabs: None,
                        })
                    }
                    // F3d: typed knife creation — chains, depths, feeds and
                    // the never-defaulted initial heading all stay unset
                    // until edited; nothing is invented here either.
                    AddOperationKind::DragKnife => {
                        OperationSettings::DragKnife(cam_core::project::DragKnifeSettings {
                            chains: vec![],
                            assignment: cam_core::project::KnifeAssignment {
                                tool_id: tool_id.clone(),
                                cutting_feed_mm_min: None,
                                plunge_feed_mm_min: None,
                                swivel_feed_mm_min: None,
                                max_stepdown_mm: None,
                            },
                            top: cam_core::project::HeightRef {
                                reference: Default::default(),
                                offset_mm: 0.,
                            },
                            bottom: cam_core::project::HeightRef {
                                reference: cam_core::project::HeightReference::OperationTop,
                                offset_mm: 0.,
                            },
                            stepdown_mm: None,
                            swivel_depth_mm: None,
                            corner_threshold_deg: None,
                            through_cut_allowance_mm: None,
                            start: Default::default(),
                            closure_overlap_mm: None,
                            alignment: Default::default(),
                        })
                    }
                };
                job.operations.push(cam_core::project::Operation {
                    id: id.clone(),
                    name: name.clone(),
                    enabled: true,
                    settings,
                });
            }
        }
        // Every intermediate state stays a valid document; an edit that
        // breaks references is rejected with the located engine diagnostic.
        job.validate()?;
    }
    Ok(job)
}

fn scoped_job(job: &CamJob, scope: &PlanScope) -> Result<CamJob> {
    match scope {
        PlanScope::AllEnabled => Ok(job.clone()),
        PlanScope::ThroughOperation { operation_id } => {
            let Some(end) = job.operations.iter().position(|op| op.id == *operation_id) else {
                return Err(error(
                    "SEQUENCE_OPERATION_ID",
                    format!("prefix scope references unknown operation '{operation_id}'"),
                ));
            };
            let mut scoped = job.clone();
            for operation in scoped.operations.iter_mut().skip(end + 1) {
                operation.enabled = false;
            }
            Ok(scoped)
        }
    }
}

fn plan_summary(plan: &OperationPlan, checks: &BasicCheckReport) -> Value {
    json!({
        "engineVersion": ENGINE_VERSION,
        "inputFingerprint": plan.input_fingerprint,
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
                // Resolved outputs (face planes, tab placements) so previews
                // show exactly what was generated.
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
        "execution": plan.execution.iter().map(|item| match item {
            ExecutionItem::ToolChange { tool_id } => json!({"toolChange": tool_id}),
            ExecutionItem::SetProcessIntent { intent } => json!({"processIntent": intent}),
            ExecutionItem::RunStage { stage_id } => json!({"runStage": stage_id}),
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

fn motion_page(plan: &OperationPlan, offset: usize) -> Result<Value> {
    if offset > plan.motions.len() {
        return Err(error(
            "SEQUENCE_MOTION_OFFSET",
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

fn parse_profile(value: &Value) -> Result<SequenceProfile> {
    let raw = value.to_string();
    if raw.len() > crate::export::PROFILE_BYTES {
        return Err(error("SEQUENCE_PROFILE_LIMIT", "profile exceeds 64 KB"));
    }
    SequenceProfile::from_json(&raw)
}

/// Execute one ui-8 command. Planning recomputes from the submitted document
/// on every call; retained-plan task leases arrive with the worker milestone.
pub fn execute(command: SequenceCommand) -> Result<Value> {
    match command {
        SequenceCommand::Open { json } => open(&json),
        SequenceCommand::Edit { job, edits } => {
            let edited = apply_edits(parse_job(&job)?, &edits)?;
            document_projection(&edited, false)
        }
        SequenceCommand::ApplyProfile { job, profile } => {
            let job = parse_job(&job)?;
            let raw = profile.to_string();
            if raw.len() > crate::export::PROFILE_BYTES {
                return Err(error("SEQUENCE_PROFILE_LIMIT", "profile exceeds 64 KB"));
            }
            let legacy = cam_core::post::LinuxCncProfile::from_json(&raw)?;
            let applied = apply_legacy_profile(&legacy, &job)?;
            document_projection(&applied, false)
        }
        SequenceCommand::UpdateSettings {
            job,
            operation_id,
            settings,
        } => {
            let mut job = parse_job(&job)?;
            let parsed: OperationSettings = serde_json::from_value(settings)
                .map_err(|e| error("SEQUENCE_SETTINGS_JSON", e.to_string()))?;
            let operation = job
                .operations
                .iter_mut()
                .find(|op| op.id == operation_id)
                .ok_or_else(|| {
                    error(
                        "SEQUENCE_OPERATION_ID",
                        format!("cannot update unknown operation '{operation_id}'"),
                    )
                })?;
            operation.settings = parsed;
            job.validate()?;
            document_projection(&job, false)
        }
        SequenceCommand::Plan { job, scope } => {
            let job = parse_job(&job)?;
            let scoped = scoped_job(&job, &scope)?;
            let plan = OperationPlan::plan_job(&scoped, &PlanLimits::default())?;
            let checks = check_plan(&plan)?;
            Ok(json!({
                "summary": plan_summary(&plan, &checks),
                "motions": motion_page(&plan, 0)?,
                "scope": scope,
            }))
        }
        SequenceCommand::Motions { job, scope, offset } => {
            let job = parse_job(&job)?;
            let scoped = scoped_job(&job, &scope)?;
            let plan = OperationPlan::plan_job(&scoped, &PlanLimits::default())?;
            Ok(json!({
                "motions": motion_page(&plan, offset)?,
                "scope": scope,
            }))
        }
        SequenceCommand::Contours { job } => {
            let job = parse_job(&job)?;
            let catalogue = cam_core::contours::ContourCatalogue::build(&job)?;
            let entry = |contour: &cam_core::contours::Contour| {
                let mut min_x = f64::INFINITY;
                let mut min_y = f64::INFINITY;
                let mut max_x = f64::NEG_INFINITY;
                let mut max_y = f64::NEG_INFINITY;
                for vertex in &contour.vertices {
                    min_x = min_x.min(vertex.x);
                    min_y = min_y.min(vertex.y);
                    max_x = max_x.max(vertex.x);
                    max_y = max_y.max(vertex.y);
                }
                json!({
                    "id": contour.id,
                    "componentId": contour.component_id,
                    "closed": contour.closed,
                    "role": match contour.role {
                        cam_core::contours::ContourRole::Outer => "outer",
                        cam_core::contours::ContourRole::Hole => "hole",
                        cam_core::contours::ContourRole::Open => "open",
                    },
                    "parentContourId": contour.parent_contour_id,
                    "perimeterMm": contour.perimeter_mm,
                    "suggestedSide": match contour.suggested_side() {
                        cam_core::project::ContourSide::Inside => "inside",
                        cam_core::project::ContourSide::Outside => "outside",
                        cam_core::project::ContourSide::On => "on",
                    },
                    // Anchors (starts, manual tabs) bind to the source
                    // geometry; the UI needs the fingerprint to build
                    // them and detect stale attachments.
                    "sourceFingerprint": contour.source_fingerprint,
                    "bounds": {
                        "minXmm": min_x, "minYmm": min_y, "maxXmm": max_x, "maxYmm": max_y,
                    },
                })
            };
            let contours: Vec<Value> = catalogue.contours.iter().map(entry).collect();
            // Open centerline chains (knife import) carry the same shape with
            // role "open"; their vertices stay in source order.
            let open_chains: Vec<Value> = catalogue.open_chains.iter().map(entry).collect();
            Ok(json!({ "contours": contours, "openChains": open_chains }))
        }
        SequenceCommand::Export { job, profile } => {
            let job = parse_job(&job)?;
            let profile = parse_profile(&profile)?;
            let plan = OperationPlan::plan_job(&job, &PlanLimits::default())?;
            let trusted = TrustedPlan::from_generated(plan);
            let prepared = PreparedExecution::prepare(&trusted, &profile)?;
            let export = prepared.export(&trusted, &profile)?;
            let program: &SequenceProgram = &export.program;
            if program.gcode.len() > crate::export::PROGRAM_BYTES {
                return Err(error(
                    "SEQUENCE_PROGRAM_LIMIT",
                    "exported program exceeds the 8 MB service limit",
                ));
            }
            Ok(json!({
                "program": program,
                "report": export.report,
                "documentFingerprint": fingerprint(&job),
            }))
        }
        SequenceCommand::ApplyKnifeTool {
            job,
            library,
            operation_id,
            tool_id,
            preset_id,
        } => {
            let job = parse_job(&job)?;
            let raw = library.to_string();
            if raw.len() > cam_core::tool_library::MAX_LIBRARY_BYTES {
                return Err(error("LIBRARY_RESOURCE_LIMIT", "tool library exceeds 8 MB"));
            }
            let library = cam_core::tool_library::ToolLibrary::from_json(&raw)?;
            let applied =
                library.apply_knife_to_job(&job, &operation_id, &tool_id, preset_id.as_deref())?;
            document_projection(&applied, false)
        }
        SequenceCommand::KnifeEvidence {
            job,
            profile,
            sample_limit,
        } => {
            use cam_core::operations::drag_knife::evidence::{
                KNIFE_EVIDENCE_MAX_SAMPLES, build_evidence,
            };
            let job = parse_job(&job)?;
            let profile = parse_profile(&profile)?;
            let plan = OperationPlan::plan_job(&job, &PlanLimits::default())?;
            let trusted = TrustedPlan::from_generated(plan);
            let prepared = PreparedExecution::prepare(&trusted, &profile)?;
            let export = prepared.export(&trusted, &profile)?;
            if export.program.gcode.len() > crate::export::PROGRAM_BYTES {
                return Err(error(
                    "SEQUENCE_PROGRAM_LIMIT",
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
                // Physical stock without requiring any milling tool: a
                // knife-only job still describes its stock and traces.
                "stock": {
                    "thicknessMm": job.setup.stock.thickness_mm,
                    "xy": job.setup.stock.xy,
                    "hasKnifeStages": trusted.plan().stages.iter()
                        .any(|stage| stage.role == cam_core::sequence::StageRole::Knife),
                },
                "summary": plan_summary(trusted.plan(), &export.report.basic_checks),
            }))
        }
        SequenceCommand::Capabilities => Ok(json!({
            "apiVersion": SEQUENCE_API_VERSION,
            "engineVersion": ENGINE_VERSION,
            // The knife planner ships its geometry in F2: the operation kind
            // is advertised and the independent replay gates its plans.
            "operationKinds": ["flat_vcarve", "face", "profile", "drag_knife"],
            "planScopes": ["all_enabled", "through_operation"],
            "features": {
                "openContours": true,
                "rampedTabs": false,
                "rotatedFacing": false,
                "profileFinishing": true,
                "profileEntries": true,
                "knifeReplay": true,
                "knifeToolLibrary": true,
                // F3: emitted-byte decoding, actual-output replay evidence
                // and prefix-contact checks ship with the knife integration.
                "knifeEmittedEvidence": true,
                "knifeContactChecks": true,
                "legacyJobMigration": true,
                "contourCatalogue": true,
                "motionPaging": true,
            },
            "limits": {
                "pageMotions": crate::task::PAGE_MOTIONS,
                "jobBytes": crate::document::JOB_BYTES,
            },
        })),
    }
}

/// Admission for ui-8 requests: same identity rules as ui-7, its own version.
pub fn validate_identity(
    api_version: &str,
    instance_id: &str,
    request_id: &str,
    revision: u64,
    expected_instance: &str,
) -> std::result::Result<(), crate::admission::Failure> {
    if api_version != SEQUENCE_API_VERSION || instance_id != expected_instance {
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

/// The ui-8 envelope shared by both adapters.
pub fn envelope(request_id: &str, revision: u64, data: Result<Value>) -> Value {
    let mut envelope = json!({
        "apiVersion": SEQUENCE_API_VERSION,
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
