//! Flat V-carve compatibility adapter (plan section 13.1).
//!
//! Wraps the existing endmill and combined planners without modifying their
//! motion order. The adapter is the only code that constructs a
//! [`crate::job::Job`] from the canonical model, and only for Flat V-carve
//! operations; new Face/Profile/Knife planners never build legacy jobs.
use crate::operations::LocatedDiagnostic;
use crate::{
    geometry::{Diagnostic, Region, Result},
    job::{
        Job as LegacyJob, MachineProfile as LegacyMachineProfile,
        OperationSettings as LegacyOperationSettings, SourceSnapshot as LegacySourceSnapshot,
        StockSettings as LegacyStockSettings, ToolGeometry as LegacyToolGeometry,
        ToolSettings as LegacyToolSettings,
    },
    model::{EndmillSpec, VBitSpec},
    operations::PlanContext,
    pocket::{EndmillPlanningSettings, EntryStrategy, plan_endmill_with_region},
    project::v5::{CamJobV5, FlatVcarveSettingsV5},
    project::{
        CamJob, FlatVcarveMode, FlatVcarveSettings, MillingAssignment, ToolCapabilities,
        ToolGeometry,
    },
    sequence::{
        CoolantIntent, GenerationStatus, LegacyPassEvidence, LegacyStageEvidence, LocalStage,
        PathControlIntent, PlanIssue, PlannedOperation, ProcessIntent, ProcessSpindle, StageRole,
        legacy_motion_mapping,
    },
    svg::Bounds,
    toolpath::PlannedMotion,
    vcarve::plan_combined_with_region,
};

fn incomplete(_operation_id: &str, issues: Vec<PlanIssue>) -> PlannedOperation {
    PlannedOperation {
        status: GenerationStatus::Incomplete,
        stages: vec![],
        motions: vec![],
        stage_evidence: vec![],
        pass_evidence: vec![],
        issues,
        preparation: vec![],
        named_outputs: vec![],
    }
}

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
    missing_fields_ctx(&PlanContext::from_v4(job), operation_id, settings, "source")
}

/// The collection-document variant of [`missing_fields`] (plan section 22.4):
/// identical requirements, with the artwork container field named for the
/// collection and the assignments carried by the schema-5 shapes.
pub(crate) fn missing_fields_v5(
    ctx: &PlanContext,
    operation_id: &str,
    settings: &FlatVcarveSettingsV5,
) -> Vec<LocatedDiagnostic> {
    let mapped = crate::project::v5::resolve::to_flat_vcarve_settings(settings);
    missing_fields_ctx(ctx, operation_id, &mapped, "artwork")
}

fn missing_fields_ctx(
    ctx: &PlanContext,
    operation_id: &str,
    settings: &FlatVcarveSettings,
    artwork_field: &str,
) -> Vec<LocatedDiagnostic> {
    let mut missing = vec![];
    let mut push = |path: String, what: &str| {
        missing.push(LocatedDiagnostic::missing(
            operation_id,
            &path,
            format!("set {what} before planning operation '{operation_id}'"),
        ));
    };
    if !ctx.has_artwork {
        push(artwork_field.into(), "an imported SVG source");
    }
    if ctx.setup.stock.thickness_mm.is_none() {
        push("setup.stock.thickness_mm".into(), "the stock thickness");
    }
    if ctx.setup.clearance_above_stock_mm.is_none() {
        push(
            "setup.clearance_above_stock_mm".into(),
            "the clearance plane",
        );
    }
    if ctx.setup.start_xy_mm.is_none() {
        push("setup.start_xy_mm".into(), "the start XY");
    }
    if ctx.tolerances.motion_tolerance_mm.is_none() {
        push(
            "tolerances.motion_tolerance_mm".into(),
            "the motion tolerance",
        );
    }
    if ctx.tolerances.verification_tolerance_mm.is_none() {
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
        let tool = ctx.tool(&assignment.tool_id);
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
        let endmill = ctx.tool(&settings.endmill.tool_id);
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
                if endmill.is_some_and(|t| t.capabilities.plunge_capable.is_none()) {
                    push(
                        format!("operations[{operation_id}].endmill.plunge_capable"),
                        "explicit endmill plunge capability",
                    );
                }
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
        let vbit = ctx.tool(&settings.vbit.tool_id);
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

/// Construct the legacy planning job for a collection Flat V-carve operation.
/// The legacy model requires exactly one source snapshot; in the collection
/// path the resolved region is the geometry authority and the snapshot is an
/// explicitly inert settings carrier — never parsed, never a synthetic SVG
/// for re-import (plan section 22.4).
pub(crate) fn to_legacy_job_v5(
    job: &CamJobV5,
    ctx: &PlanContext,
    operation_id: &str,
    settings: &FlatVcarveSettingsV5,
) -> Result<LegacyJob> {
    let mapped = crate::project::v5::resolve::to_flat_vcarve_settings(settings);
    let endmill_tool = ctx
        .tool(&settings.endmill.tool_id)
        .ok_or_else(|| error("PROJECT_TOOL_REFERENCE", "endmill tool not found"))?;
    let vbit_tool = ctx
        .tool(&settings.vbit.tool_id)
        .ok_or_else(|| error("PROJECT_TOOL_REFERENCE", "V-bit tool not found"))?;
    let rough = settings
        .rough
        .as_ref()
        .ok_or_else(|| error("MISSING_MACHINING_SETTING", "rough settings are required"))?;
    let endmill_planning = EndmillPlanningSettings {
        clearance_z_mm: ctx
            .setup
            .clearance_above_stock_mm
            .ok_or_else(|| error("MISSING_MACHINING_SETTING", "clearance is required"))?,
        start_xy_mm: ctx
            .setup
            .start_xy_mm
            .ok_or_else(|| error("MISSING_MACHINING_SETTING", "start XY is required"))?,
        strategy: rough.strategy,
        entry: rough.entry.clone(),
        max_layers: rough.max_layers,
        max_loops_per_layer: rough.max_loops_per_layer,
        max_motions: rough.max_motions,
    };
    let legacy = LegacyJob {
        schema_version: crate::job::JOB_SCHEMA_VERSION,
        name: job.name.clone(),
        source: LegacySourceSnapshot {
            filename: "collection".into(),
            svg: String::new(),
        },
        import: crate::svg::ImportOptions {
            geometry_tolerance_mm: 0.001,
            ticks_per_mm: None,
            placement: crate::svg::Placement::default(),
            mode: crate::svg::ImportMode::Fill,
        },
        selected_region_ids: mapped.component_ids.clone(),
        stock: LegacyStockSettings {
            thickness_mm: ctx.setup.stock.thickness_mm,
        },
        operation: LegacyOperationSettings {
            id: operation_id.into(),
            endmill_id: mapped.endmill.tool_id.clone(),
            vbit_id: mapped.vbit.tool_id.clone(),
            max_depth_mm: settings.max_depth_mm,
            wall_allowance_mm: settings.wall_allowance_mm,
            max_floor_ridge_mm: settings.max_floor_ridge_mm,
            max_detail_residual_mm: settings.max_detail_residual_mm,
        },
        tools: vec![
            legacy_tool(
                endmill_tool.id,
                endmill_tool.geometry.ok_or_else(|| {
                    error("MISSING_MACHINING_SETTING", "endmill geometry is required")
                })?,
                endmill_tool.capabilities,
                &mapped.endmill,
            )?,
            legacy_tool(
                vbit_tool.id,
                vbit_tool.geometry.ok_or_else(|| {
                    error("MISSING_MACHINING_SETTING", "V-bit geometry is required")
                })?,
                vbit_tool.capabilities,
                &mapped.vbit,
            )?,
        ],
        tolerances: ctx.tolerances.clone(),
        machine_profile: job.legacy_machine_profile.clone(),
        endmill_planning: Some(endmill_planning),
        vbit_planning: if settings.mode == FlatVcarveMode::Combined {
            settings.finish.clone()
        } else {
            None
        },
    };
    legacy.validate_settings()?;
    Ok(legacy)
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
        vbit_planning: if settings.mode == FlatVcarveMode::Combined {
            settings.finish.clone()
        } else {
            None
        },
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
                blade_heading_deg: None,
            }
        })
        .collect()
}

fn incomplete_from_missing(missing: Vec<LocatedDiagnostic>) -> PlannedOperation {
    PlannedOperation {
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
    }
}

/// The resolved carving top in setup coordinates, including uniform-surface
/// admission for face-referenced tops (plan section 13.2). Top resolution and
/// admission are generation outcomes, not document errors.
fn resolve_carve_top(
    ctx: &PlanContext,
    operation_id: &str,
    top: &crate::project::HeightRef,
    target_bounds: Option<Bounds>,
    published_faces: &std::collections::BTreeMap<String, crate::operations::PublishedFace>,
    prior_motions: &[PlannedMotion],
) -> Result<f64> {
    let planes: std::collections::BTreeMap<String, f64> = published_faces
        .iter()
        .map(|(id, face)| (id.clone(), face.z_mm))
        .collect();
    crate::setup::resolve_top_values(ctx.setup.stock.thickness_mm, top, &planes).and_then(|top_z| {
        if let crate::project::HeightReference::FaceResult {
            operation_id: face_id,
        } = &top.reference
        {
            admit_uniform_faced_top(
                operation_id,
                face_id,
                published_faces,
                target_bounds,
                prior_motions,
                top_z,
            )?;
        }
        Ok(top_z)
    })
}

/// Plan one Flat V-carve operation through the legacy engine and map its
/// output into generic rough/finish stages without reordering motions.
/// Missing editable values yield an incomplete, empty result carrying the
/// located diagnostics; the plan stays inspectable and cannot be exported.
pub(crate) fn plan(
    job: &CamJob,
    ctx: &PlanContext,
    operation_id: &str,
    settings: &FlatVcarveSettings,
    published_faces: &std::collections::BTreeMap<String, crate::operations::PublishedFace>,
    prior_motions: &[PlannedMotion],
) -> Result<PlannedOperation> {
    let missing = missing_fields_ctx(ctx, operation_id, settings, "source");
    if !missing.is_empty() {
        return Ok(incomplete_from_missing(missing));
    }
    // Resolve the selection's filled union once. The legacy planners consumed
    // the identical import through their own job inspection; the resolved-
    // region entry points and the collection path share this result (plan
    // section 22.4: one import, one union authority).
    let source = job.source.as_ref().expect("missing fields checked");
    let imported = crate::svg::import_svg(&source.svg, &job.import, Some(&settings.component_ids));
    let (region, bounds) = match imported {
        Ok(geometry) => (geometry.selected, geometry.bounds),
        Err(diagnostic) => {
            // A face-referenced top ran its admission through this import and
            // reported the failure as an incomplete generation; preserve that
            // exact behavior (a stock-top import failure stayed a hard error
            // inside the legacy planners).
            if matches!(
                settings.top.reference,
                crate::project::HeightReference::FaceResult { .. }
            ) {
                return Ok(incomplete(
                    operation_id,
                    vec![PlanIssue {
                        code: diagnostic.code,
                        message: diagnostic.message,
                        operation_id: Some(operation_id.into()),
                        stage_id: None,
                    }],
                ));
            }
            return Err(diagnostic);
        }
    };
    let top_z = match resolve_carve_top(
        ctx,
        operation_id,
        &settings.top,
        bounds,
        published_faces,
        prior_motions,
    ) {
        Ok(top_z) => top_z,
        Err(diagnostic) => {
            return Ok(incomplete(
                operation_id,
                vec![PlanIssue {
                    code: diagnostic.code,
                    message: diagnostic.message,
                    operation_id: Some(operation_id.into()),
                    stage_id: None,
                }],
            ));
        }
    };
    let mut legacy = to_legacy_job(job, operation_id, settings)?;
    if top_z != 0. {
        // local_z = setup_z - top_z for every target, query and report; the
        // physical stock bottom moves to local -(thickness + top_z) and the
        // clearance plane to clearance - top_z above the faced surface.
        let thickness = legacy.stock.thickness_mm.expect("missing fields checked") + top_z;
        legacy.stock.thickness_mm = Some(thickness);
        if let Some(planning) = &mut legacy.endmill_planning {
            planning.clearance_z_mm -= top_z;
        }
    }
    run_legacy(
        &legacy,
        region,
        operation_id,
        settings.mode,
        &settings.endmill,
        &settings.vbit,
        top_z,
    )
}

/// The collection-document entry (plan section 22.4): the resolved union of
/// the selected filled components arrives from the document resolver —
/// possibly spanning several artwork items — and no source is imported,
/// merged or synthesized here.
pub(crate) fn plan_v5(
    ctx: &PlanContext,
    job: &CamJobV5,
    operation_id: &str,
    settings: &FlatVcarveSettingsV5,
    resolved: &crate::project::v5::resolve::ResolvedVcarveRegion,
    published_faces: &std::collections::BTreeMap<String, crate::operations::PublishedFace>,
    prior_motions: &[PlannedMotion],
) -> Result<PlannedOperation> {
    let missing = missing_fields_v5(ctx, operation_id, settings);
    if !missing.is_empty() {
        return Ok(incomplete_from_missing(missing));
    }
    let top_z = match resolve_carve_top(
        ctx,
        operation_id,
        &settings.top,
        resolved.bounds,
        published_faces,
        prior_motions,
    ) {
        Ok(top_z) => top_z,
        Err(diagnostic) => {
            return Ok(incomplete(
                operation_id,
                vec![PlanIssue {
                    code: diagnostic.code,
                    message: diagnostic.message,
                    operation_id: Some(operation_id.into()),
                    stage_id: None,
                }],
            ));
        }
    };
    // Legacy-construction failures are generation outcomes in the collection
    // model (numeric readiness is planner-side): the rest of the job stays
    // inspectable instead of failing the whole plan.
    let mut legacy = match to_legacy_job_v5(job, ctx, operation_id, settings) {
        Ok(legacy) => legacy,
        Err(diagnostic) => {
            return Ok(incomplete(
                operation_id,
                vec![PlanIssue {
                    code: diagnostic.code,
                    message: diagnostic.message,
                    operation_id: Some(operation_id.into()),
                    stage_id: None,
                }],
            ));
        }
    };
    if top_z != 0. {
        let thickness = legacy.stock.thickness_mm.expect("missing fields checked") + top_z;
        legacy.stock.thickness_mm = Some(thickness);
        if let Some(planning) = &mut legacy.endmill_planning {
            planning.clearance_z_mm -= top_z;
        }
    }
    let mapped = crate::project::v5::resolve::to_flat_vcarve_settings(settings);
    run_legacy(
        &legacy,
        resolved.region.clone(),
        operation_id,
        mapped.mode,
        &mapped.endmill,
        &mapped.vbit,
        top_z,
    )
}

/// Shared machining core over a fully constructed legacy job and the resolved
/// selected region. The schema-4 adapter and the collection planner meet
/// here; neither mode re-imports any source.
#[allow(clippy::too_many_arguments)]
fn run_legacy(
    legacy: &LegacyJob,
    region: Region,
    operation_id: &str,
    mode: FlatVcarveMode,
    endmill: &MillingAssignment,
    vbit: &MillingAssignment,
    top_z: f64,
) -> Result<PlannedOperation> {
    let rough_stage_id = format!("{operation_id}-vcarve-rough");
    let finish_stage_id = format!("{operation_id}-vcarve-finish");
    match mode {
        FlatVcarveMode::EndmillOnly => {
            let plan = plan_endmill_with_region(legacy, region)?;
            let mut motions = map_motions(
                &plan.motions,
                operation_id,
                &rough_stage_id,
                &endmill.tool_id,
                false,
                |_| 0,
            );
            shift_to_setup(&mut motions, top_z);
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
                    tool_id: endmill.tool_id.clone(),
                    motion_range: (0, motions.len()),
                    intent: milling_intent(endmill)?,
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
            let plan = plan_combined_with_region(legacy, region)?;
            let mut endmill_motions = map_motions(
                &plan.endmill.motions,
                operation_id,
                &rough_stage_id,
                &endmill.tool_id,
                false,
                |_| 0,
            );
            shift_to_setup(&mut endmill_motions, top_z);
            // V-bit pass identity comes from the recorded executions; motions
            // outside any execution keep the fallback pass 0.
            let mut pass_by_motion = std::collections::BTreeMap::new();
            for (pass, execution) in plan.executions.iter().enumerate() {
                for id in execution.first_motion_id..execution.end_motion_id {
                    pass_by_motion.insert(id, pass);
                }
            }
            let mut vbit_motions = map_motions(
                &plan.vbit_motions,
                operation_id,
                &finish_stage_id,
                &vbit.tool_id,
                true,
                |id| pass_by_motion.get(&id).copied().unwrap_or(0),
            );
            shift_to_setup(&mut vbit_motions, top_z);
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
                    tool_id: endmill.tool_id.clone(),
                    motion_range: (0, rough_end),
                    intent: milling_intent(endmill)?,
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
                    tool_id: vbit.tool_id.clone(),
                    motion_range: (rough_end, motions.len()),
                    intent: milling_intent(vbit)?,
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

/// Shift local-frame legacy motions back into setup coordinates.
fn shift_to_setup(motions: &mut [PlannedMotion], top_z: f64) {
    for motion in motions.iter_mut() {
        motion.start.z += top_z;
        motion.end.z += top_z;
    }
}

/// Uniform-top admission (plan section 13.2): a face-referenced carve may run
/// only where the referenced face established a plane across the whole carve
/// target, and no earlier removal went below that plane inside the target.
/// The target bounds arrive resolved — one source or a cross-item union — so
/// admission never re-imports geometry.
#[allow(clippy::too_many_arguments)]
fn admit_uniform_faced_top(
    operation_id: &str,
    face_id: &str,
    published_faces: &std::collections::BTreeMap<String, crate::operations::PublishedFace>,
    target_bounds: Option<Bounds>,
    prior_motions: &[PlannedMotion],
    top_z: f64,
) -> Result<()> {
    let Some(face) = published_faces.get(face_id) else {
        return Err(error(
            "HEIGHT_REFERENCE_UNRESOLVED",
            format!(
                "operation '{operation_id}' references face '{face_id}' whose plane is not established"
            ),
        ));
    };
    let Some(bounds) = target_bounds else {
        return Err(error(
            "SURFACE_REFERENCE_OUTSIDE_COVERAGE",
            format!(
                "operation '{operation_id}' selects no geometry to bound against the faced plane"
            ),
        ));
    };
    let covered_max_x = face.covered.min_x_mm + face.covered.width_mm;
    let covered_max_y = face.covered.min_y_mm + face.covered.length_mm;
    let margin = 1e-9;
    if bounds.min.x < face.covered.min_x_mm - margin
        || bounds.max.x > covered_max_x + margin
        || bounds.min.y < face.covered.min_y_mm - margin
        || bounds.max.y > covered_max_y + margin
    {
        return Err(error(
            "SURFACE_REFERENCE_OUTSIDE_COVERAGE",
            format!(
                "operation '{operation_id}' selects geometry outside the area faced by '{face_id}'; no assumed global surface shift"
            ),
        ));
    }
    // Earlier removal strictly below the faced plane inside the target would
    // invalidate the fresh-planar starting surface the legacy engine assumes.
    let epsilon = 1e-9;
    for motion in prior_motions {
        if motion.effect != crate::toolpath::MotionEffect::MillingSweep {
            continue;
        }
        let touches_target = overlaps_bounds(
            motion,
            bounds.min.x,
            bounds.min.y,
            bounds.max.x,
            bounds.max.y,
        );
        if !touches_target {
            continue;
        }
        if motion.start.z.min(motion.end.z) < top_z - epsilon {
            return Err(error(
                "SURFACE_REFERENCE_OUTSIDE_COVERAGE",
                format!(
                    "earlier milling inside the carve target reached below the faced plane;                      '{}' cannot assume a fresh planar surface",
                    operation_id
                ),
            ));
        }
    }
    Ok(())
}

fn overlaps_bounds(motion: &PlannedMotion, min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> bool {
    let a = motion.start;
    let b = motion.end;
    let motion_min_x = a.x.min(b.x);
    let motion_max_x = a.x.max(b.x);
    let motion_min_y = a.y.min(b.y);
    let motion_max_y = a.y.max(b.y);
    motion_min_x <= max_x && motion_max_x >= min_x && motion_min_y <= max_y && motion_max_y >= min_y
}
