//! M6 linear LinuxCNC output, with explicit machine contracts and numeric
//! readback. No filesystem, machine connection, or arbitrary G-code templates.
mod profile;
mod reader;
pub use profile::*;

use crate::{
    geometry::{Diagnostic, Result},
    motion::{Motion, MotionKind, Position},
    vcarve::{AuthenticatedPlan, CombinedPlan},
    verification::{
        StockVerification, VerificationOptions, VerificationReport, VerificationStatus,
        verify_authenticated_plan, verify_emitted_motions,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramLayout {
    Combined,
    PerTool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Program {
    pub filename: String,
    pub gcode: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct ProgramEvidence {
    pub filename: String,
    pub sha256: String,
    pub motion_count: usize,
    pub clearance_link_count: usize,
    /// Post-M6 positioning with an initially unknown position. Its safety is
    /// established by the declared machine contract, outside M5 stock bounds.
    pub contract_positioning_blocks: usize,
    pub tool_changes: usize,
    pub prerequisites: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ExportReport {
    pub artifact_kind: String,
    pub schema_version: u32,
    pub engine_version: String,
    pub status: VerificationStatus,
    pub profile: LinuxCncProfile,
    pub profile_fingerprint: String,
    pub layout: ProgramLayout,
    pub machine_z_offset_mm: f64,
    pub plan_verification: VerificationReport,
    pub emitted_verification: Option<StockVerification>,
    pub emitted_motion_fingerprint: Option<String>,
    pub programs: Vec<ProgramEvidence>,
    pub diagnostics: Vec<Diagnostic>,
    pub limitations: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct ExportResult {
    /// Empty whenever verification fails or is inconclusive.
    pub programs: Vec<Program>,
    pub report: ExportReport,
}
pub(super) fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("postprocessor")
}
fn hash(v: &impl Serialize) -> Result<String> {
    crate::plan_hash::hash(v).map_err(|e| error("POST_JSON", e.to_string()))
}
pub(super) fn rounded(v: f64, places: usize) -> f64 {
    format!("{v:.places$}")
        .parse()
        .expect("finite formatted number")
}
pub(super) fn machine_position(p: Position, profile: &LinuxCncProfile, offset: f64) -> Position {
    Position {
        x: rounded(p.x, profile.decimal_places),
        y: rounded(p.y, profile.decimal_places),
        z: rounded(p.z + offset, profile.decimal_places),
    }
}
fn stock_position(p: Position, offset: f64) -> Position {
    Position {
        z: p.z - offset,
        ..p
    }
}
fn xyz(p: Position, places: usize) -> String {
    format!("X{:.places$} Y{:.places$} Z{:.places$}", p.x, p.y, p.z)
}
fn scalar(v: f64) -> Result<String> {
    // Preserve feeds exactly, independently of coordinate precision.
    if !v.is_finite() || !(0.000001..=1_000_000.).contains(&v) {
        return Err(error(
            "POST_NUMBER_RANGE",
            "feeds and spindle speeds must be within 0.000001..=1000000",
        ));
    }
    Ok(v.to_string())
}
pub(super) struct Stage<'a> {
    pub role: &'static str,
    pub id: &'a str,
    pub motions: &'a [Motion],
    pub spindle: f64,
}
struct ProgramReadback {
    programs: Vec<Program>,
    endmill: Vec<Motion>,
    vbit: Vec<Motion>,
    evidence: Vec<ProgramEvidence>,
}
fn stages(plan: &CombinedPlan) -> Vec<Stage<'_>> {
    [
        Stage {
            role: "endmill",
            id: &plan.endmill.job.operation.endmill_id,
            motions: &plan.endmill.motions,
            spindle: plan.endmill.spindle_rpm,
        },
        Stage {
            role: "vbit",
            id: &plan.endmill.job.operation.vbit_id,
            motions: &plan.vbit_motions,
            spindle: plan.vbit_spindle_rpm,
        },
    ]
    .into_iter()
    .filter(|s| !s.motions.is_empty())
    .collect()
}
fn modal(lines: &mut Vec<String>, p: &LinuxCncProfile) {
    lines.extend([
        "G21 G17 G90 G94 G40 G80 G61".into(),
        p.work_offset.clone(),
        "G92.1".into(),
    ]);
}
fn emit(
    plan: &CombinedPlan,
    p: &LinuxCncProfile,
    stages: &[&Stage<'_>],
    filename: &str,
    offset: f64,
) -> Result<Program> {
    let mut lines = vec![
        "(CAM Linear LinuxCNC program; read export-report.json before use)".into(),
        format!(
            "(CAM Engine {}; profile {})",
            env!("CARGO_PKG_VERSION"),
            p.id
        ),
        format!("(CAM Plan {})", plan.motion_fingerprint),
        "(CAM Start ready for M6 under the profile startup and clearance contract)".into(),
    ];
    if filename == "vbit.ngc" && !plan.endmill.motions.is_empty() {
        lines.push(
            "(CAM Prerequisite: endmill.ngc from this export must already have run on this stock)"
                .into(),
        );
    }
    lines.push("M5".into());
    lines.push("M9".into());
    modal(&mut lines, p);
    let mut previous_stage_end = p.program_start_position_mm;
    for stage in stages {
        let tool = p.tool(stage.id);
        lines.push(format!("(CAM Stage {}; tool {})", stage.role, stage.id));
        lines.push("M5".into());
        lines.push("M9".into());
        lines.push(format!("T{} M6", tool.tool_number));
        lines.push("M5".into());
        lines.push("M9".into());
        modal(&mut lines, p);
        if p.length_compensation == LengthCompensation::ToolTable {
            lines.push(format!("G43 H{}", tool.length_offset_number.unwrap()));
        }
        // The contract applies after modal restoration and (if post-managed)
        // G43. Never infer the new tool-tip position from the old tool length.
        let mut current = p.returned_position(previous_stage_end);
        if let M6Return::SafeRetract {
            z_mm,
            transit_xy_mm,
        } = p.m6.return_position
        {
            lines.push(format!("G0 Z{z_mm:.places$}", places = p.decimal_places));
            lines.push(format!(
                "G0 {}",
                xyz(Position::new(transit_xy_mm, z_mm), p.decimal_places)
            ));
        }
        let start = machine_position(stage.motions[0].start, p, offset);
        if current.z != start.z {
            lines.push(format!(
                "G0 Z{:.places$}",
                start.z,
                places = p.decimal_places
            ));
            current.z = start.z;
        }
        if current != start {
            lines.push(format!("G0 {}", xyz(start, p.decimal_places)));
        }
        lines.push(format!("G97 S{}", scalar(stage.spindle)?));
        lines.push(
            match tool.spindle_direction {
                SpindleDirection::Clockwise => "M3",
                SpindleDirection::Counterclockwise => "M4",
            }
            .into(),
        );
        lines.push(format!("G4 P{}", p.spindle_spinup_seconds));
        lines.push(
            match p.coolant {
                Coolant::Off => "M9",
                Coolant::Flood => "M8",
                Coolant::Mist => "M7",
            }
            .into(),
        );
        for m in stage.motions {
            let end = machine_position(m.end, p, offset);
            let feed = match m.feed_mm_min {
                Some(v) => format!(" F{}", scalar(v)?),
                None => String::new(),
            };
            lines.push(format!(
                "{} {}{}",
                if m.kind.rapid() { "G0" } else { "G1" },
                xyz(end, p.decimal_places),
                feed
            ));
            current = end;
        }
        previous_stage_end = Some(current);
    }
    lines.extend(["M5".into(), "M9".into(), "M2".into()]);
    if lines.iter().any(|l| l.len() > 240) {
        return Err(error(
            "POST_LINE_LENGTH",
            "an emitted block exceeds the 240-character limit",
        ));
    }
    Ok(Program {
        filename: filename.into(),
        gcode: lines.join("\n") + "\n",
    })
}

/// Authenticate and verify the original plan, emit, independently read the
/// numeric subset, compare every reconstructed segment, and verify actual
/// output coordinates in stock-top space. No failed output program is returned.
pub fn export_plan(
    plan: &CombinedPlan,
    profile: &LinuxCncProfile,
    layout: ProgramLayout,
    options: &VerificationOptions,
) -> Result<ExportResult> {
    export_authenticated_plan(
        &AuthenticatedPlan::from_plan(plan)?,
        profile,
        layout,
        options,
    )
}

/// Export a freshly authenticated immutable plan without JSON round-trips.
pub fn export_authenticated_plan(
    plan: &AuthenticatedPlan,
    profile: &LinuxCncProfile,
    layout: ProgramLayout,
    options: &VerificationOptions,
) -> Result<ExportResult> {
    process(plan, profile, layout, options, None)
}
/// Recheck saved output bytes against the plan/profile. Comments never supply
/// motion metadata or authority; stage ordering and every block are checked.
pub fn verify_programs(
    plan: &CombinedPlan,
    profile: &LinuxCncProfile,
    layout: ProgramLayout,
    options: &VerificationOptions,
    programs: &[Program],
) -> Result<ExportReport> {
    Ok(process(
        &AuthenticatedPlan::from_plan(plan)?,
        profile,
        layout,
        options,
        Some(programs),
    )?
    .report)
}
fn process(
    authenticated: &AuthenticatedPlan,
    profile: &LinuxCncProfile,
    layout: ProgramLayout,
    options: &VerificationOptions,
    supplied: Option<&[Program]>,
) -> Result<ExportResult> {
    options.validate()?;
    if options
        .decimal_places
        .is_some_and(|n| n != profile.decimal_places)
    {
        return Err(error(
            "POST_PRECISION",
            "output precision is owned by the machine profile",
        ));
    }
    let plan = authenticated.plan();
    profile.validate(&plan.endmill.job)?;
    let offset = profile.z_offset(&plan.endmill.job)?;
    let mut original_options = options.clone();
    // Output rounding must happen AFTER stock-datum translation. Ordinary M5
    // stock-top decimal rounding is not interchangeable at rounding ties.
    original_options.decimal_places = None;
    let original = verify_authenticated_plan(authenticated, &original_options)?;
    let mut report = ExportReport {
        artifact_kind: "linuxcnc_export_report".into(), schema_version: 1,
        engine_version: env!("CARGO_PKG_VERSION").into(), status: original.status,
        profile: profile.clone(), profile_fingerprint: hash(profile)?, layout,
        machine_z_offset_mm: offset, plan_verification: original,
        emitted_verification: None, emitted_motion_fingerprint: None, programs: vec![], diagnostics: vec![],
        limitations: vec![
            "Acceptance covers authenticated modeled stock and the exact emitted linear subset under the declared machine contract; it is not LinuxCNC simulation or physical-machine approval.".into(),
            "Operator startup position, active current-tool compensation, homing, stock datum, table height, zero XY rotation, safe clearance across fixtures, and M6 return behavior require machine setup. Hidden macro/probing motion and tool-table contents are not interpreted.".into(),
            "Per-tool files restore all required modes independently; V-bit rest machining still requires the matching endmill stock history. No G28/G30/G53 or probing positions are invented.".into(),
        ],
    };
    if report.status != VerificationStatus::Passed {
        return Ok(ExportResult {
            programs: vec![],
            report,
        });
    }
    let all_stages = stages(plan);
    if all_stages.is_empty() {
        return Err(error("POST_EMPTY", "no executable motions"));
    }
    let groups: Vec<(String, Vec<&Stage<'_>>)> = match layout {
        ProgramLayout::Combined => vec![("combined.ngc".into(), all_stages.iter().collect())],
        ProgramLayout::PerTool => all_stages
            .iter()
            .map(|s| (format!("{}.ngc", s.role), vec![s]))
            .collect(),
    };
    let result = (|| -> Result<ProgramReadback> {
        if supplied.is_some_and(|s| s.len() != groups.len()) {
            return Err(error(
                "POST_PROGRAM_SET",
                "program count differs from selected layout and nonempty stages",
            ));
        }
        let mut programs = vec![];
        let mut endmill = vec![];
        let mut vbit = vec![];
        let mut evidence = vec![];
        for (i, (filename, group)) in groups.iter().enumerate() {
            let program = match supplied {
                Some(s) => s[i].clone(),
                None => emit(plan, profile, group, filename, offset)?,
            };
            if &program.filename != filename {
                return Err(error(
                    "POST_PROGRAM_SET",
                    "program filenames/order differ from the expected layout",
                ));
            }
            let read = reader::read(&program.gcode, profile, group, offset)?;
            evidence.push(ProgramEvidence {
                filename: filename.clone(),
                sha256: format!("{:x}", Sha256::digest(program.gcode.as_bytes())),
                motion_count: read.motions.len(),
                clearance_link_count: read.clearance_links,
                contract_positioning_blocks: if matches!(
                    profile.m6.return_position,
                    M6Return::SafeRetract { .. }
                ) {
                    2 * group.len()
                } else {
                    0
                },
                tool_changes: group.len(),
                prerequisites: if filename == "vbit.ngc" && !plan.endmill.motions.is_empty() {
                    vec!["Run endmill.ngc from this exact export on the same stock first.".into()]
                } else {
                    vec![]
                },
            });
            for m in read.motions {
                if m.tool_id == plan.endmill.job.operation.endmill_id {
                    endmill.push(m);
                } else {
                    vbit.push(m);
                }
            }
            programs.push(program);
        }
        Ok(ProgramReadback {
            programs,
            endmill,
            vbit,
            evidence,
        })
    })();
    let ProgramReadback {
        programs,
        endmill,
        vbit,
        evidence,
    } = match result {
        Ok(v) => v,
        Err(d) => {
            report.status = VerificationStatus::Failed;
            report.diagnostics.push(d);
            return Ok(ExportResult {
                programs: vec![],
                report,
            });
        }
    };
    let s = plan.endmill.job.endmill_planning.as_ref().unwrap();
    let start = stock_position(
        machine_position(
            Position::new(s.start_xy_mm, s.clearance_z_mm),
            profile,
            offset,
        ),
        offset,
    );
    let emitted =
        verify_emitted_motions(&plan.endmill.job, &endmill, &vbit, &original_options, start)?;
    report.status = emitted.status;
    report.emitted_motion_fingerprint = Some(hash(&(&endmill, &vbit))?);
    report.emitted_verification = Some(emitted);
    report.programs = evidence;
    Ok(ExportResult {
        programs: if report.status == VerificationStatus::Passed {
            programs
        } else {
            vec![]
        },
        report,
    })
}
