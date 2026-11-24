//! Checked file export for upstream three-axis Grbl 1.1 in spindle mode.
mod blocks;
#[cfg(test)]
mod tests;
use super::{
    Coolant, error,
    machine::{Controller, GrblProfile, MachineMoves},
    motion_profile::MotionProfileReport,
    sequence::{
        OutputLayout, PreparedSpindle, SequenceBundle, SequenceProgram, file_groups, finish_bundle,
        resolve_coolant, resolve_spindle,
    },
};
use crate::{
    checks::require_pass,
    geometry::Result,
    motion::Position,
    project::WorkZero,
    sequence::{GenerationStatus, PathControlIntent, ProcessIntent, TrustedSequencePlan},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManualSetup {
    pub controller: Controller,
    pub work_offset: String,
    pub work_zero: WorkZero,
    pub clearance_work_z_mm: f64,
    pub instructions: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct Process {
    spindle: PreparedSpindle,
    coolant: Coolant,
}

// Only preparation sees stage names, ownership and planning intent.
struct Run<'a> {
    process: Process,
    motions: MachineMoves<'a>,
}

fn process(intent: &ProcessIntent, stage_id: &str, profile: &GrblProfile) -> Result<Process> {
    if intent.path_control != PathControlIntent::UseMachineProfile {
        return Err(error(
            "PROCESS_PATH_CONTROL",
            format!(
                "stage '{stage_id}' requires {:?}, which Grbl 1.1 cannot establish",
                intent.path_control
            ),
        )
        .source(stage_id));
    }
    let spindle = resolve_spindle(intent, stage_id)?;
    if matches!(
        spindle,
        PreparedSpindle::On {
            direction: super::SpindleDirection::Counterclockwise,
            ..
        }
    ) && !profile.counterclockwise_spindle
    {
        return Err(error(
            "PROCESS_SPINDLE_STATE",
            format!("stage '{stage_id}' requires counterclockwise spindle support"),
        )
        .source(stage_id));
    }
    Ok(Process {
        spindle,
        coolant: resolve_coolant(intent, profile.coolant),
    })
}

pub fn export_bundle(
    plan: &dyn TrustedSequencePlan,
    profile: &GrblProfile,
    layout: OutputLayout,
) -> Result<SequenceBundle> {
    profile.validate()?;
    let plan = plan.trusted();
    let checks = plan.basic_checks()?;
    require_pass(&checks)?;
    if plan.motions().is_empty() {
        return Err(error("POST_EMPTY", "no executable motions"));
    }
    if plan.operation_results().iter().any(|r| {
        matches!(
            r.generation_status,
            GenerationStatus::Incomplete | GenerationStatus::Inconclusive
        )
    }) {
        return Err(error(
            "PLAN_GENERATION_INCOMPLETE",
            "one or more operations did not generate completely",
        ));
    }
    if plan.setup().clearance_above_stock_mm != Some(profile.clearance_z_mm) {
        return Err(error(
            "POST_CLEARANCE",
            "Grbl profile clearance differs from planned setup clearance",
        ));
    }
    let runs = super::machine::stage_runs(plan)?
        .into_iter()
        .map(|run| {
            Ok(Run {
                process: process(&run.intent, &run.stage.stage_id, profile)?,
                motions: run.motions,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let groups = file_groups(plan.stages(), OutputLayout::SequentialFiles);
    if layout == OutputLayout::OneProgram && groups.len() > 1 {
        let stage = &plan.stages()[groups[1].0];
        return Err(error(
            "GRBL_TOOL_CHANGE",
            format!(
                "stage '{}' needs tool '{}'. Select sequential files for manual tool changes.",
                stage.stage_id, stage.tool_id
            ),
        ));
    }
    let origin = crate::setup::resolve_work_zero_setup(plan.setup())?;
    let frame = Position {
        x: origin.x_mm,
        y: origin.y_mm,
        z: origin.z_mm,
    };
    let clearance = profile.clearance_z_mm - frame.z;
    let manual_setup = ManualSetup {
        controller: profile.controller,
        work_offset: profile.work_offset.clone(),
        work_zero: plan.setup().work_zero.clone(),
        clearance_work_z_mm: clearance,
        instructions: concat!(
            "Run files in manifest order on the same stock. Before each file fit its toolId, ",
            "preserve work XY and touch off Z at the job datum. Start in spindle mode with ",
            "G92 and tool-length offsets inactive and a clear vertical path to clearanceWorkZMm."
        )
        .into(),
    };
    let hash = |e: serde_json::Error| error("POST_JSON", e.to_string());
    let profile_id = crate::plan_hash::hash(profile).map_err(hash)?;
    let processes: Vec<_> = runs
        .iter()
        .zip(plan.stages())
        .map(|(r, s)| (&s.tool_id, r.process))
        .collect();
    let process_id =
        crate::plan_hash::hash(&(profile.controller, &manual_setup, processes)).map_err(hash)?;
    // A finer formatting attempt must still meet the original geometric budget.
    // The controller reads 32-bit floats regardless of the number of printed places.
    let tolerance = (0.5 * 10f64.powi(-(profile.decimal_places as i32))).min(
        plan.tolerances()
            .motion_tolerance_mm
            .unwrap_or(f64::INFINITY),
    );
    for places in profile.decimal_places..=9 {
        let mut files = Vec::with_capacity(groups.len());
        let mut retry = false;
        for (index, &(start, end)) in groups.iter().enumerate() {
            let file = blocks::program(&runs[start..end], profile, frame, places, tolerance);
            let gcode = match file {
                Err(e) if e.code == "POST_PRECISION" && places < 9 => {
                    retry = true;
                    break;
                }
                other => other?,
            };
            let filename = if layout == OutputLayout::OneProgram {
                "sequence.nc".into()
            } else {
                format!("{:02}-tool.nc", index + 1)
            };
            files.push(SequenceProgram { filename, gcode });
        }
        if retry {
            continue;
        }
        let motion_profile = MotionProfileReport::from_stages(
            plan.stages().iter().map(|s| (s, None, None)),
            plan.motions(),
            profile.decimal_places,
            places,
        );
        return Ok(finish_bundle(
            plan,
            layout,
            files,
            &groups,
            super::sequence::BundleVerification {
                profile_fingerprint: &profile_id,
                process_fingerprint: &process_id,
                machine_offset_mm: [frame.x, frame.y, frame.z],
                basic_checks: checks,
                motion_profile,
                knife_replay: None,
            },
            Some(manual_setup),
        ));
    }
    unreachable!("the final precision attempt returns its error")
}
