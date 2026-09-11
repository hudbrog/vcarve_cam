//! Applied machine configuration commands and the scope-mapping resolver
//! (plan section 22.7, H4).
//!
//! The job's single `AppliedMachineConfiguration` is the only object Setup →
//! Machine, mapping readouts and export address. Application copies a complete
//! validated schema-2 [`SequenceProfile`] (a reusable configuration file) into
//! an editable snapshot: rows for tools this job does not contain are dropped
//! instead of transplanted, and T numbers are never inferred from cutter
//! geometry — association stays explicit through [`set_tool_mapping`].
//!
//! The snapshot deliberately allows unset preparation fields so an incomplete
//! configuration never blocks saving; [`resolve_sequence_profile`] is the gate
//! that turns a complete snapshot into the validated profile export needs,
//! validating mappings only for the requested executable scope: unused job
//! tools require no T/H mappings, dangling rows stay repairable draft issues,
//! and conflicting active mappings block output.
use super::{
    AppliedMachineConfiguration, AppliedToolMapping, CamJobV5, ConfigurationOrigin,
    commands::{AffectedEntity, CommandOutcome},
    references::ReadinessScope,
};
use crate::{
    geometry::{Diagnostic, Result},
    post::{
        LengthCompensation,
        sequence::{SEQUENCE_PROFILE_SCHEMA_VERSION, SequenceProfile, SequenceToolMapping},
    },
    sequence::{scoped_enabled_operations_v5, tool_ids_of_v5},
};
use std::collections::{BTreeMap, BTreeSet};

fn machine_error(message: impl Into<String>) -> Diagnostic {
    Diagnostic::new("MACHINE_COMMAND", message).at_stage("machine")
}

/// Copy a reusable configuration file into the job's one applied machine
/// snapshot. Every output field is copied independently — the file is not
/// needed again and later changes to it never affect the job. Mapping rows
/// are copied only for job tools that exist here; a reusable file from
/// another job cannot transplant its opaque tool IDs into this one.
pub fn apply_machine_configuration(
    job: &CamJobV5,
    profile: &SequenceProfile,
    configuration_name: &str,
) -> Result<CommandOutcome> {
    profile.validate_shape()?;
    if let Some(clearance) = job.setup.clearance_above_stock_mm
        && clearance != profile.clearance_z_mm
    {
        return Err(Diagnostic::new(
            "MACHINE_CLEARANCE_CONFLICT",
            format!(
                "configuration clearance {} mm differs from the job setup clearance {clearance} mm",
                profile.clearance_z_mm
            ),
        )
        .at_stage("machine"));
    }
    let tools = profile
        .tools
        .iter()
        .filter(|mapping| job.tools.iter().any(|tool| tool.id == mapping.tool_id))
        .map(|mapping| AppliedToolMapping {
            job_tool_id: mapping.tool_id.clone(),
            tool_number: Some(mapping.tool_number),
            length_offset_number: mapping.length_offset_number,
        })
        .collect();
    let mut candidate = job.clone();
    candidate.machine_configuration = Some(AppliedMachineConfiguration {
        origin: ConfigurationOrigin {
            configuration_id: profile.id.clone(),
            name: configuration_name.to_string(),
        },
        work_offset: Some(profile.work_offset.clone()),
        clearance_z_mm: Some(profile.clearance_z_mm),
        decimal_places: Some(profile.decimal_places),
        program_start_position_mm: profile.program_start_position_mm,
        length_compensation: Some(profile.length_compensation),
        path_control: Some(profile.path_control),
        spindle_spinup_seconds: Some(profile.spindle_spinup_seconds),
        coolant: Some(profile.coolant),
        m6: Some(profile.m6.clone()),
        tools,
    });
    CommandOutcome::commit(candidate, vec![AffectedEntity::MachineConfiguration])
}

/// Set one job tool's controller mapping exactly: `Some`/`None` are absolute
/// values, and both numbers `None` removes the row. Uncertain fields stay
/// explicitly unresolved rather than being fabricated.
pub fn set_tool_mapping(
    job: &CamJobV5,
    job_tool_id: &str,
    tool_number: Option<u32>,
    length_offset_number: Option<u32>,
) -> Result<CommandOutcome> {
    if !job.tools.iter().any(|tool| tool.id == job_tool_id) {
        return Err(machine_error(format!("unknown job tool '{job_tool_id}'")));
    }
    if tool_number == Some(0) {
        return Err(machine_error(format!(
            "tool number for '{job_tool_id}' must be positive"
        )));
    }
    let mut candidate = job.clone();
    let configuration = candidate
        .machine_configuration
        .as_mut()
        .ok_or_else(|| machine_error("the job has no applied machine configuration"))?;
    let row = AppliedToolMapping {
        job_tool_id: job_tool_id.to_string(),
        tool_number,
        length_offset_number,
    };
    match configuration
        .tools
        .iter()
        .position(|mapping| mapping.job_tool_id == job_tool_id)
    {
        Some(_) if tool_number.is_none() && length_offset_number.is_none() => {
            configuration
                .tools
                .retain(|mapping| mapping.job_tool_id != job_tool_id);
        }
        Some(position) => configuration.tools[position] = row,
        None => configuration.tools.push(row),
    }
    CommandOutcome::commit(candidate, vec![AffectedEntity::MachineConfiguration])
}

/// Turn the applied machine snapshot into the complete validated schema-2
/// profile export needs, validating mappings only for the requested
/// executable scope (plan section 22.7):
///
/// * a missing configuration or missing preparation fields never fabricate
///   values — the missing field list is reported instead;
/// * a snapshot clearance disagreeing with the job's setup clearance is a
///   conflict, not a silent override;
/// * only tools used by enabled operations in the scope need T numbers (and
///   H numbers under tool-table length compensation);
/// * two active tools mapped to one T number block output;
/// * rows for tools outside the job, and incomplete rows of unused tools,
///   are dropped from the resolved profile as repairable draft state.
pub fn resolve_sequence_profile(job: &CamJobV5, scope: &ReadinessScope) -> Result<SequenceProfile> {
    let configuration = job.machine_configuration.as_ref().ok_or_else(|| {
        Diagnostic::new(
            "MACHINE_CONFIGURATION_ABSENT",
            "the job has no applied machine configuration; apply one before export",
        )
        .at_stage("machine")
    })?;
    let mut missing = vec![];
    if configuration.work_offset.is_none() {
        missing.push("work_offset");
    }
    if configuration.clearance_z_mm.is_none() {
        missing.push("clearance_z_mm");
    }
    if configuration.decimal_places.is_none() {
        missing.push("decimal_places");
    }
    if configuration.length_compensation.is_none() {
        missing.push("length_compensation");
    }
    if configuration.path_control.is_none() {
        missing.push("path_control");
    }
    if configuration.spindle_spinup_seconds.is_none() {
        missing.push("spindle_spinup_seconds");
    }
    if configuration.coolant.is_none() {
        missing.push("coolant");
    }
    if configuration.m6.is_none() {
        missing.push("m6");
    }
    if !missing.is_empty() {
        return Err(Diagnostic::new(
            "MACHINE_CONFIGURATION_INCOMPLETE",
            format!(
                "the applied machine configuration is incomplete: {} not set",
                missing.join(", ")
            ),
        )
        .at_stage("machine"));
    }
    let clearance = configuration.clearance_z_mm.expect("checked above");
    if let Some(setup_clearance) = job.setup.clearance_above_stock_mm
        && setup_clearance != clearance
    {
        return Err(Diagnostic::new(
            "MACHINE_CLEARANCE_CONFLICT",
            format!(
                "applied clearance {clearance} mm differs from the job setup clearance {setup_clearance} mm"
            ),
        )
        .at_stage("machine"));
    }
    let length_compensation = configuration.length_compensation.expect("checked above");

    let enabled = scoped_enabled_operations_v5(job, scope)?;
    let mut used_ids = BTreeSet::new();
    for operation in &enabled {
        // The nominal V-bit shape still participates in target geometry and
        // execution fingerprints in endmill-only mode. It is not a physical
        // stage, so its unused controller mapping cannot block preparation.
        let executed = match &operation.settings {
            super::OperationSettingsV5::FlatVcarve(s)
                if s.mode == crate::project::FlatVcarveMode::EndmillOnly =>
            {
                vec![s.endmill.tool_id.as_str()]
            }
            settings => tool_ids_of_v5(settings),
        };
        for id in executed {
            used_ids.insert(id.to_string());
        }
    }
    let rows: BTreeMap<&str, &AppliedToolMapping> = configuration
        .tools
        .iter()
        .map(|mapping| (mapping.job_tool_id.as_str(), mapping))
        .collect();
    let mut active_numbers: BTreeMap<u32, &str> = BTreeMap::new();
    for id in &used_ids {
        let Some(row) = rows.get(id.as_str()) else {
            return Err(mapping_error(id, "has no mapping row"));
        };
        let Some(number) = row.tool_number else {
            return Err(mapping_error(
                id,
                "is used in the selected scope but has no T number",
            ));
        };
        if length_compensation == LengthCompensation::ToolTable
            && row.length_offset_number.is_none()
        {
            return Err(Diagnostic::new(
                "MACHINE_MAPPING_INCOMPLETE",
                format!(
                    "tool '{id}' is used with tool-table length compensation but has no H number"
                ),
            )
            .at_stage("machine"));
        }
        if let Some(other) = active_numbers.insert(number, id.as_str()) {
            return Err(Diagnostic::new(
                "MACHINE_MAPPING_CONFLICT",
                format!("tools '{other}' and '{id}' are both active and map to T{number}"),
            )
            .at_stage("machine"));
        }
    }

    let profile = SequenceProfile {
        schema_version: SEQUENCE_PROFILE_SCHEMA_VERSION,
        id: configuration.origin.configuration_id.clone(),
        work_offset: configuration.work_offset.clone().expect("checked above"),
        clearance_z_mm: clearance,
        decimal_places: configuration.decimal_places.expect("checked above"),
        program_start_position_mm: configuration.program_start_position_mm,
        length_compensation,
        path_control: configuration.path_control.expect("checked above"),
        tools: configuration
            .tools
            .iter()
            .filter(|mapping| job.tools.iter().any(|tool| tool.id == mapping.job_tool_id))
            .filter_map(|mapping| {
                mapping.tool_number.map(|number| SequenceToolMapping {
                    tool_id: mapping.job_tool_id.clone(),
                    tool_number: number,
                    length_offset_number: mapping.length_offset_number,
                })
            })
            .collect(),
        spindle_spinup_seconds: configuration.spindle_spinup_seconds.expect("checked above"),
        coolant: configuration.coolant.expect("checked above"),
        m6: configuration.m6.clone().expect("checked above"),
    };
    profile.validate_shape()?;
    Ok(profile)
}

fn mapping_error(id: &str, what: &str) -> Diagnostic {
    Diagnostic::new("MACHINE_MAPPING_MISSING", format!("tool '{id}' {what}")).at_stage("machine")
}
