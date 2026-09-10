//! Basic automatic plan checks (plan section 14.2).
//!
//! These are bounded structural/numeric checks over the produced motions and
//! execution items — deliberately separate from detailed stock-quality
//! analysis (M5), which stays optional and operation-specific. A plan that
//! fails a basic check cannot be exported; an unrun detailed analysis cannot
//! block generation, and a passing one cannot authorize a broken plan.
use crate::{
    geometry::{Diagnostic, Result},
    sequence::{
        ExecutionItem, ExecutionStage, GenerationStatus, OperationPlan, ProcessSpindle, StageRole,
    },
    toolpath::{Interpolation, MotionEffect},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Passed,
    Failed,
    Inconclusive,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CheckFinding {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct BasicCheckReport {
    pub status: CheckStatus,
    pub findings: Vec<CheckFinding>,
    /// True when export may proceed once process state is prepared.
    pub export_ready: bool,
}

fn finding(code: &str, message: impl Into<String>) -> CheckFinding {
    CheckFinding {
        code: code.into(),
        message: message.into(),
        operation_id: None,
        stage_id: None,
    }
}

impl BasicCheckReport {
    /// First blocking finding when the report failed.
    pub fn first_failure(&self) -> Option<&CheckFinding> {
        if self.status == CheckStatus::Failed {
            self.findings.first()
        } else {
            None
        }
    }
}

/// One expected execution step during the ordered walk.
#[derive(Debug)]
enum Expected<'a> {
    ToolChange(&'a str, &'a ExecutionStage),
    Intent(&'a ExecutionStage),
    Run(&'a ExecutionStage),
}

/// Run the automatic basic checks over a finished plan.
pub fn check_plan(plan: &OperationPlan) -> Result<BasicCheckReport> {
    let mut findings = vec![];
    let mut failed = false;
    macro_rules! fail {
        ($code:expr, $message:expr) => {{
            findings.push(finding($code, $message));
            failed = true;
        }};
    }

    // Generation completeness: incomplete or inconclusive operations block
    // export of this sequence but the report itself stays deterministic.
    for result in &plan.operation_results {
        match result.generation_status {
            GenerationStatus::Complete | GenerationStatus::Empty => {}
            GenerationStatus::Incomplete | GenerationStatus::Inconclusive => {
                let mut f = finding(
                    "PLAN_GENERATION_INCOMPLETE",
                    format!(
                        "operation '{}' did not generate completely; export is unavailable for this sequence",
                        result.operation_id
                    ),
                );
                f.operation_id = Some(result.operation_id.clone());
                findings.push(f);
                failed = true;
            }
        }
    }

    // Stage/motion ownership: dense motion ids, exactly one stage each,
    // stages ordered and contiguous.
    for (expected_id, motion) in plan.motions.iter().enumerate() {
        if motion.id != expected_id {
            fail!(
                "PLAN_MOTION_IDS",
                format!(
                    "motion id {} breaks the dense global ordering at {}",
                    motion.id, expected_id
                )
            );
            break;
        }
    }
    for (index, stage) in plan.stages.iter().enumerate() {
        if stage.motion_range.0 >= stage.motion_range.1 {
            let mut f = finding(
                "PLAN_STAGE_RANGE",
                format!(
                    "stage '{}' has an empty or inverted motion range",
                    stage.stage_id
                ),
            );
            f.stage_id = Some(stage.stage_id.clone());
            f.operation_id = Some(stage.operation_id.clone());
            findings.push(f);
            failed = true;
            continue;
        }
        if stage.motion_range.1 > plan.motions.len() {
            let mut f = finding(
                "PLAN_STAGE_RANGE",
                format!("stage '{}' extends past the motion array", stage.stage_id),
            );
            f.stage_id = Some(stage.stage_id.clone());
            findings.push(f);
            failed = true;
            continue;
        }
        if index > 0 && plan.stages[index - 1].motion_range.1 != stage.motion_range.0 {
            let mut f = finding(
                "PLAN_STAGE_ORDER",
                format!(
                    "stage '{}' does not continue where the previous stage ended",
                    stage.stage_id
                ),
            );
            f.stage_id = Some(stage.stage_id.clone());
            findings.push(f);
            failed = true;
        }
        for motion in &plan.motions[stage.motion_range.0..stage.motion_range.1] {
            if motion.stage_id != stage.stage_id || motion.tool_id != stage.tool_id {
                let mut f = finding(
                    "PLAN_STAGE_OWNERSHIP",
                    format!(
                        "motion {} does not belong to stage '{}'",
                        motion.id, stage.stage_id
                    ),
                );
                f.stage_id = Some(stage.stage_id.clone());
                findings.push(f);
                failed = true;
                break;
            }
        }
    }
    let covered: usize = plan
        .stages
        .iter()
        .map(|s| s.motion_range.1 - s.motion_range.0)
        .sum();
    if covered != plan.motions.len() {
        fail!(
            "PLAN_STAGE_COVERAGE",
            format!(
                "{} of {} motions are covered by stages",
                covered,
                plan.motions.len()
            )
        );
    }

    // Motion semantics: finite coordinates, positive feeds on linear feeds,
    // no feed on rapids, milling sweeps only with a milling stage role.
    for motion in &plan.motions {
        if !motion.start.finite() || !motion.end.finite() {
            let mut f = finding(
                "PLAN_MOTION_NUMERIC",
                format!("motion {} has non-finite coordinates", motion.id),
            );
            f.operation_id = Some(motion.operation_id.clone());
            findings.push(f);
            failed = true;
        }
        match motion.interpolation {
            Interpolation::LinearFeed => {
                if !motion.feed_mm_min.is_some_and(|f| f.is_finite() && f > 0.) {
                    let mut f = finding(
                        "PLAN_FEED_REQUIRED",
                        format!("linear feed motion {} lacks a positive feed", motion.id),
                    );
                    f.operation_id = Some(motion.operation_id.clone());
                    findings.push(f);
                    failed = true;
                }
            }
            Interpolation::Rapid => {
                if motion.feed_mm_min.is_some() {
                    let mut f = finding(
                        "PLAN_FEED_INVALID",
                        format!("rapid motion {} carries a feed", motion.id),
                    );
                    f.operation_id = Some(motion.operation_id.clone());
                    findings.push(f);
                    failed = true;
                }
            }
        }
        if motion.effect == MotionEffect::MillingSweep
            && !matches!(
                stage_role(plan, &motion.stage_id),
                Some(
                    StageRole::VcarveRough
                        | StageRole::VcarveFinish
                        | StageRole::Face
                        | StageRole::ProfileRough
                        | StageRole::ProfileFinish
                )
            )
        {
            let mut f = finding(
                "PLAN_EFFECT_ROLE_MISMATCH",
                format!("motion {} removes stock outside a milling stage", motion.id),
            );
            f.stage_id = Some(motion.stage_id.clone());
            findings.push(f);
            failed = true;
        }
    }

    // Execution accounting: every stage runs exactly once in order under its
    // own process intent. A tool change appears exactly when the selected
    // tool changes — consecutive same-tool stages continue without one, and
    // recurring tools are re-selected rather than regrouped.
    let mut expected: Vec<Expected<'_>> = vec![];
    let mut selected: Option<&str> = None;
    for stage in &plan.stages {
        if selected != Some(stage.tool_id.as_str()) {
            expected.push(Expected::ToolChange(&stage.tool_id, stage));
            selected = Some(stage.tool_id.as_str());
        }
        expected.push(Expected::Intent(stage));
        expected.push(Expected::Run(stage));
    }
    let mut actual = plan.execution.iter();
    for want in &expected {
        let Some(got) = actual.next() else {
            let mut f = finding(
                "PLAN_EXECUTION_INCOMPLETE",
                format!(
                    "execution ends before the ordered steps of stage '{}'",
                    stage_of(want).stage_id
                ),
            );
            f.stage_id = Some(stage_of(want).stage_id.clone());
            findings.push(f);
            failed = true;
            break;
        };
        let ok = match (want, got) {
            (Expected::ToolChange(tool, _), ExecutionItem::ToolChange { tool_id }) => {
                tool == tool_id
            }
            (Expected::Intent(stage), ExecutionItem::SetProcessIntent { intent }) => {
                if matches!(intent.spindle, ProcessSpindle::Off) && stage.role != StageRole::Knife {
                    let mut f = finding(
                        "PROCESS_SPINDLE_STATE",
                        format!("milling stage '{}' requests spindle off", stage.stage_id),
                    );
                    f.stage_id = Some(stage.stage_id.clone());
                    findings.push(f);
                    failed = true;
                }
                true
            }
            (Expected::Run(stage), ExecutionItem::RunStage { stage_id }) => {
                &stage.stage_id == stage_id
            }
            _ => false,
        };
        if !ok {
            let stage = stage_of(want);
            let mut f = finding(
                "PLAN_EXECUTION_ORDER",
                format!(
                    "execution step for stage '{}' does not match the ordered plan",
                    stage.stage_id
                ),
            );
            f.stage_id = Some(stage.stage_id.clone());
            findings.push(f);
            failed = true;
            break;
        }
    }
    if actual.next().is_some() {
        fail!(
            "PLAN_EXECUTION_EXTRA",
            "execution contains items after the last stage ran".to_string()
        );
    }

    let status = if failed {
        CheckStatus::Failed
    } else {
        CheckStatus::Passed
    };
    Ok(BasicCheckReport {
        status,
        findings,
        export_ready: status == CheckStatus::Passed,
    })
}

fn stage_of<'a>(expected: &'a Expected<'_>) -> &'a ExecutionStage {
    match expected {
        Expected::ToolChange(_, stage) | Expected::Intent(stage) | Expected::Run(stage) => stage,
    }
}

fn stage_role(plan: &OperationPlan, stage_id: &str) -> Option<StageRole> {
    plan.stages
        .iter()
        .find(|s| s.stage_id == stage_id)
        .map(|s| s.role)
}

/// Convenience wrapper used by export preparation: pass or surface the report.
pub fn require_pass(report: &BasicCheckReport) -> Result<()> {
    match report.status {
        CheckStatus::Passed => Ok(()),
        CheckStatus::Failed | CheckStatus::Inconclusive => Err(report
            .findings
            .first()
            .map(|f| Diagnostic::new("PLAN_BASIC_CHECKS", f.message.clone()).at_stage("checks"))
            .unwrap_or_else(|| {
                Diagnostic::new("PLAN_BASIC_CHECKS", "basic checks did not pass").at_stage("checks")
            })),
    }
}
