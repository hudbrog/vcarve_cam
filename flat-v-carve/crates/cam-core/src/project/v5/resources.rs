//! Applied cutting-profile and tool-assignment commands (plan section 22.6,
//! H4): copy preset values into exactly one assignment, store the copied
//! baseline that Reset and Modified are calculated against, and reapply from
//! an explicitly supplied current library revision.
//!
//! Library documents are inputs to Apply/Reapply only: after application the
//! job carries independent copied values and provenance, so removing the
//! library or editing it later never invalidates or changes a saved job
//! (scenario 7 of plan section 22.12). Choosing a different tool for an
//! assignment without a preset clears the assignment's applicable cutting
//! fields and its previous baseline; re-applying the same tool keeps values
//! for revalidation.
use super::{
    CamJobV5, CuttingBaseline, JobToolV5, KnifeAssignmentV5, LibraryOrigin, MillingAssignmentV5,
    OperationSettingsV5,
};
use crate::{
    geometry::{Diagnostic, Result},
    project::ToolCapabilities,
    tool_library::{LibraryGeometry, ToolLibrary},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Which assignment of an operation a resource command addresses. The
/// identity is `(operation id, role)` — the same job tool may back several
/// assignments with independent baselines (plan section 22.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentRole {
    /// Flat V-carve rough-stage assignment.
    Endmill,
    /// Flat V-carve finish-stage assignment (kept even in endmill-only mode).
    Vbit,
    /// The single milling assignment of Face/Profile operations.
    Milling,
    /// The knife assignment of a Drag Knife operation.
    Knife,
}

impl AssignmentRole {
    fn name(self) -> &'static str {
        match self {
            Self::Endmill => "endmill",
            Self::Vbit => "vbit",
            Self::Milling => "milling",
            Self::Knife => "knife",
        }
    }
}

/// Mutable access to one assignment addressed by role.
enum AssignmentRef<'a> {
    Milling(&'a mut MillingAssignmentV5),
    Knife(&'a mut KnifeAssignmentV5),
}

fn assignment_mut<'a>(
    settings: &'a mut OperationSettingsV5,
    operation_id: &str,
    role: AssignmentRole,
) -> Result<AssignmentRef<'a>> {
    let mismatch = |kind: &str| {
        resource_error(format!(
            "operation '{operation_id}' is a {kind} operation and has no {} assignment",
            role.name()
        ))
    };
    Ok(match settings {
        OperationSettingsV5::FlatVcarve(settings) => match role {
            AssignmentRole::Endmill => AssignmentRef::Milling(&mut settings.endmill),
            AssignmentRole::Vbit => AssignmentRef::Milling(&mut settings.vbit),
            _ => return Err(mismatch("flat_vcarve")),
        },
        OperationSettingsV5::Face(settings) => match role {
            AssignmentRole::Milling => AssignmentRef::Milling(&mut settings.assignment),
            _ => return Err(mismatch("face")),
        },
        OperationSettingsV5::Profile(settings) => match role {
            AssignmentRole::Milling => AssignmentRef::Milling(&mut settings.assignment),
            _ => return Err(mismatch("profile")),
        },
        OperationSettingsV5::DragKnife(settings) => match role {
            AssignmentRole::Knife => AssignmentRef::Knife(&mut settings.assignment),
            _ => return Err(mismatch("drag_knife")),
        },
    })
}

fn resource_error(message: impl Into<String>) -> Diagnostic {
    Diagnostic::new("RESOURCE_COMMAND", message).at_stage("resources")
}

/// Whether an assignment's effective cutting values match its stored
/// baseline (plan section 22.6): `Custom` without provenance, `Applied`
/// while the copied values stand unmodified, `Modified` once any applicable
/// field was edited. Fields outside the preset surface (spindle direction,
/// tool choice) do not participate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileStatus {
    Custom,
    Applied,
    Modified,
}

fn milling_status(assignment: &MillingAssignmentV5) -> ProfileStatus {
    let Some(applied) = &assignment.applied_profile else {
        return ProfileStatus::Custom;
    };
    let CuttingBaseline::Milling {
        spindle_rpm,
        cutting_feed_mm_min,
        plunge_feed_mm_min,
        max_stepdown_mm,
        stepover_mm,
    } = &applied.baseline
    else {
        return ProfileStatus::Modified;
    };
    let matches = assignment.spindle_rpm == *spindle_rpm
        && assignment.cutting_feed_mm_min == *cutting_feed_mm_min
        && assignment.plunge_feed_mm_min == *plunge_feed_mm_min
        && assignment.max_stepdown_mm == *max_stepdown_mm
        && assignment.stepover_mm == *stepover_mm;
    if matches {
        ProfileStatus::Applied
    } else {
        ProfileStatus::Modified
    }
}

fn knife_status(assignment: &KnifeAssignmentV5) -> ProfileStatus {
    let Some(applied) = &assignment.applied_profile else {
        return ProfileStatus::Custom;
    };
    let CuttingBaseline::Knife {
        cutting_feed_mm_min,
        plunge_feed_mm_min,
        swivel_feed_mm_min,
        max_stepdown_mm,
    } = &applied.baseline
    else {
        return ProfileStatus::Modified;
    };
    let matches = assignment.cutting_feed_mm_min == *cutting_feed_mm_min
        && assignment.plunge_feed_mm_min == *plunge_feed_mm_min
        && assignment.swivel_feed_mm_min == *swivel_feed_mm_min
        && assignment.max_stepdown_mm == *max_stepdown_mm;
    if matches {
        ProfileStatus::Applied
    } else {
        ProfileStatus::Modified
    }
}

/// Copy a preset's complete applicable field set into a milling assignment,
/// including unset values: a partial preset copies its nulls as well, and
/// spindle direction (not a preset field) is never touched.
fn copy_milling_preset(
    assignment: &mut MillingAssignmentV5,
    preset: &crate::tool_library::CuttingPreset,
) {
    assignment.spindle_rpm = preset.spindle_rpm;
    assignment.cutting_feed_mm_min = preset.cutting_feed_mm_min;
    assignment.plunge_feed_mm_min = preset.plunge_feed_mm_min;
    assignment.max_stepdown_mm = preset.max_stepdown_mm;
    assignment.stepover_mm = preset.stepover_mm;
}

fn copy_knife_preset(
    assignment: &mut KnifeAssignmentV5,
    preset: &crate::tool_library::KnifeCuttingPreset,
) {
    assignment.cutting_feed_mm_min = preset.cutting_feed_mm_min;
    assignment.plunge_feed_mm_min = preset.plunge_feed_mm_min;
    assignment.swivel_feed_mm_min = preset.swivel_feed_mm_min;
    assignment.max_stepdown_mm = preset.max_stepdown_mm;
}

fn clear_milling(assignment: &mut MillingAssignmentV5) {
    assignment.spindle_rpm = None;
    assignment.spindle_direction = None;
    assignment.cutting_feed_mm_min = None;
    assignment.plunge_feed_mm_min = None;
    assignment.max_stepdown_mm = None;
    assignment.stepover_mm = None;
    assignment.applied_profile = None;
}

fn clear_knife(assignment: &mut KnifeAssignmentV5) {
    assignment.cutting_feed_mm_min = None;
    assignment.plunge_feed_mm_min = None;
    assignment.swivel_feed_mm_min = None;
    assignment.max_stepdown_mm = None;
    assignment.applied_profile = None;
}

fn operation_index(job: &CamJobV5, operation_id: &str) -> Result<usize> {
    job.operations
        .iter()
        .position(|op| op.id == operation_id)
        .ok_or_else(|| resource_error(format!("unknown operation '{operation_id}'")))
}

/// Apply one named cutting profile to exactly one assignment: every
/// applicable preset field is copied (unset ones included) and stored as the
/// baseline Reset and Modified are calculated against. Sibling assignments —
/// including other assignments of the same job tool — are never touched, and
/// the job keeps no live dependency on the library document.
pub fn apply_cutting_profile(
    job: &CamJobV5,
    operation_id: &str,
    role: AssignmentRole,
    library: &ToolLibrary,
    library_id: &str,
    library_tool_id: &str,
    preset_id: &str,
) -> Result<super::commands::CommandOutcome> {
    let index = operation_index(job, operation_id)?;
    let mut candidate = job.clone();
    let tool = library.tool(library_tool_id)?;
    match assignment_mut(
        &mut candidate.operations[index].settings,
        operation_id,
        role,
    )? {
        AssignmentRef::Milling(assignment) => {
            let preset = tool.preset(preset_id)?;
            copy_milling_preset(assignment, preset);
            assignment.applied_profile = Some(super::AppliedProfile {
                library_id: library_id.to_string(),
                library_tool_id: library_tool_id.to_string(),
                preset_id: preset_id.to_string(),
                revision_at_application: library.revision,
                name_at_application: preset.name.clone(),
                baseline: CuttingBaseline::Milling {
                    spindle_rpm: preset.spindle_rpm,
                    cutting_feed_mm_min: preset.cutting_feed_mm_min,
                    plunge_feed_mm_min: preset.plunge_feed_mm_min,
                    max_stepdown_mm: preset.max_stepdown_mm,
                    stepover_mm: preset.stepover_mm,
                },
            });
        }
        AssignmentRef::Knife(assignment) => {
            let preset = tool.knife_preset(preset_id)?;
            copy_knife_preset(assignment, preset);
            assignment.applied_profile = Some(super::AppliedProfile {
                library_id: library_id.to_string(),
                library_tool_id: library_tool_id.to_string(),
                preset_id: preset_id.to_string(),
                revision_at_application: library.revision,
                name_at_application: preset.name.clone(),
                baseline: CuttingBaseline::Knife {
                    cutting_feed_mm_min: preset.cutting_feed_mm_min,
                    plunge_feed_mm_min: preset.plunge_feed_mm_min,
                    swivel_feed_mm_min: preset.swivel_feed_mm_min,
                    max_stepdown_mm: preset.max_stepdown_mm,
                },
            });
        }
    }
    super::commands::CommandOutcome::commit(
        candidate,
        vec![super::commands::AffectedEntity::Operation(
            operation_id.into(),
        )],
    )
}

/// Restore one assignment's cutting values to its stored baseline without
/// opening any library (plan section 22.6). A custom assignment with no
/// applied profile has nothing to reset to and is a located error.
pub fn reset_assignment(
    job: &CamJobV5,
    operation_id: &str,
    role: AssignmentRole,
) -> Result<super::commands::CommandOutcome> {
    let index = operation_index(job, operation_id)?;
    let mut candidate = job.clone();
    match assignment_mut(
        &mut candidate.operations[index].settings,
        operation_id,
        role,
    )? {
        AssignmentRef::Milling(assignment) => {
            let applied = assignment.applied_profile.as_ref().ok_or_else(|| {
                resource_error(format!(
                    "operation '{operation_id}' ({}) has no applied profile to reset",
                    role.name()
                ))
            })?;
            let CuttingBaseline::Milling {
                spindle_rpm,
                cutting_feed_mm_min,
                plunge_feed_mm_min,
                max_stepdown_mm,
                stepover_mm,
            } = &applied.baseline
            else {
                return Err(resource_error(
                    "the stored baseline does not match the assignment kind",
                ));
            };
            assignment.spindle_rpm = *spindle_rpm;
            assignment.cutting_feed_mm_min = *cutting_feed_mm_min;
            assignment.plunge_feed_mm_min = *plunge_feed_mm_min;
            assignment.max_stepdown_mm = *max_stepdown_mm;
            assignment.stepover_mm = *stepover_mm;
        }
        AssignmentRef::Knife(assignment) => {
            let applied = assignment.applied_profile.as_ref().ok_or_else(|| {
                resource_error(format!(
                    "operation '{operation_id}' ({}) has no applied profile to reset",
                    role.name()
                ))
            })?;
            let CuttingBaseline::Knife {
                cutting_feed_mm_min,
                plunge_feed_mm_min,
                swivel_feed_mm_min,
                max_stepdown_mm,
            } = &applied.baseline
            else {
                return Err(resource_error(
                    "the stored baseline does not match the assignment kind",
                ));
            };
            assignment.cutting_feed_mm_min = *cutting_feed_mm_min;
            assignment.plunge_feed_mm_min = *plunge_feed_mm_min;
            assignment.swivel_feed_mm_min = *swivel_feed_mm_min;
            assignment.max_stepdown_mm = *max_stepdown_mm;
        }
    }
    super::commands::CommandOutcome::commit(
        candidate,
        vec![super::commands::AffectedEntity::Operation(
            operation_id.into(),
        )],
    )
}

/// Reapply the stored profile provenance from an explicitly supplied current
/// library revision: the preset's current values are copied and become the
/// new baseline. Reapplying needs the library record; a removed record is a
/// located error, never a silent no-op.
pub fn reapply_profile(
    job: &CamJobV5,
    operation_id: &str,
    role: AssignmentRole,
    library: &ToolLibrary,
    library_id: &str,
) -> Result<super::commands::CommandOutcome> {
    let index = operation_index(job, operation_id)?;
    let mut candidate = job.clone();
    let (library_tool_id, preset_id) = {
        let settings = &candidate.operations[index].settings;
        match assignment_ref_of(settings, role)? {
            AssignmentRead::Milling(assignment) => assignment
                .applied_profile
                .as_ref()
                .map(|p| (p.library_tool_id.clone(), p.preset_id.clone())),
            AssignmentRead::Knife(assignment) => assignment
                .applied_profile
                .as_ref()
                .map(|p| (p.library_tool_id.clone(), p.preset_id.clone())),
        }
    }
    .ok_or_else(|| {
        resource_error(format!(
            "operation '{operation_id}' ({}) has no applied profile to reapply",
            role.name()
        ))
    })?;
    let tool = library.tool(&library_tool_id).map_err(|e| {
        resource_error(format!(
            "reapply needs library tool '{library_tool_id}': {}",
            e.message
        ))
    })?;
    match assignment_mut(
        &mut candidate.operations[index].settings,
        operation_id,
        role,
    )? {
        AssignmentRef::Milling(assignment) => {
            let preset = tool.preset(&preset_id).map_err(|e| {
                resource_error(format!("reapply needs the library record: {}", e.message))
            })?;
            copy_milling_preset(assignment, preset);
            assignment.applied_profile = Some(super::AppliedProfile {
                library_id: library_id.to_string(),
                library_tool_id,
                preset_id,
                revision_at_application: library.revision,
                name_at_application: preset.name.clone(),
                baseline: CuttingBaseline::Milling {
                    spindle_rpm: preset.spindle_rpm,
                    cutting_feed_mm_min: preset.cutting_feed_mm_min,
                    plunge_feed_mm_min: preset.plunge_feed_mm_min,
                    max_stepdown_mm: preset.max_stepdown_mm,
                    stepover_mm: preset.stepover_mm,
                },
            });
        }
        AssignmentRef::Knife(assignment) => {
            let preset = tool.knife_preset(&preset_id).map_err(|e| {
                resource_error(format!("reapply needs the library record: {}", e.message))
            })?;
            copy_knife_preset(assignment, preset);
            assignment.applied_profile = Some(super::AppliedProfile {
                library_id: library_id.to_string(),
                library_tool_id,
                preset_id,
                revision_at_application: library.revision,
                name_at_application: preset.name.clone(),
                baseline: CuttingBaseline::Knife {
                    cutting_feed_mm_min: preset.cutting_feed_mm_min,
                    plunge_feed_mm_min: preset.plunge_feed_mm_min,
                    swivel_feed_mm_min: preset.swivel_feed_mm_min,
                    max_stepdown_mm: preset.max_stepdown_mm,
                },
            });
        }
    }
    super::commands::CommandOutcome::commit(
        candidate,
        vec![super::commands::AffectedEntity::Operation(
            operation_id.into(),
        )],
    )
}

enum AssignmentRead<'a> {
    Milling(&'a MillingAssignmentV5),
    Knife(&'a KnifeAssignmentV5),
}

fn assignment_ref_of<'a>(
    settings: &'a OperationSettingsV5,
    role: AssignmentRole,
) -> Result<AssignmentRead<'a>> {
    Ok(match settings {
        OperationSettingsV5::FlatVcarve(settings) => match role {
            AssignmentRole::Endmill => AssignmentRead::Milling(&settings.endmill),
            AssignmentRole::Vbit => AssignmentRead::Milling(&settings.vbit),
            _ => {
                return Err(resource_error(
                    "assignment role does not match the operation",
                ));
            }
        },
        OperationSettingsV5::Face(settings) => match role {
            AssignmentRole::Milling => AssignmentRead::Milling(&settings.assignment),
            _ => {
                return Err(resource_error(
                    "assignment role does not match the operation",
                ));
            }
        },
        OperationSettingsV5::Profile(settings) => match role {
            AssignmentRole::Milling => AssignmentRead::Milling(&settings.assignment),
            _ => {
                return Err(resource_error(
                    "assignment role does not match the operation",
                ));
            }
        },
        OperationSettingsV5::DragKnife(settings) => match role {
            AssignmentRole::Knife => AssignmentRead::Knife(&settings.assignment),
            _ => {
                return Err(resource_error(
                    "assignment role does not match the operation",
                ));
            }
        },
    })
}

/// Project library geometry into a job-tool geometry snapshot. The library's
/// legacy endmill spelling carries `plunge_capable` in the spec; the job
/// stores capabilities separately exactly once.
fn job_geometry(geometry: &LibraryGeometry) -> crate::project::ToolGeometry {
    match geometry {
        LibraryGeometry::Endmill(spec) => {
            crate::project::ToolGeometry::Endmill(crate::project::EndmillGeometry {
                diameter_mm: spec.diameter_mm,
                cutting_length_mm: spec.cutting_length_mm,
            })
        }
        LibraryGeometry::Vbit(spec) => crate::project::ToolGeometry::Vbit(spec.clone()),
        LibraryGeometry::DragKnife(spec) => crate::project::ToolGeometry::DragKnife(spec.clone()),
    }
}

fn library_capabilities(tool: &crate::tool_library::LibraryTool) -> ToolCapabilities {
    match &tool.geometry {
        // A knife never carries milling entry capabilities (plan section 5.3).
        LibraryGeometry::DragKnife(_) => ToolCapabilities::default(),
        LibraryGeometry::Endmill(spec) => ToolCapabilities {
            plunge_capable: tool.plunge_capable.or(Some(spec.plunge_capable)),
            ramp_capable: tool.ramp_capable,
        },
        LibraryGeometry::Vbit(_) => ToolCapabilities {
            plunge_capable: tool.plunge_capable,
            ramp_capable: tool.ramp_capable,
        },
    }
}

/// Apply a library tool's geometry to one assignment ("Use in operation"):
/// the job gains or refreshes an independent tool snapshot with copied
/// provenance, and the assignment binds it. Choosing a *different* tool
/// without a cutting preset clears the assignment's applicable cutting
/// fields and its previous baseline; unchanged geometry with the same origin
/// reuses its snapshot and keeps values. Changed library geometry creates a
/// new snapshot; editing shared job geometry is a separate explicit action.
/// Other assignments using the shared snapshot are not rewritten.
pub fn apply_tool_to_assignment(
    job: &CamJobV5,
    operation_id: &str,
    role: AssignmentRole,
    library: &ToolLibrary,
    library_id: &str,
    library_tool_id: &str,
) -> Result<super::commands::CommandOutcome> {
    let added = add_library_tool(job, library, library_id, library_tool_id)?;
    let tool_id = added.1;
    use_job_tool(&added.0.job, operation_id, role, &tool_id)
}

/// Copy one library tool's spindle rotation onto exactly one assignment,
/// addressed by `(operation id, role)`. "Use in operation" uses this so a
/// Flat V-carve rough/finish assignment and a Face/Profile operation's single
/// milling assignment all take the library tool's rotation. Spindle direction
/// sits outside the preset surface, so the assignment's cutting values, its
/// stored baseline and its Applied/Modified status are untouched.
pub fn set_assignment_spindle_direction(
    job: &CamJobV5,
    operation_id: &str,
    role: AssignmentRole,
    direction: Option<crate::project::SpindleDirection>,
) -> Result<super::commands::CommandOutcome> {
    let index = operation_index(job, operation_id)?;
    let mut candidate = job.clone();
    match assignment_mut(
        &mut candidate.operations[index].settings,
        operation_id,
        role,
    )? {
        AssignmentRef::Milling(assignment) => assignment.spindle_direction = direction,
        AssignmentRef::Knife(_) => {
            return Err(resource_error(format!(
                "operation '{operation_id}' is a drag_knife operation and has no spindle to set"
            )));
        }
    }
    super::commands::CommandOutcome::commit(
        candidate,
        vec![super::commands::AffectedEntity::Operation(
            operation_id.into(),
        )],
    )
}

/// Add independent geometry without changing an assignment. Only an exact
/// unchanged snapshot with the same explicit origin can be reused. Colliding
/// local IDs and equal dimensions never identify the same physical tool.
pub fn add_library_tool(
    job: &CamJobV5,
    library: &ToolLibrary,
    library_id: &str,
    library_tool_id: &str,
) -> Result<(super::commands::CommandOutcome, String)> {
    library.validate()?;
    let tool = library.tool(library_tool_id)?;
    let geometry = Some(job_geometry(&tool.geometry));
    let capabilities = library_capabilities(tool);
    if let Some(existing) = job.tools.iter().find(|t| {
        t.library_origin
            .as_ref()
            .is_some_and(|o| o.library_id == library_id && o.tool_id == library_tool_id)
            && t.geometry == geometry
            && t.capabilities == capabilities
    }) {
        return Ok((
            super::commands::CommandOutcome::commit(job.clone(), vec![])?,
            existing.id.clone(),
        ));
    }
    let mut id = tool.id.clone();
    let mut suffix = 2;
    let referenced = assigned_tool_ids(job);
    while job.tools.iter().any(|t| t.id == id)
        || referenced.contains(&id)
        || job
            .machine_configuration
            .as_ref()
            .is_some_and(|m| m.tools.iter().any(|row| row.job_tool_id == id))
    {
        id = format!("{}-{suffix}", tool.id.chars().take(85).collect::<String>());
        suffix += 1;
    }
    let mut candidate = job.clone();
    candidate.tools.push(JobToolV5 {
        id: id.clone(),
        name: tool.name.clone(),
        geometry,
        capabilities,
        library_origin: Some(LibraryOrigin {
            library_id: library_id.into(),
            tool_id: tool.id.clone(),
            copied_revision: library.revision,
            name_at_copy: tool.name.clone(),
        }),
    });
    Ok((
        super::commands::CommandOutcome::commit(
            candidate,
            vec![super::commands::AffectedEntity::JobTool(id.clone())],
        )?,
        id,
    ))
}

/// Bind a selected copied job tool; changing physical tools clears only this
/// assignment's cutting values and baseline. Existing geometry stays shared.
pub fn use_job_tool(
    job: &CamJobV5,
    operation_id: &str,
    role: AssignmentRole,
    tool_id: &str,
) -> Result<super::commands::CommandOutcome> {
    let index = operation_index(job, operation_id)?;
    let tool = job
        .tools
        .iter()
        .find(|t| t.id == tool_id)
        .ok_or_else(|| resource_error(format!("unknown job tool '{tool_id}'")))?;
    // The role decides which geometry kinds are acceptable: V-bit geometry
    // belongs to a V-bit assignment, a knife to a knife assignment.
    let accepts = |geometry: &crate::project::ToolGeometry| match role {
        AssignmentRole::Endmill => matches!(geometry, crate::project::ToolGeometry::Endmill(_)),
        AssignmentRole::Vbit => matches!(geometry, crate::project::ToolGeometry::Vbit(_)),
        AssignmentRole::Milling => matches!(
            geometry,
            crate::project::ToolGeometry::Endmill(_) | crate::project::ToolGeometry::Vbit(_)
        ),
        AssignmentRole::Knife => matches!(geometry, crate::project::ToolGeometry::DragKnife(_)),
    };
    if !tool.geometry.as_ref().is_some_and(accepts) {
        return Err(resource_error(format!(
            "job tool '{tool_id}' does not fit the {} assignment of operation '{operation_id}'",
            role.name()
        )));
    }
    let mut candidate = job.clone();
    match assignment_mut(
        &mut candidate.operations[index].settings,
        operation_id,
        role,
    )? {
        AssignmentRef::Milling(assignment) => {
            if assignment.tool_id != tool.id {
                clear_milling(assignment);
                assignment.tool_id = tool.id.clone();
            }
        }
        AssignmentRef::Knife(assignment) => {
            if assignment.tool_id != tool.id {
                clear_knife(assignment);
                assignment.tool_id = tool.id.clone();
            }
        }
    }
    super::commands::CommandOutcome::commit(
        candidate,
        vec![
            super::commands::AffectedEntity::Operation(operation_id.into()),
            super::commands::AffectedEntity::JobTool(tool.id.clone()),
        ],
    )
}

/// The `(operation id, role)` list of every assignment in document order.
pub fn assignments_of(job: &CamJobV5) -> Vec<(String, AssignmentRole)> {
    let mut out = vec![];
    for operation in &job.operations {
        match &operation.settings {
            OperationSettingsV5::FlatVcarve(_) => {
                out.push((operation.id.clone(), AssignmentRole::Endmill));
                out.push((operation.id.clone(), AssignmentRole::Vbit));
            }
            OperationSettingsV5::Face(_) | OperationSettingsV5::Profile(_) => {
                out.push((operation.id.clone(), AssignmentRole::Milling));
            }
            OperationSettingsV5::DragKnife(_) => {
                out.push((operation.id.clone(), AssignmentRole::Knife));
            }
        }
    }
    out
}

/// Read-only status of one assignment for the inspection DTOs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentStatus {
    pub operation_id: String,
    pub role: AssignmentRole,
    pub tool_id: String,
    pub status: ProfileStatus,
    /// Copied provenance when a profile was applied.
    pub applied: Option<super::AppliedProfile>,
}

pub fn assignment_statuses(job: &CamJobV5) -> Vec<AssignmentStatus> {
    assignments_of(job)
        .into_iter()
        .filter_map(|(operation_id, role)| {
            let operation = job.operations.iter().find(|op| op.id == operation_id)?;
            let read = assignment_ref_of(&operation.settings, role).ok()?;
            let (tool_id, status, applied) = match read {
                AssignmentRead::Milling(assignment) => (
                    assignment.tool_id.clone(),
                    milling_status(assignment),
                    assignment.applied_profile.clone(),
                ),
                AssignmentRead::Knife(assignment) => (
                    assignment.tool_id.clone(),
                    knife_status(assignment),
                    assignment.applied_profile.clone(),
                ),
            };
            Some(AssignmentStatus {
                operation_id,
                role,
                tool_id,
                status,
                applied,
            })
        })
        .collect()
}

/// Job tool IDs referenced by at least one assignment.
pub fn assigned_tool_ids(job: &CamJobV5) -> BTreeSet<String> {
    assignment_statuses(job)
        .into_iter()
        .map(|status| status.tool_id)
        .collect()
}
