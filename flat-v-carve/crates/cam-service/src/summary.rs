//! Plan summaries projected from engine analysis; identical bytes for every
//! service adapter because the UI parses one schema.
use crate::document::{ENGINE_VERSION, UiDiagnostic};
use cam_core::{motion::Motion, pocket::EndmillPlan, vcarve::CombinedPlan};
use serde_json::{Value, json};

fn finish(mut summary: Value, motions: usize, cutting: usize) -> Value {
    summary["engineVersion"] = json!(ENGINE_VERSION);
    summary["motionCount"] = json!(motions);
    summary["cuttingMotionCount"] = json!(cutting);
    summary["previewMotionCount"] = json!(motions);
    summary["omittedMotionCount"] = json!(0);
    summary
}

/// Endmill-only summary; the motion counts cover the recorded endmill pass.
pub fn endmill(plan: &EndmillPlan) -> Value {
    let summary = json!({
        "status": plan.analysis.status, "inputFingerprint": plan.input_fingerprint,
        "motionFingerprint": plan.motion_fingerprint, "meaning": plan.analysis.meaning,
        "limitations": plan.analysis.limitations,
        "diagnostics": plan.analysis.diagnostics.iter().take(100).cloned().map(UiDiagnostic::from).collect::<Vec<_>>(),
        "omittedDiagnostics": plan.analysis.diagnostics.len().saturating_sub(100),
        "generationIssues": plan.generation_issues.iter().take(100).collect::<Vec<_>>(),
        "omittedGenerationIssues": plan.generation_issues.len().saturating_sub(100),
    });
    finish(
        summary,
        plan.motions.len(),
        plan.motions.iter().filter(|m| m.kind.cutting()).count(),
    )
}

/// Combined summary merging both stages' diagnostics and issues in stage order.
pub fn combined(plan: &CombinedPlan) -> Value {
    let diagnostics = plan
        .endmill
        .analysis
        .diagnostics
        .iter()
        .chain(&plan.analysis.diagnostics)
        .collect::<Vec<_>>();
    let issues = plan
        .endmill
        .generation_issues
        .iter()
        .chain(&plan.generation_issues)
        .collect::<Vec<_>>();
    let summary = json!({
        "status": plan.analysis.status, "inputFingerprint": plan.input_fingerprint,
        "motionFingerprint": plan.motion_fingerprint, "meaning": plan.analysis.meaning,
        "limitations": plan.analysis.limitations.iter().chain(&plan.endmill.analysis.limitations).collect::<Vec<_>>(),
        "diagnostics": diagnostics.iter().take(100).map(|d| UiDiagnostic::from((*d).clone())).collect::<Vec<_>>(),
        "omittedDiagnostics": diagnostics.len().saturating_sub(100),
        "generationIssues": issues.iter().take(100).collect::<Vec<_>>(),
        "omittedGenerationIssues": issues.len().saturating_sub(100),
    });
    let motions: Vec<&Motion> = plan
        .endmill
        .motions
        .iter()
        .chain(&plan.vbit_motions)
        .collect();
    finish(
        summary,
        motions.len(),
        motions.iter().filter(|m| m.kind.cutting()).count(),
    )
}
