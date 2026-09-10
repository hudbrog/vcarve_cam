//! Flat V-carve compatibility adapter (plan section 13.1).
//!
//! Wraps the existing endmill and combined planners without modifying their
//! motion order. The adapter is the only code that constructs a
//! [`crate::job::Job`] from the canonical model, and only for Flat V-carve
//! operations; new Face/Profile/Knife planners never build legacy jobs.
use crate::operations::LocatedDiagnostic;
use crate::{
    geometry::{Diagnostic, Result},
    job::{
        Job as LegacyJob, MachineProfile as LegacyMachineProfile,
        OperationSettings as LegacyOperationSettings, StockSettings as LegacyStockSettings,
        ToolGeometry as LegacyToolGeometry, ToolSettings as LegacyToolSettings,
    },
    model::{EndmillSpec, VBitSpec},
    pocket::{EndmillPlanningSettings, EntryStrategy, plan_endmill},
    project::{
        CamJob, FlatVcarveMode, FlatVcarveSettings, MillingAssignment, ToolCapabilities,
        ToolGeometry,
    },
    sequence::{
        CoolantIntent, GenerationStatus, LegacyPassEvidence, LegacyStageEvidence, LocalStage,
        PathControlIntent, PlanIssue, PlannedOperation, ProcessIntent, ProcessSpindle, StageRole,
        legacy_motion_mapping,
    },
    toolpath::PlannedMotion,
    vcarve::plan_combined,
};

fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("flat_vcarve")
}

/// Required-but-unset fields for planning this operation. Supplied invalid
/// values are document errors caught by validation; this list is only about
/// missing editable values.
pub fn missing_fields(
    job: &CamJob,
    operation_id: &str,
    settings: &FlatVcarveSettings,
) -> Vec<LocatedDiagnostic> {
    let mut missing = vec![];
    let mut push = |path: String, what: &str| {
        missing.push(LocatedDiagnostic::missing(
            operation_id,
            &path,
            format!("set {what} before planning operation '{operation_id}'"),
        ));
    };
    if job.source.is_none() {
        push("source".into(), "an imported SVG source");
    }
    if job.setup.stock.thickness_mm.is_none() {
        push("setup.stock.thickness_mm".into(), "the stock thickness");
    }
    if job.setup.clearance_above_stock_mm.is_none() {
        push(
            "setup.clearance_above_stock_mm".into(),
            "the clearance plane",
        );
    }
    if job.setup.start_xy_mm.is_none() {
        push("setup.start_xy_mm".into(), "the start XY");
    }
    if job.tolerances.motion_tolerance_mm.is_none() {
        push(
            "tolerances.motion_tolerance_mm".into(),
            "the motion tolerance",
        );
    }
    if job.tolerances.verification_tolerance_mm.is_none() {
        push(
            "tolerances.verification_tolerance_mm".into(),
            "the verification tolerance",
        );
    }
    if settings.component_ids.is_empty() {
        push(
            format!("operations[{operation_id}].component_ids"),
            "at least one selected component",
        );
    }
    if settings.max_depth_mm.is_none() {
        push(
            format!("operations[{operation_id}].max_depth_mm"),
            "the carve depth",
        );
    }
    if settings.wall_allowance_mm.is_none() {
        push(
            format!("operations[{operation_id}].wall_allowance_mm"),
            "the wall allowance",
        );
    }
    if settings.rough.is_none() {
        push(
            format!("operations[{operation_id}].rough"),
            "the rough-stage settings",
        );
    }
    if settings.mode == FlatVcarveMode::Combined {
        if settings.finish.is_none() {
            push(
                format!("operations[{operation_id}].finish"),
                "the V-bit finish settings",
            );
        }
        if settings.max_floor_ridge_mm.is_none() {
            push(
                format!("operations[{operation_id}].max_floor_ridge_mm"),
                "the floor ridge limit",
            );
        }
        if settings.max_detail_residual_mm.is_none() {
            push(
                format!("operations[{operation_id}].max_detail_residual_mm"),
                "the detail residual limit",
            );
        }
    }
    for (role, assignment, requires_values) in [
        ("endmill", &settings.endmill, true),
        // Endmill-only planning never consumes the V-bit cutting values; its
        // geometry still defines the nominal target (checked below).
        (
            "vbit",
            &settings.vbit,
            settings.mode == FlatVcarveMode::Combined,
        ),
    ] {
        let tool = job.tools.iter().find(|t| t.id == assignment.tool_id);
        if tool.is_some_and(|t| t.geometry.is_none()) {
            push(format!("tools[{role}].geometry"), "the tool geometry");
        }
        if requires_values {
            for (value, name) in [
                (assignment.spindle_rpm, "spindle_rpm"),
                (assignment.cutting_feed_mm_min, "cutting_feed_mm_min"),
                (assignment.plunge_feed_mm_min, "plunge_feed_mm_min"),
                (assignment.max_stepdown_mm, "max_stepdown_mm"),
                (assignment.stepover_mm, "stepover_mm"),
            ] {
                if value.is_none() {
                    push(format!("operations[{operation_id}].{role}.{name}"), name);
                }
            }
        }
    }
    // Entry capability per stage, mirroring the legacy planners' checks.
    if let Some(rough) = &settings.rough {
        let endmill = job.tools.iter().find(|t| t.id == settings.endmill.tool_id);
        match rough.entry {
            EntryStrategy::Plunge => {
                if endmill.is_none_or(|t| t.capabilities.plunge_capable != Some(true)) {
                    push(
                        format!("operations[{operation_id}].endmill.plunge_capable"),
                        "explicit endmill plunge capability",
                    );
                }
            }
            EntryStrategy::Ramp { .. } => {
                if endmill.is_none_or(|t| t.capabilities.ramp_capable != Some(true)) {
                    push(
                        format!("operations[{operation_id}].endmill.ramp_capable"),
                        "explicit endmill ramp capability",
                    );
                }
            }
        }
    }
    if settings.mode == FlatVcarveMode::Combined {
        let vbit = job.tools.iter().find(|t| t.id == settings.vbit.tool_id);
        if vbit.is_none_or(|t| t.capabilities.plunge_capable.is_none()) {
            push(
                format!("operations[{operation_id}].vbit.plunge_capable"),
                "explicit V-bit plunge capability",
            );
        }
    }
    missing
}

fn legacy_tool(
    id: &str,
    geometry: &ToolGeometry,
    capabilities: &ToolCapabilities,
    assignment: &MillingAssignment,
) -> Result<LegacyToolSettings> {
    let geometry = match geometry {
        ToolGeometry::Endmill(g) => LegacyToolGeometry::Endmill(EndmillSpec {
            diameter_mm: g.diameter_mm,
            cutting_length_mm: g.cutting_length_mm,
            plunge_capable: capabilities.plunge_capable.ok_or_else(|| {
                error(
                    "MISSING_MACHINING_SETTING",
                    "endmill plunge capability is required before planning",
                )
            })?,
        }),
        ToolGeometry::Vbit(spec) => LegacyToolGeometry::Vbit(VBitSpec {
            included_angle_deg: spec.included_angle_deg,
            tip_diameter_mm: spec.tip_diameter_mm,
            max_cutting_diameter_mm: spec.max_cutting_diameter_mm,
            cutting_height_mm: spec.cutting_height_mm,
        }),
        ToolGeometry::DragKnife(_) => {
            return Err(error(
                "LEGACY_TOOL_KIND",
                "drag-knife geometry has no legacy V-carve representation",
            ));
        }
    };
    // The endmill spec already carries the authoritative plunge value; the
    // legacy slot field is only meaningful for V-bit tools.
    let slot_plunge_capable = match &geometry {
        LegacyToolGeometry::Endmill(_) => None,
        _ => capabilities.plunge_capable,
    };
    Ok(LegacyToolSettings {
        id: id.into(),
        geometry: Some(geometry),
        spindle_rpm: assignment.spindle_rpm,
        cutting_feed_mm_min: assignment.cutting_feed_mm_min,
        plunge_feed_mm_min: assignment.plunge_feed_mm_min,
        max_stepdown_mm: assignment.max_stepdown_mm,
        stepover_mm: assignment.stepover_mm,
        ramp_capable: capabilities.ramp_capable,
        plunge_capable: slot_plunge_capable,
    })
}

/// Construct the legacy job for exactly this operation's component selection,
/// assignments and planning controls. Shared setup travel settings and the
/// canonical tolerances travel with the job; nothing else is invented.
pub fn to_legacy_job(
    job: &CamJob,
    operation_id: &str,
    settings: &FlatVcarveSettings,
) -> Result<LegacyJob> {
    let missing = missing_fields(job, operation_id, settings);
    if let Some(first) = missing.first() {
        return Err(first.diagnostic());
    }
    let endmill_tool = job
        .tools
        .iter()
        .find(|t| t.id == settings.endmill.tool_id)
        .ok_or_else(|| error("PROJECT_TOOL_REFERENCE", "endmill tool not found"))?;
    let vbit_tool = job
        .tools
        .iter()
        .find(|t| t.id == settings.vbit.tool_id)
        .ok_or_else(|| error("PROJECT_TOOL_REFERENCE", "V-bit tool not found"))?;
    let rough = settings
        .rough
        .as_ref()
        .ok_or_else(|| error("MISSING_MACHINING_SETTING", "rough settings are required"))?;
    let endmill_planning = EndmillPlanningSettings {
        clearance_z_mm: job
            .setup
            .clearance_above_stock_mm
            .ok_or_else(|| error("MISSING_MACHINING_SETTING", "clearance is required"))?,
        start_xy_mm: job
            .setup
            .start_xy_mm
            .ok_or_else(|| error("MISSING_MACHINING_SETTING", "start XY is required"))?,
        strategy: rough.strategy,
        entry: rough.entry.clone(),
        max_layers: rough.max_layers,
        max_loops_per_layer: rough.max_loops_per_layer,
        max_motions: rough.max_motions,
    };
    let machine_profile: Option<LegacyMachineProfile> = job.legacy_machine_profile.clone();
    let legacy = LegacyJob {
        schema_version: crate::job::JOB_SCHEMA_VERSION,
        name: job.name.clone(),
        source: job.source.clone().ok_or_else(|| {
            error(
                "MISSING_MACHINING_SETTING",
                "an imported source is required",
            )
        })?,
        import: job.import.clone(),
        selected_region_ids: settings.component_ids.clone(),
        stock: LegacyStockSettings {
            thickness_mm: job.setup.stock.thickness_mm,
        },
        operation: LegacyOperationSettings {
            id: operation_id.into(),
            endmill_id: settings.endmill.tool_id.clone(),
            vbit_id: settings.vbit.tool_id.clone(),
            max_depth_mm: settings.max_depth_mm,
            wall_allowance_mm: settings.wall_allowance_mm,
            max_floor_ridge_mm: settings.max_floor_ridge_mm,
            max_detail_residual_mm: settings.max_detail_residual_mm,
        },
        tools: vec![
            legacy_tool(
                &endmill_tool.id,
                endmill_tool.geometry.as_ref().ok_or_else(|| {
                    error("MISSING_MACHINING_SETTING", "endmill geometry is required")
                })?,
                &endmill_tool.capabilities,
                &settings.endmill,
            )?,
            legacy_tool(
                &vbit_tool.id,
                vbit_tool.geometry.as_ref().ok_or_else(|| {
                    error("MISSING_MACHINING_SETTING", "V-bit geometry is required")
                })?,
                &vbit_tool.capabilities,
                &settings.vbit,
            )?,
        ],
        tolerances: job.tolerances.clone(),
        machine_profile,
        endmill_planning: Some(endmill_planning),
        vbit_planning: settings.finish.clone(),
    };
    legacy.validate_settings()?;
    Ok(legacy)
}

fn milling_intent(assignment: &MillingAssignment) -> Result<ProcessIntent> {
    let rpm = assignment.spindle_rpm.ok_or_else(|| {
        error(
            "MISSING_MACHINING_SETTING",
            "spindle RPM is required for a milling stage",
        )
    })?;
    Ok(ProcessIntent {
        spindle: ProcessSpindle::Milling {
            rpm,
            direction: assignment.spindle_direction,
        },
        coolant: CoolantIntent::UseMachineProfile,
        path_control: PathControlIntent::UseMachineProfile,
    })
}

/// Issue codes that mark an incomplete (not inconclusive) legacy generation.
const INCOMPLETE_ISSUE_CODES: [&str; 3] = [
    "UNSUPPORTED_ENTRY",
    "LOOP_CLEARANCE",
    "UNSUPPORTED_VBIT_ENTRY",
];

fn status_from(issues: &[PlanIssue], motions_empty: bool, shortfall: bool) -> GenerationStatus {
    if motions_empty && issues.is_empty() && !shortfall {
        return GenerationStatus::Empty;
    }
    let uncertain = issues
        .iter()
        .any(|issue| !INCOMPLETE_ISSUE_CODES.contains(&issue.code.as_str()));
    if uncertain {
        GenerationStatus::Inconclusive
    } else if !issues.is_empty() || shortfall {
        GenerationStatus::Incomplete
    } else {
        GenerationStatus::Complete
    }
}

fn map_motions(
    legacy_motions: &[crate::motion::Motion],
    operation_id: &str,
    stage_id: &str,
    tool_id: &str,
    finishing: bool,
    pass_of: impl Fn(usize) -> usize,
) -> Vec<PlannedMotion> {
    legacy_motions
        .iter()
        .map(|m| {
            let (interpolation, purpose, effect) = legacy_motion_mapping(m.kind, finishing);
            PlannedMotion {
                id: m.id,
                operation_id: operation_id.into(),
                stage_id: stage_id.into(),
                tool_id: tool_id.into(),
                contour_id: None,
                pass_id: pass_of(m.id),
                layer: m.layer,
                interpolation,
                purpose,
                effect,
                start: m.start,
                end: m.end,
                feed_mm_min: m.feed_mm_min,
            }
        })
        .collect()
}

/// Plan one Flat V-carve operation through the legacy engine and map its
/// output into generic rough/finish stages without reordering motions.
/// Missing editable values yield an incomplete, empty result carrying the
/// located diagnostics; the plan stays inspectable and cannot be exported.
pub(crate) fn plan(
    job: &CamJob,
    operation_id: &str,
    settings: &FlatVcarveSettings,
) -> Result<PlannedOperation> {
    let missing = missing_fields(job, operation_id, settings);
    if !missing.is_empty() {
        return Ok(PlannedOperation {
            status: GenerationStatus::Incomplete,
            stages: vec![],
            motions: vec![],
            stage_evidence: vec![],
            pass_evidence: vec![],
            issues: missing
                .iter()
                .map(|d| PlanIssue {
                    code: d.code.clone(),
                    message: format!("{} ({})", d.message, d.field_path.as_deref().unwrap_or("")),
                    operation_id: d.operation_id.clone(),
                    stage_id: None,
                })
                .collect(),
            preparation: vec![],
            named_outputs: vec![],
        });
    }
    let legacy = to_legacy_job(job, operation_id, settings)?;
    let rough_stage_id = format!("{operation_id}-vcarve-rough");
    let finish_stage_id = format!("{operation_id}-vcarve-finish");
    match settings.mode {
        FlatVcarveMode::EndmillOnly => {
            let plan = plan_endmill(&legacy)?;
            let motions = map_motions(
                &plan.motions,
                operation_id,
                &rough_stage_id,
                &settings.endmill.tool_id,
                false,
                |_| 0,
            );
            let issues: Vec<PlanIssue> = plan
                .generation_issues
                .iter()
                .map(|issue| PlanIssue {
                    code: issue.code.clone(),
                    message: issue.message.clone(),
                    operation_id: Some(operation_id.into()),
                    stage_id: Some(rough_stage_id.clone()),
                })
                .collect();
            let status = status_from(&issues, motions.is_empty(), false);
            Ok(PlannedOperation {
                status,
                stages: vec![LocalStage {
                    stage_id: rough_stage_id.clone(),
                    role: StageRole::VcarveRough,
                    tool_id: settings.endmill.tool_id.clone(),
                    motion_range: (0, motions.len()),
                    intent: milling_intent(&settings.endmill)?,
                }],
                motions,
                stage_evidence: vec![LegacyStageEvidence {
                    stage_id: rough_stage_id,
                    legacy_first_motion_id: plan.motions.first().map(|m| m.id).unwrap_or(0),
                    legacy_end_motion_id: plan.motions.last().map(|m| m.id + 1).unwrap_or(0),
                }],
                pass_evidence: vec![],
                issues,
                preparation: vec![],
                named_outputs: vec![],
            })
        }
        FlatVcarveMode::Combined => {
            let plan = plan_combined(&legacy)?;
            let endmill_motions = map_motions(
                &plan.endmill.motions,
                operation_id,
                &rough_stage_id,
                &settings.endmill.tool_id,
                false,
                |_| 0,
            );
            // V-bit pass identity comes from the recorded executions; motions
            // outside any execution keep the fallback pass 0.
            let mut pass_by_motion = std::collections::BTreeMap::new();
            for (pass, execution) in plan.executions.iter().enumerate() {
                for id in execution.first_motion_id..execution.end_motion_id {
                    pass_by_motion.insert(id, pass);
                }
            }
            let vbit_motions = map_motions(
                &plan.vbit_motions,
                operation_id,
                &finish_stage_id,
                &settings.vbit.tool_id,
                true,
                |id| pass_by_motion.get(&id).copied().unwrap_or(0),
            );
            let mut issues: Vec<PlanIssue> = plan
                .endmill
                .generation_issues
                .iter()
                .map(|issue| PlanIssue {
                    code: issue.code.clone(),
                    message: issue.message.clone(),
                    operation_id: Some(operation_id.into()),
                    stage_id: Some(rough_stage_id.clone()),
                })
                .collect();
            // The combined planner folds finish-path shortfall into its
            // analysis; surface it as an explicit generation diagnostic.
            if plan.analysis.finish_paths_expected != plan.analysis.finish_paths_executed {
                issues.push(PlanIssue {
                    code: "VBIT_FINISH_PATHS_MISSING".into(),
                    message: format!(
                        "{} of {} expected finish paths executed",
                        plan.analysis.finish_paths_executed, plan.analysis.finish_paths_expected
                    ),
                    operation_id: Some(operation_id.into()),
                    stage_id: Some(finish_stage_id.clone()),
                });
            }
            let rough_end = endmill_motions.len();
            let motions = [endmill_motions, vbit_motions].concat();
            let status = status_from(
                &issues,
                motions.is_empty(),
                plan.analysis.finish_paths_expected != plan.analysis.finish_paths_executed,
            );
            let pass_evidence: Vec<_> = plan
                .executions
                .iter()
                .enumerate()
                .map(|(pass, e)| LegacyPassEvidence {
                    pass_id: pass,
                    first_motion_id: e.first_motion_id,
                    end_motion_id: e.end_motion_id,
                    pass_depth_mm: e.pass_depth_mm,
                    final_finish: e.final_finish,
                })
                .collect();
            let endmill_empty = rough_end == 0;
            let mut stages = vec![];
            let mut evidence = vec![];
            if !endmill_empty {
                stages.push(LocalStage {
                    stage_id: rough_stage_id.clone(),
                    role: StageRole::VcarveRough,
                    tool_id: settings.endmill.tool_id.clone(),
                    motion_range: (0, rough_end),
                    intent: milling_intent(&settings.endmill)?,
                });
                evidence.push(LegacyStageEvidence {
                    stage_id: rough_stage_id,
                    legacy_first_motion_id: plan.endmill.motions.first().map(|m| m.id).unwrap_or(0),
                    legacy_end_motion_id: plan
                        .endmill
                        .motions
                        .last()
                        .map(|m| m.id + 1)
                        .unwrap_or(0),
                });
            }
            let finish_len = motions.len() - rough_end;
            if finish_len > 0 {
                stages.push(LocalStage {
                    stage_id: finish_stage_id.clone(),
                    role: StageRole::VcarveFinish,
                    tool_id: settings.vbit.tool_id.clone(),
                    motion_range: (rough_end, motions.len()),
                    intent: milling_intent(&settings.vbit)?,
                });
                evidence.push(LegacyStageEvidence {
                    stage_id: finish_stage_id,
                    legacy_first_motion_id: plan.vbit_motions.first().map(|m| m.id).unwrap_or(0),
                    legacy_end_motion_id: plan.vbit_motions.last().map(|m| m.id + 1).unwrap_or(0),
                });
            }
            Ok(PlannedOperation {
                status,
                stages,
                motions,
                stage_evidence: evidence,
                pass_evidence,
                issues,
                preparation: vec![],
                named_outputs: vec![],
            })
        }
    }
}
