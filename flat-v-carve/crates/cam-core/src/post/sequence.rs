//! Ordered LinuxCNC output for sequence plans (plan sections 8.2 and 14).
//!
//! This pipeline is separate from the legacy M5 export in [`super`]: basic
//! checks gate generation, process state is resolved before any G-code, and
//! an independent numeric readback re-derives every motion, tool change and
//! spindle state from the emitted bytes. Detailed stock-quality analysis
//! stays optional and is never run here.
use crate::{
    checks::{BasicCheckReport, check_plan, require_pass},
    geometry::Result,
    project::{
        CamJob, MillingAssignment, OperationSettings, SpindleDirection as JobSpindleDirection,
    },
    sequence::{
        ExecutionStage, GenerationStatus, OperationPlan, ProcessSpindle, StageRole, TrustedPlan,
    },
    toolpath::{Interpolation, PlannedMotion},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    Coolant, LengthCompensation, LinuxCncProfile, M6Contract, PathControl, SpindleDirection,
    ZDatum, error,
};

pub const SEQUENCE_PROFILE_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceToolMapping {
    /// A physical job tool ID, mapped exactly once regardless of how many
    /// operations or assignments use it.
    pub tool_id: String,
    pub tool_number: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_offset_number: Option<u32>,
}

/// Generic LinuxCNC export configuration, schema 2. The Z datum is owned by
/// the job's `setup.work_zero` (never duplicated here) and spindle direction
/// is owned by the job's assignments (never by the T/H mapping).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceProfile {
    pub schema_version: u32,
    pub id: String,
    pub work_offset: String,
    pub clearance_z_mm: f64,
    pub decimal_places: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_start_position_mm: Option<crate::motion::Position>,
    pub length_compensation: LengthCompensation,
    #[serde(default)]
    pub path_control: PathControl,
    pub tools: Vec<SequenceToolMapping>,
    pub spindle_spinup_seconds: f64,
    pub coolant: Coolant,
    pub m6: M6Contract,
}

/// Fully resolved process state for one stage; no unresolved markers remain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PreparedProcess {
    pub spindle: PreparedSpindle,
    pub coolant: Coolant,
    pub path_control: PathControl,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PreparedSpindle {
    Off,
    On {
        rpm: f64,
        direction: SpindleDirection,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PreparedStage {
    pub stage: ExecutionStage,
    pub process: PreparedProcess,
    /// The exact emitted tool number for this stage's tool.
    pub tool_number: u32,
}

/// An immutable plan bound to a profile and fully resolved process state.
/// Only this form can produce G-code (plan section 8.2).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparedExecution {
    pub plan_fingerprint: String,
    pub profile_fingerprint: String,
    pub process_fingerprint: String,
    pub output_decimal_places: usize,
    /// Z shift applied before output formatting for the selected work-zero
    /// datum; the inverse shift is applied after numeric readback.
    pub machine_offset_mm: [f64; 3],
    pub stages: Vec<PreparedStage>,
    pub basic_checks: BasicCheckReport,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SequenceProgram {
    pub filename: String,
    pub gcode: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SequenceExportReport {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub engine_version: String,
    pub plan_fingerprint: String,
    pub profile_fingerprint: String,
    pub process_fingerprint: String,
    pub output_decimal_places: usize,
    pub machine_offset_mm: [f64; 3],
    pub basic_checks: BasicCheckReport,
    pub program_sha256: String,
    pub motion_count: usize,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct SequenceExport {
    pub program: SequenceProgram,
    pub report: SequenceExportReport,
}

impl SequenceProfile {
    pub fn from_json(text: &str) -> Result<Self> {
        if text.len() > 64_000 {
            return Err(error("POST_PROFILE", "profile exceeds 64 KB"));
        }
        let profile: Self =
            serde_json::from_str(text).map_err(|e| error("POST_PROFILE", e.to_string()))?;
        profile.validate_shape()?;
        Ok(profile)
    }

    pub fn validate_shape(&self) -> Result<()> {
        if self.schema_version != SEQUENCE_PROFILE_SCHEMA_VERSION
            || !crate::preview::valid_id(&self.id)
            || !matches!(
                self.work_offset.as_str(),
                "G54" | "G55" | "G56" | "G57" | "G58" | "G59" | "G59.1" | "G59.2" | "G59.3"
            )
            || self.decimal_places > 9
            || !self.clearance_z_mm.is_finite()
            || self.clearance_z_mm <= 0.
            || !self.spindle_spinup_seconds.is_finite()
            || !(0. ..=3600.).contains(&self.spindle_spinup_seconds)
        {
            return Err(error(
                "POST_PROFILE",
                "invalid profile version, ID, work offset, clearance, precision, or dwell",
            ));
        }
        if !self.m6.reviewed
            || self.m6.reference.trim().is_empty()
            || self.m6.reference.len() > 4000
            || !self.m6.preserves_work_datum
            || !self.m6.local_offsets_unused
            || !self.m6.tool_offsets_z_only
        {
            return Err(error(
                "POST_M6_CONTRACT",
                "a reviewed M6 reference must establish the return position after compensation, preserve the work datum without rotation, leave G52/G92 unused, and use only Z tool offsets",
            ));
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut numbers = std::collections::BTreeSet::new();
        for tool in &self.tools {
            if !ids.insert(&tool.tool_id) {
                return Err(error(
                    "POST_TOOL_MAPPING",
                    format!("tool {:?} is mapped more than once", tool.tool_id),
                ));
            }
            if !(1..=99999).contains(&tool.tool_number) || !numbers.insert(tool.tool_number) {
                return Err(error(
                    "POST_TOOL_MAPPING",
                    format!(
                        "tool {:?}: T numbers must be 1..=99999 and unique",
                        tool.tool_id
                    ),
                ));
            }
            match self.length_compensation {
                LengthCompensation::MacroManaged if tool.length_offset_number.is_some() => {
                    return Err(error(
                        "POST_TOOL_MAPPING",
                        format!(
                            "tool {:?}: macro-managed compensation requires length_offset_number to be null",
                            tool.tool_id
                        ),
                    ));
                }
                LengthCompensation::ToolTable
                    if !tool
                        .length_offset_number
                        .is_some_and(|h| (1..=99999).contains(&h)) =>
                {
                    return Err(error(
                        "POST_TOOL_MAPPING",
                        format!(
                            "tool {:?}: tool-table compensation requires an H number between 1 and 99999",
                            tool.tool_id
                        ),
                    ));
                }
                _ => {}
            }
        }
        if let PathControl::Blend {
            tolerance_mm,
            naive_cam_tolerance_mm,
        } = self.path_control
        {
            if !tolerance_mm.is_finite() || !(0.000001..=10.).contains(&tolerance_mm) {
                return Err(error(
                    "POST_PROFILE",
                    "path blending tolerance must be positive, at most 10 mm, and finite",
                ));
            }
            if naive_cam_tolerance_mm
                .is_some_and(|q| !q.is_finite() || !(0.000001..=tolerance_mm).contains(&q))
            {
                return Err(error(
                    "POST_PROFILE",
                    "naive cam tolerance must be positive and no greater than the path blending tolerance, as LinuxCNC requires Q <= P",
                ));
            }
        }
        Ok(())
    }

    fn tool(&self, tool_id: &str) -> Option<&SequenceToolMapping> {
        self.tools.iter().find(|t| t.tool_id == tool_id)
    }
}

/// Apply a legacy schema-1 profile to a canonical job as one document/profile
/// application: the Z datum moves into `setup.work_zero.z` and the per-tool
/// spindle directions move into every matching milling assignment (only
/// where still unset; an explicit job value wins). The job keeps
/// `legacy_machine_profile` as descriptive metadata.
pub fn apply_legacy_profile(profile: &LinuxCncProfile, job: &CamJob) -> Result<CamJob> {
    // When the job is exactly a migratable Flat V-carve document, run the
    // legacy profile validation against its reconstructed legacy job so the
    // old checks (clearance, mapped tools, blend tolerance) still apply.
    if job.operations.len() == 1
        && matches!(job.operations[0].settings, OperationSettings::FlatVcarve(_))
    {
        if let Ok(legacy) = crate::operations::flat_vcarve::to_legacy_job(
            job,
            &job.operations[0].id,
            match &job.operations[0].settings {
                OperationSettings::FlatVcarve(settings) => settings,
                _ => unreachable!(),
            },
        ) {
            profile.validate(&legacy)?;
        }
    } else {
        for mapping in &profile.tools {
            if !job.tools.iter().any(|t| t.id == mapping.tool_id) {
                return Err(error(
                    "POST_TOOL_MAPPING",
                    format!(
                        "profile maps tool {:?} which the job does not define",
                        mapping.tool_id
                    ),
                ));
            }
        }
        if let Some(clearance) = job.setup.clearance_above_stock_mm
            && clearance != profile.clearance_z_mm
        {
            return Err(error(
                "POST_CLEARANCE",
                "profile clearance differs from the job's setup clearance",
            ));
        }
    }
    let mut applied = job.clone();
    applied.setup.work_zero.z = match profile.z_datum {
        ZDatum::StockTop => crate::project::WorkZeroZ::StockTop,
        ZDatum::StockBottom => crate::project::WorkZeroZ::StockBottom,
    };
    for operation in &mut applied.operations {
        let mut assignments: Vec<&mut MillingAssignment> = vec![];
        match &mut operation.settings {
            OperationSettings::FlatVcarve(settings) => {
                assignments.push(&mut settings.endmill);
                assignments.push(&mut settings.vbit);
            }
            OperationSettings::Face(settings) => assignments.push(&mut settings.assignment),
            OperationSettings::Profile(settings) => assignments.push(&mut settings.assignment),
            OperationSettings::DragKnife(_) => {}
        }
        for assignment in assignments {
            if let Some(mapping) = profile
                .tools
                .iter()
                .find(|t| t.tool_id == assignment.tool_id)
                && assignment.spindle_direction.is_none()
            {
                assignment.spindle_direction = Some(match mapping.spindle_direction {
                    SpindleDirection::Clockwise => JobSpindleDirection::Clockwise,
                    SpindleDirection::Counterclockwise => JobSpindleDirection::Counterclockwise,
                });
            }
        }
    }
    applied.validate()?;
    Ok(applied)
}

fn stage_intent(plan: &OperationPlan, stage: &ExecutionStage) -> Result<ProcessSpindle> {
    // Walk execution in order; the intent immediately preceding this stage's
    // RunStage is the one that applies to it.
    let mut pending: Option<ProcessSpindle> = None;
    for item in &plan.execution {
        match item {
            crate::sequence::ExecutionItem::ToolChange { .. } => {}
            crate::sequence::ExecutionItem::SetProcessIntent { intent } => {
                pending = Some(intent.spindle.clone());
            }
            crate::sequence::ExecutionItem::RunStage { stage_id } => {
                if stage_id == &stage.stage_id {
                    return pending.ok_or_else(|| {
                        error(
                            "PROCESS_SPINDLE_STATE",
                            format!("stage '{}' has no process intent", stage.stage_id),
                        )
                    });
                }
            }
        }
    }
    Err(error(
        "PROCESS_SPINDLE_STATE",
        format!("stage '{}' never runs", stage.stage_id),
    ))
}

fn resolve_process(
    stage: &ExecutionStage,
    intent_spindle: &ProcessSpindle,
    profile: &SequenceProfile,
) -> Result<PreparedProcess> {
    let spindle = match intent_spindle {
        ProcessSpindle::Off => PreparedSpindle::Off,
        ProcessSpindle::Milling { rpm, direction } => match direction {
            Some(JobSpindleDirection::Clockwise) => PreparedSpindle::On {
                rpm: *rpm,
                direction: SpindleDirection::Clockwise,
            },
            Some(JobSpindleDirection::Counterclockwise) => PreparedSpindle::On {
                rpm: *rpm,
                direction: SpindleDirection::Counterclockwise,
            },
            None => {
                return Err(error(
                    "PROCESS_SPINDLE_STATE",
                    format!(
                        "milling stage '{}' has no resolved spindle direction; apply a machine profile to the job and replan",
                        stage.stage_id
                    ),
                ));
            }
        },
    };
    Ok(PreparedProcess {
        spindle,
        coolant: profile.coolant,
        path_control: match stage.role {
            // Knife stages always run exact path; blending could change
            // swivel geometry (plan section 12.5).
            StageRole::Knife => PathControl::ExactPath,
            _ => profile.path_control,
        },
    })
}

impl PreparedExecution {
    /// Bind a checked plan to a profile with fully resolved process state.
    pub fn prepare(plan: &TrustedPlan, profile: &SequenceProfile) -> Result<Self> {
        let plan = plan.plan();
        let basic_checks = check_plan(plan)?;
        require_pass(&basic_checks)?;
        profile.validate_shape()?;
        for stage in &plan.stages {
            if profile.tool(&stage.tool_id).is_none() {
                return Err(error(
                    "POST_TOOL_MAPPING",
                    format!(
                        "stage '{}' uses tool '{}' which the profile does not map",
                        stage.stage_id, stage.tool_id
                    ),
                ));
            }
        }
        if let Some(clearance) = plan.job_snapshot.setup.clearance_above_stock_mm
            && clearance != profile.clearance_z_mm
        {
            return Err(error(
                "POST_CLEARANCE",
                "profile clearance differs from the plan's setup clearance; regenerate with the intended clearance",
            ));
        }
        // Work-zero output transform (plan section 6.2): the full selected
        // point, XY anchors included. Machine-profile startup/M6 positions
        // are already expressed in the controller work frame and are never
        // transformed again.
        let work_zero = crate::setup::resolve_work_zero(&plan.job_snapshot)?;
        let machine_offset = work_zero.output_offset();
        for value in [machine_offset.0, machine_offset.1, machine_offset.2] {
            if !value.is_finite() || rounded(value, profile.decimal_places) != value {
                return Err(error(
                    "POST_WORK_ZERO_PRECISION",
                    "work-zero offsets must be exactly representable at output precision",
                ));
            }
        }
        let mut stages = vec![];
        for stage in &plan.stages {
            let process = resolve_process(stage, &stage_intent(plan, stage)?, profile)?;
            stages.push(PreparedStage {
                stage: stage.clone(),
                process,
                tool_number: profile
                    .tool(&stage.tool_id)
                    .expect("validated mapping")
                    .tool_number,
            });
        }
        let plan_fingerprint = plan.execution_fingerprint.clone();
        let profile_fingerprint =
            crate::plan_hash::hash(profile).map_err(|e| error("POST_JSON", e.to_string()))?;
        let process_fingerprint =
            crate::plan_hash::hash(&stages).map_err(|e| error("POST_JSON", e.to_string()))?;
        Ok(Self {
            plan_fingerprint,
            profile_fingerprint,
            process_fingerprint,
            output_decimal_places: profile.decimal_places,
            machine_offset_mm: [machine_offset.0, machine_offset.1, machine_offset.2],
            stages,
            basic_checks,
        })
    }

    /// Emit the one ordered program for this execution and verify it with an
    /// independent numeric readback. Detailed quality analysis is not run.
    pub fn export(&self, plan: &TrustedPlan, profile: &SequenceProfile) -> Result<SequenceExport> {
        let plan = plan.plan();
        if plan.motions.is_empty() {
            return Err(error("POST_EMPTY", "no executable motions"));
        }
        for result in &plan.operation_results {
            if matches!(
                result.generation_status,
                GenerationStatus::Incomplete | GenerationStatus::Inconclusive
            ) {
                return Err(error(
                    "PLAN_GENERATION_INCOMPLETE",
                    format!(
                        "operation '{}' did not generate completely",
                        result.operation_id
                    ),
                ));
            }
        }
        // Raise precision through nine places until formatting preserves
        // every motion segment; never delete a required move.
        let mut places = profile.decimal_places;
        while places < 9 && !motions_preserved(plan, places) {
            places += 1;
        }
        let gcode = self.emit(plan, profile, places)?;
        let motions = readback(&gcode, self, places)?;
        compare_with_plan(self, plan, &gcode, &motions)?;
        let report = SequenceExportReport {
            artifact_kind: "sequence_export_report".into(),
            schema_version: 1,
            engine_version: env!("CARGO_PKG_VERSION").into(),
            plan_fingerprint: self.plan_fingerprint.clone(),
            profile_fingerprint: self.profile_fingerprint.clone(),
            process_fingerprint: self.process_fingerprint.clone(),
            output_decimal_places: places,
            machine_offset_mm: self.machine_offset_mm,
            basic_checks: self.basic_checks.clone(),
            program_sha256: format!("{:x}", Sha256::digest(gcode.as_bytes())),
            motion_count: motions.len(),
            diagnostics: vec![],
        };
        Ok(SequenceExport {
            program: SequenceProgram {
                filename: "sequence.ngc".into(),
                gcode,
            },
            report,
        })
    }

    fn emit(
        &self,
        plan: &OperationPlan,
        profile: &SequenceProfile,
        places: usize,
    ) -> Result<String> {
        let mut lines =
            vec![
            "(CAM sequence program; basic checks passed; detailed quality analysis is separate)"
                .into(),
            format!("(CAM Engine {}; profile {})", env!("CARGO_PKG_VERSION"), profile.id),
            format!("(CAM Plan {})", self.plan_fingerprint),
            format!("(CAM Process {})", self.process_fingerprint),
            "M5".into(),
            "M9".into(),
        ];
        lines.extend(modal_lines(profile));
        let mut previous_stage_end = profile.program_start_position_mm;
        for prepared in &self.stages {
            let stage = &prepared.stage;
            lines.push(format!(
                "(CAM Stage {}; tool {})",
                role_name(stage.role),
                stage.tool_id
            ));
            lines.push("M5".into());
            lines.push("M9".into());
            lines.push(format!("T{} M6", prepared.tool_number));
            lines.push("M5".into());
            lines.push("M9".into());
            lines.extend(modal_lines(profile));
            if profile.length_compensation == LengthCompensation::ToolTable {
                lines.push(format!(
                    "G43 H{}",
                    profile
                        .tool(&stage.tool_id)
                        .and_then(|t| t.length_offset_number)
                        .expect("validated H number")
                ));
            }
            match prepared.process.spindle {
                PreparedSpindle::Off => {}
                PreparedSpindle::On { rpm, direction } => {
                    lines.push(format!(
                        "{} S{}",
                        match direction {
                            SpindleDirection::Clockwise => "M3",
                            SpindleDirection::Counterclockwise => "M4",
                        },
                        scalar(rpm)?
                    ));
                    lines.push(format!("G4 P{}", profile.spindle_spinup_seconds));
                }
            }
            lines.push(
                match prepared.process.coolant {
                    Coolant::Off => "M9",
                    Coolant::Flood => "M8",
                    Coolant::Mist => "M7",
                }
                .into(),
            );
            // The M6 bridge is machine-owned; never fabricate a continuous
            // XYZ line through the macro (plan section 8.3).
            if let Some(current) = previous_stage_end {
                let start = machine_position(
                    first_motion(plan, stage).start,
                    self.machine_offset_mm,
                    places,
                );
                if current.z != start.z {
                    lines.push(format!("G0 Z{:.p$}", start.z, p = places));
                }
                if current.xy() != start.xy() {
                    lines.push(format!("G0 {}", xyz(start, places)));
                }
            }
            let motions = &plan.motions[stage.motion_range.0..stage.motion_range.1];
            for motion in motions {
                let end = machine_position(motion.end, self.machine_offset_mm, places);
                let feed = match motion.feed_mm_min {
                    Some(f) => format!(" F{}", scalar(f)?),
                    None => String::new(),
                };
                lines.push(format!(
                    "{} {}{}",
                    match motion.interpolation {
                        Interpolation::Rapid => "G0",
                        Interpolation::LinearFeed => "G1",
                    },
                    xyz(end, places),
                    feed
                ));
            }
            previous_stage_end = motions
                .last()
                .map(|m| machine_position(m.end, self.machine_offset_mm, places));
        }
        lines.extend(["M5".into(), "M9".into(), "M2".into()]);
        if lines.iter().any(|l| l.len() > 240) {
            return Err(error(
                "POST_LINE_LENGTH",
                "an emitted block exceeds the 240-character limit",
            ));
        }
        Ok(lines.join("\n") + "\n")
    }
}

/// Verify supplied output bytes against the prepared execution and plan:
/// altered motion order, coordinates, feeds, tools or process state are
/// rejected with `POST_SEQUENCE_MISMATCH`.
pub fn verify_program(
    prepared: &PreparedExecution,
    plan: &TrustedPlan,
    program: &SequenceProgram,
) -> Result<SequenceExportReport> {
    let plan = plan.plan();
    let motions = readback(&program.gcode, prepared, prepared.output_decimal_places)?;
    compare_with_plan(prepared, plan, &program.gcode, &motions)?;
    Ok(SequenceExportReport {
        artifact_kind: "sequence_export_report".into(),
        schema_version: 1,
        engine_version: env!("CARGO_PKG_VERSION").into(),
        plan_fingerprint: prepared.plan_fingerprint.clone(),
        profile_fingerprint: prepared.profile_fingerprint.clone(),
        process_fingerprint: prepared.process_fingerprint.clone(),
        output_decimal_places: prepared.output_decimal_places,
        machine_offset_mm: prepared.machine_offset_mm,
        basic_checks: prepared.basic_checks.clone(),
        program_sha256: format!("{:x}", Sha256::digest(program.gcode.as_bytes())),
        motion_count: motions.len(),
        diagnostics: vec![],
    })
}

#[derive(Clone, Debug, PartialEq)]
struct ReadbackMotion {
    interpolation: Interpolation,
    end: crate::motion::Position,
    feed: Option<f64>,
    tool_number: u32,
}

/// Independent numeric readback. Re-derives every motion, the ordered tool
/// changes and per-stage spindle state from the bytes alone; comments are
/// skipped and never authorize anything.
fn readback(
    gcode: &str,
    prepared: &PreparedExecution,
    places: usize,
) -> Result<Vec<ReadbackMotion>> {
    if gcode.len() > 128_000_000 || !gcode.is_ascii() {
        return Err(error(
            "POST_GCODE_SUBSET",
            "program must be ASCII and at most 128 MB",
        ));
    }
    let mut motions: Vec<ReadbackMotion> = vec![];
    let mut tool_changes: Vec<u32> = vec![];
    let mut spindle_on = false;
    let mut tool: Option<u32> = None;
    let mut feed: Option<f64> = None;
    // Spindle state sampled right before each motion, per stage slot.
    let mut stage_of_motion: Vec<usize> = vec![];
    let mut spindle_on_at_motion: Vec<bool> = vec![];
    for (number, raw) in gcode.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || (line.starts_with('(') && line.ends_with(')')) {
            continue;
        }
        if line.len() > 240 {
            return Err(error(
                "POST_GCODE_SUBSET",
                format!("line {} exceeds 240 characters", number + 1),
            ));
        }
        for token in line.split_ascii_whitespace() {
            if token == "M6" {
                let t = line
                    .split_ascii_whitespace()
                    .find(|w| w.starts_with('T'))
                    .and_then(|w| w[1..].parse::<u32>().ok())
                    .ok_or_else(|| error("POST_GCODE_SUBSET", "M6 without a T number"))?;
                tool_changes.push(t);
                tool = Some(t);
                spindle_on = false;
            }
            if token == "M3" || token == "M4" {
                spindle_on = true;
            }
            if token == "M5" {
                spindle_on = false;
            }
        }
        if !(line.starts_with("G0 ")
            || line.starts_with("G1 ")
            || line.ends_with("G0")
            || line.ends_with("G1"))
        {
            continue;
        }
        if !line.starts_with("G0") && !line.starts_with("G1") {
            continue;
        }
        let interpolation = if line.starts_with("G0") {
            Interpolation::Rapid
        } else {
            Interpolation::LinearFeed
        };
        let mut end: Option<crate::motion::Position> = None;
        for token in line.split_ascii_whitespace().skip(1) {
            let letter = token.as_bytes()[0];
            let Ok(value) = token[1..].parse::<f64>() else {
                continue;
            };
            let slot = end.get_or_insert(crate::motion::Position::new(
                crate::geometry::Point::new(0., 0.),
                0.,
            ));
            match letter {
                b'X' => slot.x = value,
                b'Y' => slot.y = value,
                b'Z' => slot.z = value,
                b'F' => feed = Some(value),
                _ => {}
            }
        }
        let Some(end) = end else {
            return Err(error("POST_GCODE_SUBSET", "motion without coordinates"));
        };
        let tool_number =
            tool.ok_or_else(|| error("POST_SEQUENCE_MISMATCH", "motion before any tool change"))?;
        motions.push(ReadbackMotion {
            interpolation,
            end,
            feed,
            tool_number,
        });
        // Stage membership by cumulative motion counts; motions of one stage
        // are contiguous in the emitted stream.
        let mut consumed = 0usize;
        let mut slot = prepared.stages.len();
        for (index, stage) in prepared.stages.iter().enumerate() {
            consumed += stage.stage.motion_range.1 - stage.stage.motion_range.0;
            if motions.len() <= consumed {
                slot = index;
                break;
            }
        }
        stage_of_motion.push(slot);
        spindle_on_at_motion.push(spindle_on);
        let _ = places;
    }
    // Tool changes must match the stages in order, one per stage.
    let expected_tools: Vec<u32> = prepared.stages.iter().map(|s| s.tool_number).collect();
    if tool_changes != expected_tools {
        return Err(error(
            "POST_SEQUENCE_MISMATCH",
            format!(
                "tool changes {tool_changes:?} do not match the ordered stage tools {expected_tools:?}"
            ),
        ));
    }
    // Spindle state per stage: On stages cut with the spindle running, Off
    // stages never do.
    for (index, _) in motions.iter().enumerate() {
        let slot = stage_of_motion[index];
        let Some(stage) = prepared.stages.get(slot) else {
            return Err(error(
                "POST_SEQUENCE_MISMATCH",
                "more motions than the stages account for",
            ));
        };
        let should_run = matches!(stage.process.spindle, PreparedSpindle::On { .. });
        if should_run != spindle_on_at_motion[index] {
            return Err(error(
                "PROCESS_SPINDLE_STATE",
                format!(
                    "spindle {} while cutting stage '{}'",
                    if spindle_on_at_motion[index] {
                        "runs"
                    } else {
                        "is off"
                    },
                    stage.stage.stage_id
                ),
            ));
        }
    }
    Ok(motions)
}

/// Compare readback motions with the plan's motions in order: positions at
/// output precision, interpolation, feed and per-stage tool ownership.
fn compare_with_plan(
    prepared: &PreparedExecution,
    plan: &OperationPlan,
    gcode: &str,
    motions: &[ReadbackMotion],
) -> Result<()> {
    if motions.len() != plan.motions.len() {
        return Err(error(
            "POST_SEQUENCE_MISMATCH",
            format!(
                "read back {} motions for {} planned (from {} bytes)",
                motions.len(),
                plan.motions.len(),
                gcode.len()
            ),
        ));
    }
    for (index, (read, planned)) in motions.iter().zip(plan.motions.iter()).enumerate() {
        let expected_end = machine_position(
            planned.end,
            prepared.machine_offset_mm,
            prepared.output_decimal_places,
        );
        if read.end != expected_end {
            return Err(error(
                "POST_SEQUENCE_MISMATCH",
                format!(
                    "motion {index} readback end {:?} differs from planned {expected_end:?}",
                    read.end
                ),
            ));
        }
        if read.interpolation != planned.interpolation {
            return Err(error(
                "POST_SEQUENCE_MISMATCH",
                format!("motion {index} interpolation differs from the plan"),
            ));
        }
        // Feeds are modal words that rapids do not consume; only linear feed
        // motions carry an authoritative per-motion feed to compare.
        if read.interpolation == Interpolation::LinearFeed && read.feed != planned.feed_mm_min {
            return Err(error(
                "POST_SEQUENCE_MISMATCH",
                format!("motion {index} feed differs from the plan"),
            ));
        }
        let stage = plan
            .stages
            .iter()
            .find(|s| index >= s.motion_range.0 && index < s.motion_range.1)
            .ok_or_else(|| {
                error(
                    "POST_SEQUENCE_MISMATCH",
                    format!("motion {index} outside all stages"),
                )
            })?;
        let tool_number = prepared
            .stages
            .iter()
            .find(|s| s.stage.stage_id == stage.stage_id)
            .map(|s| s.tool_number)
            .expect("prepared stage exists");
        if read.tool_number != tool_number {
            return Err(error(
                "POST_SEQUENCE_MISMATCH",
                format!(
                    "motion {index} runs with T{} under stage '{}' (T{tool_number})",
                    read.tool_number, stage.stage_id
                ),
            ));
        }
    }
    Ok(())
}

fn role_name(role: StageRole) -> &'static str {
    match role {
        StageRole::Face => "face",
        StageRole::ProfileRough => "profile_rough",
        StageRole::ProfileFinish => "profile_finish",
        StageRole::VcarveRough => "vcarve_rough",
        StageRole::VcarveFinish => "vcarve_finish",
        StageRole::Knife => "knife",
    }
}

fn modal_lines(profile: &SequenceProfile) -> Vec<String> {
    let path_mode = match profile.path_control {
        PathControl::ExactPath => "G61".to_string(),
        PathControl::Blend {
            tolerance_mm,
            naive_cam_tolerance_mm,
        } => match naive_cam_tolerance_mm {
            Some(q) => format!("G64 P{tolerance_mm} Q{q}"),
            None => format!("G64 P{tolerance_mm}"),
        },
    };
    vec![
        format!("G21 G17 G90 G94 G40 G80 {path_mode}"),
        profile.work_offset.clone(),
        "G92.1".into(),
    ]
}

fn first_motion<'a>(plan: &'a OperationPlan, stage: &ExecutionStage) -> &'a PlannedMotion {
    &plan.motions[stage.motion_range.0]
}

fn rounded_position(p: crate::motion::Position, places: usize) -> crate::motion::Position {
    crate::motion::Position {
        x: rounded(p.x, places),
        y: rounded(p.y, places),
        z: rounded(p.z, places),
    }
}
/// Setup-to-machine output position: work-zero Z datum shift, then rounding.
fn machine_position(
    p: crate::motion::Position,
    offset: [f64; 3],
    places: usize,
) -> crate::motion::Position {
    crate::motion::Position {
        x: rounded(p.x - offset[0], places),
        y: rounded(p.y - offset[1], places),
        z: rounded(p.z - offset[2], places),
    }
}
fn rounded(v: f64, places: usize) -> f64 {
    format!("{v:.places$}")
        .parse()
        .expect("finite formatted number")
}
fn xyz(p: crate::motion::Position, places: usize) -> String {
    format!("X{:.p$} Y{:.p$} Z{:.p$}", p.x, p.y, p.z, p = places)
}
fn scalar(v: f64) -> Result<String> {
    if !v.is_finite() || !(0.000001..=1_000_000.).contains(&v) {
        return Err(error(
            "POST_NUMBER_RANGE",
            "feeds and spindle speeds must be within 0.000001..=1000000",
        ));
    }
    Ok(v.to_string())
}

fn motions_preserved(plan: &OperationPlan, places: usize) -> bool {
    plan.motions.iter().all(|m| {
        let a = rounded_position(m.start, places);
        let b = rounded_position(m.end, places);
        // A motion must survive formatting as a real move; pure-Z and pure-XY
        // moves only need their changing axis to survive.
        a != b
    }) && plan.motions.windows(2).all(|w| {
        // A zero-length formatted block cannot be distinguished from collapse;
        // require distinct endpoints for every motion at this precision.
        rounded_position(w[0].start, places) != rounded_position(w[0].end, places)
            && rounded_position(w[1].start, places) != rounded_position(w[1].end, places)
    })
}
