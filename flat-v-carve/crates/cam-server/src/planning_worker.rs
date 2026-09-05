//! Private IPC for cam-web's disposable compute process. No shell or user paths.
use crate::document::{ENGINE_VERSION, JOB_BYTES, UiDiagnostic};
use crate::inspection::Inspection;
use cam_core::{job::Job, motion::Motion, pocket::plan_endmill, vcarve::plan_combined};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, Read, Write};

pub const PREVIEW_MOTIONS: usize = 20_000;
pub const ARTIFACT_BYTES: usize = 16_000_000;
pub const WORKER_BYTES: usize = 32_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Endmill,
    Combined,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub stage: Stage,
    pub job: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Output {
    pub summary: Value,
    pub motions: Vec<Motion>,
    pub artifact: String,
    pub inspection: Inspection,
}

struct BoundedWriter(Vec<u8>, usize);
impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len() + bytes.len() > self.1 {
            return Err(io::Error::other(
                "Planning result exceeds the local service size limit",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn artifact(plan: &impl Serialize) -> Result<String, String> {
    let mut writer = BoundedWriter(Vec::new(), ARTIFACT_BYTES);
    serde_json::to_writer(&mut writer, plan).map_err(|e| e.to_string())?;
    String::from_utf8(writer.0).map_err(|e| e.to_string())
}

pub fn calculate(input: Input) -> Result<Output, Value> {
    let job = Job::from_json(&input.job).map_err(|d| json!(UiDiagnostic::from(d)))?;
    // Readiness is decided by the selected core planner, including its stage-specific
    // settings checks. Editable-job validation alone never implies readiness.
    let (mut summary, motions, artifact, inspection) = match input.stage {
        Stage::Endmill => {
            let plan = plan_endmill(&job).map_err(|d| json!(UiDiagnostic::from(d)))?;
            let summary = json!({
                "status": plan.analysis.status, "inputFingerprint": plan.input_fingerprint,
                "motionFingerprint": plan.motion_fingerprint, "meaning": plan.analysis.meaning,
                "limitations": plan.analysis.limitations,
                "diagnostics": plan.analysis.diagnostics.iter().take(100).cloned().map(UiDiagnostic::from).collect::<Vec<_>>(),
                "omittedDiagnostics": plan.analysis.diagnostics.len().saturating_sub(100),
                "generationIssues": plan.generation_issues.iter().take(100).collect::<Vec<_>>(),
                "omittedGenerationIssues": plan.generation_issues.len().saturating_sub(100),
            });
            let artifact = artifact(&plan).map_err(resource_error)?;
            let inspection = Inspection::endmill(&plan);
            (summary, plan.motions, artifact, inspection)
        }
        Stage::Combined => {
            let plan = plan_combined(&job).map_err(|d| json!(UiDiagnostic::from(d)))?;
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
            let artifact = artifact(&plan).map_err(resource_error)?;
            let inspection = Inspection::combined(&plan);
            let motions = plan
                .endmill
                .motions
                .into_iter()
                .chain(plan.vbit_motions)
                .collect();
            (summary, motions, artifact, inspection)
        }
    };
    summary["engineVersion"] = json!(ENGINE_VERSION);
    summary["motionCount"] = json!(motions.len());
    summary["cuttingMotionCount"] = json!(motions.iter().filter(|m| m.kind.cutting()).count());
    summary["previewMotionCount"] = json!(motions.len().min(PREVIEW_MOTIONS));
    summary["omittedMotionCount"] = json!(motions.len().saturating_sub(PREVIEW_MOTIONS));
    Ok(Output {
        summary,
        motions: motions.into_iter().take(PREVIEW_MOTIONS).collect(),
        artifact,
        inspection,
    })
}
fn resource_error(message: String) -> Value {
    json!({"code": "PLAN_RESULT_LIMIT", "severity": "error", "stage": "planning", "message": message})
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    io::stdin()
        .take((JOB_BYTES * 2 + 1024) as u64)
        .read_to_end(&mut bytes)?;
    let input: Input = serde_json::from_slice(&bytes)?;
    let mut reply = calculate(input);
    let mut writer = BoundedWriter(Vec::new(), WORKER_BYTES);
    if serde_json::to_writer(&mut writer, &reply).is_err() {
        // A large display must not discard an otherwise usable artifact.
        if let Ok(result) = &mut reply {
            result.inspection.omit_geometry();
        }
    } else {
        io::stdout().write_all(&writer.0)?;
        return Ok(());
    }
    writer.0.clear();
    if let Err(error) = serde_json::to_writer(&mut writer, &reply) {
        writer.0.clear();
        serde_json::to_writer(
            &mut writer,
            &Result::<Output, Value>::Err(resource_error(error.to_string())),
        )?;
    }
    io::stdout().write_all(&writer.0)?;
    Ok(())
}
