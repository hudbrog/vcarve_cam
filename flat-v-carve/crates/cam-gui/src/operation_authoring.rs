//! Operation lifecycle for the current single-operation workspace.
use cam_core::project::v5::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Action {
    Delete,
    AddKnife,
    AddVcarve,
}

pub fn empty_job() -> CamJobV5 {
    CamJobV5 {
        schema_version: 5,
        name: "New job".into(),
        setup: Default::default(),
        artwork: vec![],
        tools: vec![],
        operations: vec![],
        tolerances: cam_core::job::PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
        },
        machine_configuration: None,
        legacy_machine_profile: None,
    }
}

fn new_tool(job: &mut CamJobV5, prefix: &str, name: &str) -> String {
    let mut id = prefix.to_owned();
    let mut n = 2;
    while job.tools.iter().any(|t| t.id == id) {
        id = format!("{prefix}-{n}");
        n += 1;
    }
    job.tools.push(JobToolV5 {
        id: id.clone(),
        name: name.into(),
        geometry: None,
        capabilities: Default::default(),
        library_origin: None,
    });
    id
}

pub fn apply(job: &CamJobV5, action: Action) -> Result<CamJobV5, String> {
    let mut job = job.clone();
    if matches!(action, Action::Delete) {
        if job.operations.len() != 1 {
            return Err("Select an operation to delete".into());
        }
        job.operations.clear();
    } else {
        if !job.operations.is_empty() {
            return Err("Delete the current operation before adding another. Multiple-operation sequences are not supported in this workspace yet.".into());
        }
        let (id, name, settings) = match action {
            Action::AddKnife => {
                let tool_id = new_tool(&mut job, "knife-tool", "Drag knife");
                // This adds support for strokes; filled source bytes remain fills.
                for item in &mut job.artwork {
                    item.import_settings.mode = cam_core::svg::ImportMode::Centerline;
                }
                (
                    "knife",
                    "Drag knife",
                    OperationSettingsV5::DragKnife(DragKnifeSettingsV5 {
                        chains: vec![],
                        assignment: KnifeAssignmentV5 {
                            tool_id,
                            cutting_feed_mm_min: None,
                            plunge_feed_mm_min: None,
                            swivel_feed_mm_min: None,
                            max_stepdown_mm: None,
                            applied_profile: None,
                        },
                        top: Default::default(),
                        bottom: Default::default(),
                        stepdown_mm: None,
                        swivel_depth_mm: None,
                        corner_threshold_deg: None,
                        through_cut_allowance_mm: None,
                        start: Default::default(),
                        closure_overlap_mm: None,
                        alignment: Default::default(),
                    }),
                )
            }
            Action::AddVcarve => {
                let assignment = |tool_id| MillingAssignmentV5 {
                    tool_id,
                    spindle_rpm: None,
                    spindle_direction: None,
                    cutting_feed_mm_min: None,
                    plunge_feed_mm_min: None,
                    max_stepdown_mm: None,
                    stepover_mm: None,
                    applied_profile: None,
                };
                let endmill = assignment(new_tool(&mut job, "endmill", "Endmill"));
                let vbit = assignment(new_tool(&mut job, "vbit", "V-bit target"));
                (
                    "carving",
                    "Flat V-carve",
                    OperationSettingsV5::FlatVcarve(FlatVcarveSettingsV5 {
                        components: vec![],
                        mode: cam_core::project::FlatVcarveMode::EndmillOnly,
                        endmill,
                        vbit,
                        top: Default::default(),
                        max_depth_mm: None,
                        wall_allowance_mm: None,
                        max_floor_ridge_mm: None,
                        max_detail_residual_mm: None,
                        rough: None,
                        finish: None,
                    }),
                )
            }
            Action::Delete => unreachable!(),
        };
        job.operations.push(OperationV5 {
            id: id.into(),
            name: name.into(),
            enabled: true,
            settings,
        });
    }
    job.validate_structure().map_err(|e| e.to_string())?;
    Ok(job)
}
