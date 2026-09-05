use cam_core::pocket::EndmillPlan;
use serde_json::Value;
use std::{fs, path::PathBuf, process::Command};
struct Scratch(PathBuf);
impl Scratch {
    fn new(id: &str) -> Self {
        let p = std::env::temp_dir().join(format!("cam-m3-{id}-{}", std::process::id()));
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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/m3/{name}.json"))
}
fn json(p: &PathBuf) -> Value {
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn endmill_plan_inspection_and_verification_replay_embedded_job() {
    let s = Scratch::new("replay");
    let job = s.0.join("job.json");
    fs::copy(fixture("rectangle"), &job).unwrap();
    let plan = s.0.join("plan.json");
    let svg = s.0.join("preview.svg");
    let report = s.0.join("report.json");
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
    assert!(out.stdout.is_empty());
    let data = json(&plan);
    assert_eq!(data["artifact_kind"], "endmill_plan");
    assert!(data.get("analysis").is_none());
    let rebuilt = EndmillPlan::from_json(&data.to_string()).unwrap();
    assert_eq!(
        rebuilt.analysis.status,
        cam_core::pocket::PlanStatus::Complete
    );
    assert_eq!(rebuilt.analysis.layers[1].depth_mm, 2.);
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
    let preview = fs::read_to_string(&svg).unwrap();
    assert!(preview.contains("M3 endmill stage: Complete"));
    assert!(preview.contains("Remaining target / missing floor"));
    assert!(preview.contains("Recorded motion centers"));
    let out = cam()
        .arg("verify")
        .arg(&plan)
        .arg("--output")
        .arg(&report)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        json(&report)["analysis"],
        serde_json::to_value(&rebuilt.analysis).unwrap()
    );
    let mut changed = data;
    changed["analysis"] = serde_json::json!({"status":"empty","layers":[]});
    fs::write(&plan, changed.to_string()).unwrap();
    assert!(
        cam()
            .arg("verify")
            .arg(&plan)
            .arg("--output")
            .arg(&report)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(json(&report)["analysis"]["status"], "complete");
}

#[test]
fn empty_and_partial_plans_have_explicit_exit_status_and_inspectable_stock() {
    let s = Scratch::new("status");
    for (name, status, exit) in [
        ("no-access", "empty", 0),
        ("unsupported-entry", "incomplete", 1),
        ("resource-limit", "inconclusive", 1),
    ] {
        let plan = s.0.join(format!("{name}.json"));
        let svg = s.0.join(format!("{name}.svg"));
        let out = cam()
            .arg("plan")
            .arg(fixture(name))
            .arg("--output")
            .arg(&plan)
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(exit),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let data = json(&plan);
        assert_eq!(data["artifact_kind"], "endmill_plan");
        let rebuilt = EndmillPlan::from_json(&data.to_string()).unwrap();
        assert_eq!(
            serde_json::to_value(rebuilt.analysis.status).unwrap(),
            status
        );
        let out = cam()
            .arg("inspect")
            .arg(&plan)
            .arg("--output")
            .arg(&svg)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(exit));
        assert!(
            fs::read_to_string(svg)
                .unwrap()
                .contains("M3 endmill stage")
        );
    }
}

#[test]
fn stale_plans_invalidate_previous_reports_and_previews() {
    let s = Scratch::new("stale");
    let plan = s.0.join("plan.json");
    let svg = s.0.join("preview.svg");
    let report = s.0.join("report.json");
    assert!(
        cam()
            .arg("plan")
            .arg(fixture("rectangle"))
            .arg("--output")
            .arg(&plan)
            .output()
            .unwrap()
            .status
            .success()
    );
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
    let mut data = json(&plan);
    data["job"]["tools"][0]["cutting_feed_mm_min"] = serde_json::json!(301.);
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
        fs::read_to_string(&svg)
            .unwrap()
            .contains("Job preview unavailable")
    );
    let out = cam()
        .arg("verify")
        .arg(&plan)
        .arg("--output")
        .arg(&report)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(json(&report)["valid"], false);
    let out = cam()
        .arg("plan")
        .arg(fixture("rectangle"))
        .arg("--output")
        .arg(fixture("rectangle"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}
