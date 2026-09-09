//! Basic automatic plan checks (plan section 14.2).
//!
//! These are bounded structural/numeric checks over the produced motions and
//! execution items — deliberately separate from detailed stock-quality
//! analysis (M5), which stays optional and operation-specific. A plan that
//! fails a basic check cannot be exported; an unrun detailed analysis cannot
//! block generation, and a passing one cannot authorize a broken plan.
use crate::{
    geometry::{Diagnostic, Result},
    sequence::{ExecutionItem, GenerationStatus, OperationPlan, ProcessSpindle, StageRole},
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

    // Execution accounting: every nonempty stage runs exactly once, tool
    // changes precede the run, and each stage's intent covers its motion.
    let mut expected_stage = 0usize;
    let mut iter = plan.execution.iter();
    while expected_stage < plan.stages.len() {
        let stage = &plan.stages[expected_stage];
        match iter.next() {
            Some(ExecutionItem::ToolChange { tool_id }) => {
                if tool_id != &stage.tool_id {
                    let mut f = finding(
                        "PLAN_EXECUTION_TOOL",
                        format!(
                            "tool change to '{tool_id}' does not match stage '{}'",
                            stage.stage_id
                        ),
                    );
                    f.stage_id = Some(stage.stage_id.clone());
                    findings.push(f);
                    failed = true;
                }
                match iter.next() {
                    Some(ExecutionItem::SetProcessIntent { intent }) => {
                        if matches!(intent.spindle, ProcessSpindle::Off)
                            && !matches!(stage.role, StageRole::Knife)
                        {
                            let mut f = finding(
                                "PROCESS_SPINDLE_STATE",
                                format!("milling stage '{}' requests spindle off", stage.stage_id),
                            );
                            f.stage_id = Some(stage.stage_id.clone());
                            findings.push(f);
                            failed = true;
                        }
                    }
                    other => {
                        let mut f = finding(
                            "PLAN_EXECUTION_ORDER",
                            format!(
                                "stage '{}' lacks its process intent before running",
                                stage.stage_id
                            ),
                        );
                        f.stage_id = Some(stage.stage_id.clone());
                        findings.push(f);
                        failed = true;
                        if other.is_none() {
                            break;
                        }
                        continue;
                    }
                }
                match iter.next() {
                    Some(ExecutionItem::RunStage { stage_id }) if stage_id == &stage.stage_id => {}
                    other => {
                        let mut f = finding(
                            "PLAN_EXECUTION_ORDER",
                            format!(
                                "expected run of stage '{}', found {:?}",
                                stage.stage_id,
                                other.map(|i| match i {
                                    ExecutionItem::ToolChange { tool_id } =>
                                        format!("tool change {tool_id}"),
                                    ExecutionItem::SetProcessIntent { .. } => "intent".into(),
                                    ExecutionItem::RunStage { stage_id } =>
                                        format!("run {stage_id}"),
                                })
                            ),
                        );
                        f.stage_id = Some(stage.stage_id.clone());
                        findings.push(f);
                        failed = true;
                        if other.is_none() {
                            break;
                        }
                        continue;
                    }
                }
            }
            Some(ExecutionItem::SetProcessIntent { .. }) | Some(ExecutionItem::RunStage { .. }) => {
                let mut f = finding(
                    "PLAN_EXECUTION_ORDER",
                    format!("stage '{}' must begin with its tool change", stage.stage_id),
                );
                f.stage_id = Some(stage.stage_id.clone());
                findings.push(f);
                failed = true;
                continue;
            }
            None => {
                let mut f = finding(
                    "PLAN_EXECUTION_INCOMPLETE",
                    format!("stage '{}' never runs", stage.stage_id),
                );
                f.stage_id = Some(stage.stage_id.clone());
                findings.push(f);
                failed = true;
                break;
            }
        }
        expected_stage += 1;
    }
    if iter.next().is_some() {
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
