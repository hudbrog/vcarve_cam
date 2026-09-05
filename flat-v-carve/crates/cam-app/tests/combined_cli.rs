use serde_json::Value;
use std::{fs, path::PathBuf, process::Command};
struct Scratch(PathBuf);
impl Scratch {
    fn new(id: &str) -> Self {
        let p = std::env::temp_dir().join(format!("cam-m4-{id}-{}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn cam() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cam"))
}
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/m4/{name}.json"))
}
fn json(p: &PathBuf) -> Value {
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}
#[test]
fn combined_plan_replays_embedded_job_and_endmill_only_mode_remains_available() {
    let s = Scratch::new("portable");
    let job = s.0.join("job.json");
    let plan = s.0.join("plan.json");
    let rough = s.0.join("rough.json");
    let svg = s.0.join("preview.svg");
    let report = s.0.join("report.json");
    fs::copy(fixture("narrow-channel"), &job).unwrap();
    let out = cam()
        .arg("plan")
        .arg(&job)
        .arg("--output")
        .arg(&plan)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let data = json(&plan);
    assert_eq!(data["artifact_kind"], "combined_plan");
    assert_eq!(data["endmill"]["analysis"]["status"], "empty");
    assert!(!data["vbit_motions"].as_array().unwrap().is_empty());
    let out = cam()
        .arg("plan")
        .arg(&job)
        .args(["--stage", "endmill", "--output"])
        .arg(&rough)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(json(&rough)["artifact_kind"], "endmill_plan");
    fs::remove_file(job).unwrap();
    let out = cam()
        .arg("inspect")
        .arg(&plan)
        .arg("--output")
        .arg(&svg)
        .arg("--report")
        .arg(&report)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let preview = fs::read_to_string(svg).unwrap();
    assert!(preview.contains("M4 combined stage: Complete"));
    assert!(preview.contains("Sampled residual depth"));
    assert!(preview.contains("Final finish:"));
    let out = cam()
        .arg("verify")
        .arg(&plan)
        .arg("--output")
        .arg(&report)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(json(&report)["analysis"], data["analysis"]);
}
#[test]
fn partial_combined_plans_and_invalid_zero_ridge_requests_have_explicit_outputs() {
    let s = Scratch::new("status");
    let job = s.0.join("job.json");
    let plan = s.0.join("plan.json");
    let report = s.0.join("report.json");
    let mut data = json(&fixture("narrow-channel"));
    data["vbit_planning"]["max_motions"] = serde_json::json!(1);
    fs::write(&job, data.to_string()).unwrap();
    let out = cam()
        .arg("plan")
        .arg(&job)
        .args(["--stage", "combined", "--output"])
        .arg(&plan)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(json(&plan)["artifact_kind"], "combined_plan");
    assert_eq!(json(&plan)["analysis"]["status"], "inconclusive");
    let out = cam()
        .arg("verify")
        .arg(&plan)
        .arg("--output")
        .arg(&report)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(json(&report)["analysis"]["status"], "inconclusive");
    let out = cam()
        .arg("plan")
        .arg(fixture("zero-ridge"))
        .arg("--output")
        .arg(&plan)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        json(&plan)["diagnostics"][0]["code"],
        "ZERO_RIDGE_AREA_CLEARING"
    );
    assert_eq!(json(&plan)["valid"], false);
    let out = cam()
        .arg("plan")
        .arg(&job)
        .args(["--stage", "vbit", "--output"])
        .arg(&plan)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}
#[test]
fn stale_combined_motions_invalidate_preview_while_cached_reports_are_ignored() {
    let s = Scratch::new("stale");
    let plan = s.0.join("plan.json");
    let svg = s.0.join("preview.svg");
    let report = s.0.join("report.json");
    assert!(
        cam()
            .arg("plan")
            .arg(fixture("narrow-channel"))
            .arg("--output")
            .arg(&plan)
            .output()
            .unwrap()
            .status
            .success()
    );
    let mut data = json(&plan);
    data["analysis"] = serde_json::json!({"status":"empty"});
    fs::write(&plan, data.to_string()).unwrap();
    assert!(
        cam()
            .arg("inspect")
            .arg(&plan)
            .arg("--output")
            .arg(&svg)
            .arg("--report")
            .arg(&report)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(json(&report)["analysis"]["status"], "complete");
    data["vbit_motions"][0]["end"]["x"] = serde_json::json!(123.);
    fs::write(&plan, data.to_string()).unwrap();
    let out = cam()
        .arg("inspect")
        .arg(&plan)
        .arg("--output")
        .arg(&svg)
        .arg("--report")
        .arg(&report)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(json(&report)["diagnostics"][0]["code"], "STALE_PLAN");
    assert!(
        fs::read_to_string(svg)
            .unwrap()
            .contains("Job preview unavailable")
    );
}
