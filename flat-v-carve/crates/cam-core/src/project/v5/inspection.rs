//! Read-only inspection DTOs for the schema-5 collection surface (plan
//! section 22.9, H4): used-by references, assignment statuses, the applied
//! machine readout, and plan-level heights/face/timeline/stock data derived
//! from an actual generated plan. Everything here is derived state — commands
//! mutate documents, inspection only reads them, so no DTO duplicates
//! document authority.
use super::{
    CamJobV5, ConfigurationOrigin, GeometryRef, GeometryRefKind, OperationSettingsV5,
    resources::{AssignmentRole, AssignmentStatus},
};
use crate::{
    geometry::Result,
    project::RectXY,
    sequence::{ExecutionStage, GenerationStatus, OperationPlanV5, StageRole},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One operation as navigator readouts see it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationSummary {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub enabled: bool,
}

/// Which operations address one artwork item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsedByArtworkItem {
    pub item_id: String,
    pub operation_ids: Vec<String>,
}

/// Which operations address one qualified geometry (item + local ID + kind).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsedByGeometry {
    pub item_id: String,
    pub local_geometry_id: String,
    pub kind: GeometryRefKind,
    pub operation_ids: Vec<String>,
}

/// One job tool with the assignments that use it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsedByJobTool {
    pub tool_id: String,
    /// False when an assignment references a tool the job does not carry
    /// (the located issue lives in reference inspection).
    pub exists: bool,
    pub assignments: Vec<AssignmentUse>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentUse {
    pub operation_id: String,
    pub role: AssignmentRole,
}

/// The used-by index of one document (plan section 22.9): items, qualified
/// geometries and job tools with their referencing operations/assignments.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsedByIndex {
    pub artwork_items: Vec<UsedByArtworkItem>,
    pub geometries: Vec<UsedByGeometry>,
    pub job_tools: Vec<UsedByJobTool>,
}

/// One mapping row annotated with document state the row itself cannot know.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MappingRowReadout {
    pub job_tool_id: String,
    pub tool_number: Option<u32>,
    pub length_offset_number: Option<u32>,
    /// False for dangling rows (repairable draft issues; plan section 22.7).
    pub tool_exists: bool,
    pub assignments: Vec<AssignmentUse>,
}

/// Flat readout of the one applied machine configuration. All fields are
/// `None` when the job has none; unset preparation fields stay `None` so the
/// UI can show exactly what is unresolved without re-deriving completeness.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineReadout {
    pub origin: Option<ConfigurationOrigin>,
    pub work_offset: Option<String>,
    pub clearance_z_mm: Option<f64>,
    pub decimal_places: Option<usize>,
    pub program_start_position_mm: Option<crate::motion::Position>,
    pub length_compensation: Option<crate::post::LengthCompensation>,
    pub path_control: Option<crate::post::PathControl>,
    pub spindle_spinup_seconds: Option<f64>,
    pub coolant: Option<crate::post::Coolant>,
    pub m6: Option<crate::post::M6Contract>,
    pub rows: Vec<MappingRowReadout>,
}

/// The whole document inspection payload.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentInspection {
    pub machining_order: Vec<OperationSummary>,
    pub used_by: UsedByIndex,
    pub assignments: Vec<AssignmentStatus>,
    pub machine: MachineReadout,
}

/// Planner-owned missing values, preserving the structured locations that
/// an editor needs to repair a single Flat V-carve operation.
pub fn inspect_flat_vcarve_fields(
    job: &CamJobV5,
    operation_id: &str,
) -> Result<Vec<crate::operations::LocatedDiagnostic>> {
    let operation = job
        .operations
        .iter()
        .find(|op| op.id == operation_id)
        .ok_or_else(|| super::error("OPERATION_NOT_FOUND", "Unknown operation"))?;
    let OperationSettingsV5::FlatVcarve(settings) = &operation.settings else {
        return Err(super::error("OPERATION_KIND", "Expected Flat V-carve"));
    };
    Ok(crate::operations::flat_vcarve::missing_fields_v5(
        &crate::operations::PlanContext::from_v5(job),
        operation_id,
        settings,
    ))
}

/// Knife editor diagnostics use the same required fields as the planner.
pub fn inspect_knife_fields(
    job: &CamJobV5,
    operation_id: &str,
) -> Result<Vec<crate::operations::LocatedDiagnostic>> {
    let operation = job
        .operations
        .iter()
        .find(|o| o.id == operation_id)
        .ok_or_else(|| super::error("OPERATION_REFERENCE", "Unknown operation"))?;
    let OperationSettingsV5::DragKnife(settings) = &operation.settings else {
        return Err(super::error("OPERATION_KIND", "Expected Drag knife"));
    };
    Ok(crate::operations::drag_knife::missing_fields_ctx(
        &crate::operations::PlanContext::from_v5(job),
        operation_id,
        &super::resolve::to_knife_settings(settings),
        "artwork",
    ))
}

/// Face editor diagnostics: the face planner's required-but-unset fields for
/// one operation (stock, clearance, tolerances, stepdown/stepover and the
/// complete milling assignment).
pub fn inspect_face_fields(
    job: &CamJobV5,
    operation_id: &str,
) -> Result<Vec<crate::operations::LocatedDiagnostic>> {
    let operation = job
        .operations
        .iter()
        .find(|op| op.id == operation_id)
        .ok_or_else(|| super::error("OPERATION_NOT_FOUND", "Unknown operation"))?;
    let OperationSettingsV5::Face(settings) = &operation.settings else {
        return Err(super::error("OPERATION_KIND", "Expected Face"));
    };
    Ok(crate::operations::face::missing_fields_ctx(
        &crate::operations::PlanContext::from_v5(job),
        operation_id,
        &super::resolve::to_face_settings(settings),
    ))
}

/// The face editor's pre-generation readout: the axis the passes run along,
/// every position the cutter descends at, how far its sweep clears the stock
/// there, and the travel that would clear it. The same resolution the planner
/// performs, so the panel cannot promise something the plan contradicts.
pub fn face_entry_preview(
    job: &CamJobV5,
    operation_id: &str,
) -> Result<Option<crate::operations::face::FaceEntryPreview>> {
    let operation = job
        .operations
        .iter()
        .find(|op| op.id == operation_id)
        .ok_or_else(|| super::error("OPERATION_NOT_FOUND", "Unknown operation"))?;
    let OperationSettingsV5::Face(settings) = &operation.settings else {
        return Err(super::error("OPERATION_KIND", "Expected Face"));
    };
    crate::operations::face::entry_preview(
        &crate::operations::PlanContext::from_v5(job),
        &super::resolve::to_face_settings(settings),
    )
}

/// Profile editor diagnostics: the profile planner's required-but-unset fields
/// for one operation (contour selection with explicit sides, stepdown,
/// direction, the milling assignment, and whichever tab/entry/lead values the
/// current selection actually requires).
pub fn inspect_profile_fields(
    job: &CamJobV5,
    operation_id: &str,
) -> Result<Vec<crate::operations::LocatedDiagnostic>> {
    let operation = job
        .operations
        .iter()
        .find(|op| op.id == operation_id)
        .ok_or_else(|| super::error("OPERATION_NOT_FOUND", "Unknown operation"))?;
    let OperationSettingsV5::Profile(settings) = &operation.settings else {
        return Err(super::error("OPERATION_KIND", "Expected Profile"));
    };
    Ok(crate::operations::profile::missing_fields_ctx(
        &crate::operations::PlanContext::from_v5(job),
        operation_id,
        &super::resolve::to_profile_settings(settings),
        "artwork",
    ))
}

/// Planner-owned required fields for one operation, dispatched by kind. Every
/// editor and the generation pre-check use this single entry so a new
/// operation kind cannot silently skip its own requirement list.
pub fn inspect_operation_fields(
    job: &CamJobV5,
    operation_id: &str,
) -> Result<Vec<crate::operations::LocatedDiagnostic>> {
    let operation = job
        .operations
        .iter()
        .find(|op| op.id == operation_id)
        .ok_or_else(|| super::error("OPERATION_NOT_FOUND", "Unknown operation"))?;
    match &operation.settings {
        OperationSettingsV5::FlatVcarve(_) => inspect_flat_vcarve_fields(job, operation_id),
        OperationSettingsV5::Face(_) => inspect_face_fields(job, operation_id),
        OperationSettingsV5::DragKnife(_) => inspect_knife_fields(job, operation_id),
        OperationSettingsV5::Profile(_) => inspect_profile_fields(job, operation_id),
    }
}

fn operation_kind(settings: &OperationSettingsV5) -> &'static str {
    match settings {
        OperationSettingsV5::FlatVcarve(_) => "flat_vcarve",
        OperationSettingsV5::Face(_) => "face",
        OperationSettingsV5::Profile(_) => "profile",
        OperationSettingsV5::DragKnife(_) => "drag_knife",
    }
}

/// Every qualified geometry reference one operation's settings address
/// (selections, starts and manual tab anchors).
fn geometry_refs(settings: &OperationSettingsV5) -> Vec<&GeometryRef> {
    fn anchor(anchor: &super::ContourAnchorV5) -> &GeometryRef {
        &anchor.geometry
    }
    fn tab_anchors(tabs: &Option<super::TabSettingsV5>) -> Vec<&GeometryRef> {
        match tabs {
            Some(super::TabSettingsV5 {
                placement: super::TabPlacementV5::Manual { anchors },
                ..
            }) => anchors.iter().map(anchor).collect(),
            _ => vec![],
        }
    }
    fn start(settings: &super::StartSelectionV5) -> Vec<&GeometryRef> {
        match settings {
            super::StartSelectionV5::Anchor(a) => vec![anchor(a)],
            super::StartSelectionV5::Automatic => vec![],
        }
    }
    match settings {
        OperationSettingsV5::FlatVcarve(s) => s.components.iter().collect(),
        OperationSettingsV5::Face(_) => vec![],
        OperationSettingsV5::Profile(s) => s
            .contours
            .iter()
            .map(|c| &c.geometry)
            .chain(start(&s.start))
            .chain(tab_anchors(&s.tabs))
            .collect(),
        OperationSettingsV5::DragKnife(s) => s.chains.iter().chain(start(&s.start)).collect(),
    }
}

/// Build the used-by index: a reference never silently disappears into a
/// planner — the referencing operation is always discoverable.
pub fn used_by_index(job: &CamJobV5) -> UsedByIndex {
    let mut items: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut geometries: BTreeMap<(String, String, GeometryRefKind), Vec<String>> = BTreeMap::new();
    for operation in &job.operations {
        for reference in geometry_refs(&operation.settings) {
            items
                .entry(reference.artwork_item_id.0.clone())
                .or_default()
                .push(operation.id.clone());
            geometries
                .entry((
                    reference.artwork_item_id.0.clone(),
                    reference.local_geometry_id.clone(),
                    reference.kind,
                ))
                .or_default()
                .push(operation.id.clone());
        }
    }
    let assignments = super::resources::assignment_statuses(job);
    let mut tools: BTreeMap<String, Vec<AssignmentUse>> = BTreeMap::new();
    for status in &assignments {
        tools
            .entry(status.tool_id.clone())
            .or_default()
            .push(AssignmentUse {
                operation_id: status.operation_id.clone(),
                role: status.role,
            });
    }
    for tool in &job.tools {
        tools.entry(tool.id.clone()).or_default();
    }
    UsedByIndex {
        artwork_items: items
            .into_iter()
            .map(|(item_id, mut operation_ids)| {
                operation_ids.dedup();
                UsedByArtworkItem {
                    item_id,
                    operation_ids,
                }
            })
            .collect(),
        geometries: geometries
            .into_iter()
            .map(|((item_id, local_geometry_id, kind), mut operation_ids)| {
                operation_ids.dedup();
                UsedByGeometry {
                    item_id,
                    local_geometry_id,
                    kind,
                    operation_ids,
                }
            })
            .collect(),
        job_tools: tools
            .into_iter()
            .map(|(tool_id, assignments)| UsedByJobTool {
                exists: job.tools.iter().any(|tool| tool.id == tool_id),
                tool_id,
                assignments,
            })
            .collect(),
    }
}

/// Flat machine readout with per-row document annotations.
pub fn machine_readout(job: &CamJobV5) -> MachineReadout {
    let assignments = super::resources::assignment_statuses(job);
    let uses_of = |tool_id: &str| -> Vec<AssignmentUse> {
        assignments
            .iter()
            .filter(|status| status.tool_id == tool_id)
            .map(|status| AssignmentUse {
                operation_id: status.operation_id.clone(),
                role: status.role,
            })
            .collect()
    };
    match &job.machine_configuration {
        None => MachineReadout::default(),
        Some(configuration) => MachineReadout {
            origin: Some(configuration.origin.clone()),
            work_offset: configuration.work_offset.clone(),
            clearance_z_mm: configuration.clearance_z_mm,
            decimal_places: configuration.decimal_places,
            program_start_position_mm: configuration.program_start_position_mm,
            length_compensation: configuration.length_compensation,
            path_control: configuration.path_control,
            spindle_spinup_seconds: configuration.spindle_spinup_seconds,
            coolant: configuration.coolant,
            m6: configuration.m6.clone(),
            rows: configuration
                .tools
                .iter()
                .map(|row| MappingRowReadout {
                    job_tool_id: row.job_tool_id.clone(),
                    tool_number: row.tool_number,
                    length_offset_number: row.length_offset_number,
                    tool_exists: job.tools.iter().any(|tool| tool.id == row.job_tool_id),
                    assignments: uses_of(&row.job_tool_id),
                })
                .collect(),
        },
    }
}

/// The whole document inspection payload: machining order, used-by index,
/// assignment statuses and the machine readout.
pub fn inspect_document(job: &CamJobV5) -> DocumentInspection {
    DocumentInspection {
        machining_order: job
            .operations
            .iter()
            .map(|operation| OperationSummary {
                id: operation.id.clone(),
                name: operation.name.clone(),
                kind: operation_kind(&operation.settings).to_string(),
                enabled: operation.enabled,
            })
            .collect(),
        used_by: used_by_index(job),
        assignments: super::resources::assignment_statuses(job),
        machine: machine_readout(job),
    }
}

// ---------------------------------------------------------------------------
// Plan inspection (generated evidence, not candidate data)
// ---------------------------------------------------------------------------

/// Resolved operation heights in setup coordinates (plan section 6.3),
/// derived from the document values against the faces this plan actually
/// published. Flat V-carve resolves only its top.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedHeightsDto {
    pub top_z_mm: f64,
    pub bottom_z_mm: Option<f64>,
}

/// A face plane this plan published: the established Z and covered rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceReadout {
    pub z_mm: f64,
    pub covered: RectXY,
}

/// Per-operation generated evidence: status, resolved heights, published
/// face, generated tab placements and the stock-prefix identities (plan
/// section 22.8). Tab placements and face coverage come from the plan's
/// named outputs — they are what will be cut, not candidates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationInspection {
    pub operation_id: String,
    pub generation_status: GenerationStatus,
    pub heights: Option<ResolvedHeightsDto>,
    pub face: Option<FaceReadout>,
    pub tab_placements: Vec<crate::sequence::TabPlacementOutput>,
    pub stage_ids: Vec<String>,
    pub stock_before_id: String,
    pub stock_after_id: String,
    /// Machine time of this operation's motions, in seconds. Each move is
    /// timed by its own programmed feed (an arc by its swing, a plunge by its
    /// true path) and rapids by the machine's rapid rate.
    #[serde(default)]
    pub estimated_seconds: f64,
}

/// One ordered stage of the timeline with its motion span.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageInspection {
    pub stage_id: String,
    pub operation_id: String,
    pub tool_id: String,
    pub role: StageRole,
    pub motion_range: (usize, usize),
    pub motion_count: usize,
    /// Machine time of this stage's motions, in seconds, with the same
    /// per-move rule the operation readout uses.
    #[serde(default)]
    pub estimated_seconds: f64,
}

/// Stock extents and the plan's initial stock identity — present for
/// knife-only jobs too; no milling tool is invented to display stock.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockReadout {
    pub thickness_mm: Option<f64>,
    pub xy: Option<RectXY>,
    /// The stock identity the first operation was planned against; equal to
    /// every prefix's starting point.
    pub initial_stock_id: Option<String>,
}

/// Plan-level inspection over a generated [`OperationPlanV5`] (plan section
/// 22.9): ordered timeline, resolved heights and generated evidence, bound
/// to the plan's identities so readouts can label currentness.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanInspection {
    pub machining_identity: String,
    pub execution_fingerprint: String,
    pub operations: Vec<OperationInspection>,
    pub stages: Vec<StageInspection>,
    pub stock: StockReadout,
    /// The rapid rate the estimates timed `Rapid` moves with: the machine
    /// configuration's own rate, or the same assumption the playback
    /// transport rests on when the job states none.
    #[serde(default = "assumed_rapid_rate_mm_min")]
    pub rapid_rate_mm_min: f64,
}

/// The rapid rate used for timing when the job's machine configuration
/// states none. The playback transport assumes the same value, so the
/// inspection's times and the transport's program time agree.
pub const ASSUMED_RAPID_RATE_MM_MIN: f64 = 5000.;

fn assumed_rapid_rate_mm_min() -> f64 {
    ASSUMED_RAPID_RATE_MM_MIN
}

/// Machine time of one move: the true 3D path length — a plunge or ramp is
/// timed by its real path, an arc by the arc it is rather than its chord —
/// divided by the move's own rate. A feed without a usable rate contributes
/// nothing instead of failing the readout; the plan checks own that failure.
fn motion_seconds(motion: &crate::toolpath::PlannedMotion, rapid_rate_mm_min: f64) -> f64 {
    let dx = motion.end.x - motion.start.x;
    let dy = motion.end.y - motion.start.y;
    let dz = motion.end.z - motion.start.z;
    let xy = match motion.interpolation {
        crate::toolpath::Interpolation::ArcFeed(arc) => arc
            .length(motion.start.xy(), motion.end.xy())
            .unwrap_or_else(|| dx.hypot(dy)),
        _ => dx.hypot(dy),
    };
    let rate = match motion.interpolation {
        crate::toolpath::Interpolation::Rapid => rapid_rate_mm_min,
        crate::toolpath::Interpolation::LinearFeed | crate::toolpath::Interpolation::ArcFeed(_) => {
            match motion.feed_mm_min {
                Some(feed) if feed.is_finite() && feed > 0. => feed,
                _ => return 0.,
            }
        }
    };
    if !rate.is_finite() || rate <= 0. {
        return 0.;
    }
    (xy * xy + dz * dz).sqrt() / rate * 60.
}

impl From<&ExecutionStage> for StageInspection {
    fn from(stage: &ExecutionStage) -> Self {
        Self {
            stage_id: stage.stage_id.clone(),
            operation_id: stage.operation_id.clone(),
            tool_id: stage.tool_id.clone(),
            role: stage.role,
            motion_range: stage.motion_range,
            motion_count: stage.motion_range.1.saturating_sub(stage.motion_range.0),
            estimated_seconds: 0.,
        }
    }
}

/// Inspect a generated plan. Heights resolve against the face planes the
/// plan itself published, in execution order; an unresolvable height leaves
/// `heights` unset instead of failing the whole readout (the planning
/// diagnostics already carry the located failure).
pub fn inspect_plan(plan: &OperationPlanV5) -> Result<PlanInspection> {
    let job = &plan.job_snapshot;
    let rapid_rate_mm_min = job
        .machine_configuration
        .as_ref()
        .and_then(|configuration| configuration.rapid_rate_mm_min)
        .filter(|rate| rate.is_finite() && *rate > 0.)
        .unwrap_or(ASSUMED_RAPID_RATE_MM_MIN);
    // The machine time of every stage, in plan order; an operation's own time
    // is the sum of the stages it ran.
    let stage_seconds: Vec<f64> = plan
        .stages
        .iter()
        .map(|stage| {
            plan.motions[stage.motion_range.0..stage.motion_range.1]
                .iter()
                .map(|motion| motion_seconds(motion, rapid_rate_mm_min))
                .sum()
        })
        .collect();
    let mut stages = vec![];
    for (stage, seconds) in plan.stages.iter().zip(stage_seconds.iter()) {
        let mut inspection = StageInspection::from(stage);
        inspection.estimated_seconds = *seconds;
        stages.push(inspection);
    }
    let mut published: BTreeMap<String, f64> = BTreeMap::new();
    let mut operations = vec![];
    for result in &plan.operation_results {
        let settings = job
            .operations
            .iter()
            .find(|operation| operation.id == result.operation_id)
            .map(|operation| &operation.settings);
        let heights =
            settings.and_then(|settings| resolve_settings_heights(job, settings, &published));
        for output in &result.named_outputs {
            if output.kind == "face_plane"
                && let Some(z_mm) = output.z_mm
            {
                published.insert(result.operation_id.clone(), z_mm);
            }
        }
        operations.push(OperationInspection {
            operation_id: result.operation_id.clone(),
            generation_status: result.generation_status,
            heights,
            face: result
                .named_outputs
                .iter()
                .find_map(|output| {
                    if output.kind == "face_plane" {
                        Some((output.z_mm?, output.covered?))
                    } else {
                        None
                    }
                })
                .map(|(z_mm, covered)| FaceReadout { z_mm, covered }),
            tab_placements: result
                .named_outputs
                .iter()
                .flat_map(|output| output.tab_placements.iter().cloned())
                .collect(),
            stage_ids: result.stage_ids.clone(),
            stock_before_id: result.stock_before_id.clone(),
            stock_after_id: result.stock_after_id.clone(),
            estimated_seconds: result
                .stage_ids
                .iter()
                .filter_map(|stage_id| {
                    plan.stages
                        .iter()
                        .position(|stage| &stage.stage_id == stage_id)
                        .and_then(|index| stage_seconds.get(index).copied())
                })
                .sum(),
        });
    }
    Ok(PlanInspection {
        machining_identity: plan.machining_identity.clone(),
        execution_fingerprint: plan.execution_fingerprint.clone(),
        operations,
        stages,
        stock: StockReadout {
            thickness_mm: job.setup.stock.thickness_mm,
            xy: job.setup.stock.xy,
            initial_stock_id: plan
                .operation_results
                .first()
                .map(|result| result.stock_before_id.clone()),
        },
        rapid_rate_mm_min,
    })
}

/// Resolve one operation's heights against the planes published so far.
/// Returns None when the values do not resolve (read-only inspection keeps
/// the failure to the diagnostics that own it).
fn resolve_settings_heights(
    job: &CamJobV5,
    settings: &OperationSettingsV5,
    published: &BTreeMap<String, f64>,
) -> Option<ResolvedHeightsDto> {
    let thickness = job.setup.stock.thickness_mm;
    match settings {
        OperationSettingsV5::FlatVcarve(s) => Some(ResolvedHeightsDto {
            top_z_mm: crate::setup::resolve_top_values(thickness, &s.top, published).ok()?,
            bottom_z_mm: None,
        }),
        OperationSettingsV5::Face(s) => {
            crate::setup::resolve_heights_values(thickness, &s.top, &s.bottom, published)
                .ok()
                .map(|heights| ResolvedHeightsDto {
                    top_z_mm: heights.top_z,
                    bottom_z_mm: Some(heights.bottom_z),
                })
        }
        OperationSettingsV5::Profile(s) => {
            crate::setup::resolve_heights_values(thickness, &s.top, &s.bottom, published)
                .ok()
                .map(|heights| ResolvedHeightsDto {
                    top_z_mm: heights.top_z,
                    bottom_z_mm: Some(heights.bottom_z),
                })
        }
        OperationSettingsV5::DragKnife(s) => {
            crate::setup::resolve_heights_values(thickness, &s.top, &s.bottom, published)
                .ok()
                .map(|heights| ResolvedHeightsDto {
                    top_z_mm: heights.top_z,
                    bottom_z_mm: Some(heights.bottom_z),
                })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        geometry::Point,
        motion::Position,
        toolpath::{ArcMove, Interpolation, MotionEffect, MotionPurpose, PlannedMotion},
    };

    fn motion(
        interpolation: Interpolation,
        start: (f64, f64, f64),
        end: (f64, f64, f64),
        feed_mm_min: Option<f64>,
    ) -> PlannedMotion {
        let at = |(x, y, z): (f64, f64, f64)| Position { x, y, z };
        PlannedMotion {
            id: 0,
            operation_id: "op".into(),
            stage_id: "stage".into(),
            tool_id: "tool".into(),
            contour_id: None,
            pass_id: 0,
            layer: 0,
            interpolation,
            purpose: MotionPurpose::Rough,
            effect: MotionEffect::None,
            start: at(start),
            end: at(end),
            feed_mm_min,
            blade_heading_deg: None,
        }
    }

    /// A move is timed by its true path — a plunge by its depth, an arc by
    /// its swing — at its own rate, and an unusable feed rate contributes
    /// nothing rather than inventing time.
    #[test]
    fn machine_time_follows_the_real_path_and_rate() {
        // 8 mm of feed at 480 mm/min is one second, XY or Z alike.
        let cut = motion(
            Interpolation::LinearFeed,
            (0., 0., 0.),
            (8., 0., 0.),
            Some(480.),
        );
        assert_eq!(motion_seconds(&cut, ASSUMED_RAPID_RATE_MM_MIN), 1.);
        let plunge = motion(
            Interpolation::LinearFeed,
            (0., 0., 0.),
            (0., 0., -3.),
            Some(60.),
        );
        assert_eq!(motion_seconds(&plunge, ASSUMED_RAPID_RATE_MM_MIN), 3.);
        // A ramp is timed by its 3D path, not its XY projection.
        let ramp = motion(
            Interpolation::LinearFeed,
            (0., 0., 0.),
            (3., 4., -12.),
            Some(60.),
        );
        assert_eq!(motion_seconds(&ramp, ASSUMED_RAPID_RATE_MM_MIN), 13.);
        // A quarter circle of radius 4 swings 2π mm, not its 4√2 chord.
        let arc = motion(
            Interpolation::ArcFeed(ArcMove {
                center: Point::new(0., 0.),
                clockwise: false,
            }),
            (4., 0., 0.),
            (0., 4., 0.),
            Some(480.),
        );
        assert!(
            (motion_seconds(&arc, ASSUMED_RAPID_RATE_MM_MIN) - std::f64::consts::TAU / 8.).abs()
                < 1e-12
        );
        // Rapids run at the machine's rate, and a move without a usable feed
        // takes no time in the readout.
        let rapid = motion(Interpolation::Rapid, (0., 0., 0.), (100., 0., 0.), None);
        assert_eq!(motion_seconds(&rapid, 5000.), 1.2);
        assert_eq!(motion_seconds(&rapid, 3000.), 2.);
        let unpriced = motion(Interpolation::LinearFeed, (0., 0., 0.), (8., 0., 0.), None);
        assert_eq!(motion_seconds(&unpriced, ASSUMED_RAPID_RATE_MM_MIN), 0.);
    }
}
