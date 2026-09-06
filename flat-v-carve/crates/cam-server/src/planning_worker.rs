//! Private IPC for cam-web's disposable compute process. No shell or user paths.
use crate::artifact::PlanFile;
use crate::document::{ENGINE_VERSION, JOB_BYTES, UiDiagnostic};
use crate::inspection::Inspection;
use cam_core::{job::Job, motion::Motion, pocket::plan_endmill, vcarve::plan_combined};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::{self, BufWriter, Read, Write},
    path::PathBuf,
    sync::Arc,
};

pub const PREVIEW_MOTIONS: usize = crate::motion_preview::PAGE_MOTIONS;
pub const REPORT_BYTES: usize = 16_000_000;
// Only bounded display/report data crosses stdout. Complete plans stay on disk.
pub const WORKER_BYTES: usize = 32_000_000;
pub const WORKER_INPUT_BYTES: usize = JOB_BYTES * 2 + 100_000;

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
    pub verification: Option<crate::verification::Work>,
    pub export: Option<crate::exporting::Work>,
    // Created by the parent service, never supplied by an HTTP client.
    pub output_path: Option<PathBuf>,
    pub motion_output_path: Option<PathBuf>,
    // Keep a source alive while queued or running, even after ledger eviction.
    #[serde(skip)]
    pub source_artifact: Option<Arc<PlanFile>>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Output {
    pub summary: Value,
    pub motions: Vec<Motion>,
    pub artifact: String,
    // Installed by the parent after a successful worker exit. Not part of IPC.
    #[serde(skip)]
    pub plan_artifact: Option<Arc<PlanFile>>,
    #[serde(skip)]
    pub motion_artifact: Option<Arc<PlanFile>>,
    pub motion_pages: Vec<crate::motion_preview::Page>,
    pub programs: Vec<cam_core::post::Program>,
    pub inspection: Inspection,
}

struct BoundedWriter(Vec<u8>, usize);
impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.1.saturating_sub(self.0.len()) {
            return Err(io::Error::other(format!(
                "Serialized result exceeds the local service limit of {} bytes",
                self.1
            )));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn write_plan(path: &std::path::Path, plan: &impl Serialize) -> Result<(), String> {
    // The parent reserves this exact file with create_new and removes it if the
    // worker fails. Publication is the successful task installation, after flush.
    let file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let mut writer = BufWriter::with_capacity(1024 * 1024, file);
    serde_json::to_writer(&mut writer, plan).map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())
}

pub fn calculate(input: Input) -> Result<Output, Value> {
    if let Some(work) = input.export {
        return crate::exporting::calculate(work);
    }
    if let Some(work) = input.verification {
        return crate::verification::calculate(work);
    }
    let job = Job::from_json(&input.job).map_err(|d| json!(UiDiagnostic::from(d)))?;
    let output_path = input
        .output_path
        .ok_or_else(|| artifact_error("Missing service-owned plan file".into()))?;
    // Readiness is decided by the selected core planner, including its stage-specific
    // settings checks. Editable-job validation alone never implies readiness.
    let (mut summary, motions, inspection) = match input.stage {
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
            write_plan(&output_path, &plan).map_err(artifact_error)?;
            let inspection = Inspection::endmill(&plan);
            (summary, plan.motions, inspection)
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
            write_plan(&output_path, &plan).map_err(artifact_error)?;
            let inspection = Inspection::combined(&plan);
            let motions = plan
                .endmill
                .motions
                .into_iter()
                .chain(plan.vbit_motions)
                .collect();
            (summary, motions, inspection)
        }
    };
    summary["engineVersion"] = json!(ENGINE_VERSION);
    summary["motionCount"] = json!(motions.len());
    summary["cuttingMotionCount"] = json!(motions.iter().filter(|m| m.kind.cutting()).count());
    summary["previewMotionCount"] = json!(motions.len());
    summary["omittedMotionCount"] = json!(0);
    let motion_path = input
        .motion_output_path
        .ok_or_else(|| artifact_error("Missing service-owned motion preview file".into()))?;
    let motion_pages = crate::motion_preview::write(&motion_path, &motions)
        .map_err(|e| artifact_error(e.to_string()))?;
    Ok(Output {
        summary,
        motions: motions.into_iter().take(PREVIEW_MOTIONS).collect(),
        artifact: String::new(),
        plan_artifact: None,
        motion_artifact: None,
        motion_pages,
        inspection,
        programs: vec![],
    })
}
fn resource_error(message: String) -> Value {
    json!({"code": "PLAN_RESULT_LIMIT", "severity": "error", "stage": "planning", "message": message})
}
fn artifact_error(message: String) -> Value {
    json!({"code": "PLAN_ARTIFACT_IO", "severity": "error", "stage": "planning", "message": format!("Could not save the local plan artifact: {message}")})
}

fn read_input(reader: impl Read) -> io::Result<Input> {
    let mut bytes = Vec::new();
    reader
        .take(WORKER_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > WORKER_INPUT_BYTES {
        return Err(io::Error::other(format!(
            "Worker input exceeds the {WORKER_INPUT_BYTES} byte limit"
        )));
    }
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

fn reply_bytes(mut reply: Result<Output, Value>) -> io::Result<Vec<u8>> {
    let mut writer = BoundedWriter(Vec::new(), WORKER_BYTES);
    if serde_json::to_writer(&mut writer, &reply).is_err() {
        // A large display must not discard an otherwise usable artifact.
        if let Ok(result) = &mut reply {
            result.inspection.omit_geometry();
        }
    } else {
        return Ok(writer.0);
    }
    writer.0.clear();
    if let Err(error) = serde_json::to_writer(&mut writer, &reply) {
        writer.0.clear();
        serde_json::to_writer(
            &mut writer,
            &Result::<Output, Value>::Err(resource_error(error.to_string())),
        )
        .map_err(io::Error::other)?;
    }
    Ok(writer.0)
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let input = read_input(io::stdin().lock())?;
    let bytes = reply_bytes(calculate(input))?;
    io::stdout().lock().write_all(&bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_writer_accepts_the_limit_and_rejects_overflow_without_truncation() {
        let mut writer = BoundedWriter(Vec::new(), 4);
        writer.write_all(b"plan").unwrap();
        let error = writer.write_all(b"!").unwrap_err();
        assert!(error.to_string().contains("4 bytes"));
        assert_eq!(writer.0, b"plan");
    }

    #[test]
    fn large_artifacts_do_not_grow_worker_replies() {
        // Serialize more than the former 128 MB ceiling using repeated borrowed
        // chunks, without allocating a full-size JSON string even in this test.
        let artifact = PlanFile::create().unwrap();
        let chunk = "x".repeat(64 * 1024);
        write_plan(artifact.path(), &vec![chunk.as_str(); 2048]).unwrap();
        assert!(artifact.byte_len().unwrap() > 128_000_000);
        let result = Output {
            summary: json!({"status": "complete"}),
            motions: vec![],
            artifact: String::new(),
            plan_artifact: Some(artifact.clone()),
            motion_artifact: None,
            motion_pages: vec![],
            programs: vec![],
            inspection: Default::default(),
        };
        let bytes = reply_bytes(Ok(result)).unwrap();
        assert!(bytes.len() < 1024);
        let reply: Result<Output, Value> = serde_json::from_slice(&bytes).unwrap();
        let reply = reply.unwrap();
        assert!(reply.artifact.is_empty());
        assert!(reply.plan_artifact.is_none());
        assert_eq!(reply.summary["status"], "complete");
        assert!(artifact.path().exists());
    }

    #[test]
    fn verification_input_contains_a_path_without_plan_bytes_or_ownership() {
        let artifact = PlanFile::create().unwrap();
        std::fs::write(artifact.path(), "private plan data").unwrap();
        let input = Input {
            stage: Stage::Combined,
            job: String::new(),
            verification: Some(crate::verification::Work {
                artifact: artifact.path().to_owned(),
                identity: crate::verification::Identity {
                    plan_task_id: "a".repeat(128),
                    input_fingerprint: "b".repeat(64),
                    motion_fingerprint: "c".repeat(64),
                    options: Default::default(),
                },
            }),
            export: None,
            output_path: None,
            motion_output_path: None,
            source_artifact: Some(artifact.clone()),
        };
        let bytes = serde_json::to_vec(&input).unwrap();
        assert!(bytes.len() < 2048);
        assert!(!String::from_utf8_lossy(&bytes).contains("private plan data"));
        let decoded = read_input(bytes.as_slice()).unwrap();
        assert!(decoded.source_artifact.is_none());
        assert_eq!(
            decoded.verification.unwrap().identity.plan_task_id,
            "a".repeat(128)
        );
    }

    #[test]
    fn missing_output_file_is_an_io_failure_without_a_successful_reply() {
        let error =
            write_plan(std::path::Path::new("missing-parent/plan.json"), &json!({})).unwrap_err();
        assert_eq!(artifact_error(error)["code"], "PLAN_ARTIFACT_IO");
    }
}
