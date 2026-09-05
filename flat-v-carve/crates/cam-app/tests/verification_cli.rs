use serde_json::Value;
use std::{fs, path::PathBuf, process::Command};
struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("cam-m5-{name}-{}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn plan(&self) -> PathBuf {
        let plan = self.0.join("plan.json");
        let out = cam()
            .arg("plan")
            .arg(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../fixtures/m4/narrow-channel.json"),
            )
            .arg("--output")
            .arg(&plan)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        plan
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
fn read(path: &PathBuf) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn verification_reports_original_and_rounded_bounds_and_exports_locations() {
    let s = Scratch::new("rounded");
    let plan = s.plan();
    let report = s.0.join("report.json");
    let preview = s.0.join("findings.svg");
    let invoke = |places| {
        cam()
            .arg("verify")
            .arg(&plan)
            .arg("--output")
            .arg(&report)
            .arg("--preview")
            .arg(&preview)
            .args(["--decimal-places", places])
            .output()
            .unwrap()
    };
    let out = invoke("6");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty());
    let data = read(&report);
    assert_eq!(data["milestone"], "M5");
    assert_eq!(data["verification"]["status"], "passed");
    assert_eq!(data["verification"]["original"]["status"], "passed");
    assert_eq!(
        data["verification"]["rounded"]["verification"]["status"],
        "passed"
    );
    assert!(data["verification"]["authenticated_plan_fingerprint"].is_string());
    assert!(
        fs::read_to_string(&preview)
            .unwrap()
            .contains("M5 bounded verification: Passed")
    );
    let out = invoke("0");
    assert_eq!(out.status.code(), Some(1));
    let data = read(&report);
    assert_eq!(data["verification"]["status"], "failed");
    assert_eq!(data["verification"]["original"]["status"], "passed");
    assert!(
        fs::read_to_string(&preview)
            .unwrap()
            .contains("M5 bounded verification: Failed")
    );
}

#[test]
fn resource_exhaustion_and_stale_inputs_replace_successful_outputs() {
    let s = Scratch::new("stale");
    let plan = s.plan();
    let report = s.0.join("report.json");
    let preview = s.0.join("findings.svg");
    let invoke = || {
        let mut c = cam();
        c.arg("verify")
            .arg(&plan)
            .arg("--output")
            .arg(&report)
            .arg("--preview")
            .arg(&preview);
        c
    };
    assert!(invoke().output().unwrap().status.success());
    assert_eq!(
        invoke()
            .args(["--max-cells", "1"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(1)
    );
    assert_eq!(read(&report)["verification"]["status"], "inconclusive");
    assert!(
        fs::read_to_string(&preview)
            .unwrap()
            .contains("Inconclusive")
    );
    let mut data = read(&plan);
    data["vbit_motions"][0]["tool_id"] = serde_json::json!("changed-tool");
    fs::write(&plan, data.to_string()).unwrap();
    assert_eq!(invoke().output().unwrap().status.code(), Some(1));
    let data = read(&report);
    assert_eq!(data["valid"], false);
    assert_eq!(data["diagnostics"][0]["code"], "STALE_PLAN");
    assert!(
        !fs::read_to_string(&preview)
            .unwrap()
            .contains("M5 bounded verification: Passed")
    );
}

#[test]
fn verification_flags_are_scoped_and_outputs_cannot_overwrite_the_plan() {
    let s = Scratch::new("arguments");
    let plan = s.plan();
    let original = fs::read(&plan).unwrap();
    let report = s.0.join("report.json");
    let out = cam()
        .arg("verify")
        .arg(&plan)
        .arg("--output")
        .arg(&report)
        .arg("--preview")
        .arg(&plan)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(fs::read(&plan).unwrap(), original);
    for flags in [
        vec!["--max-cells", "0"],
        vec!["--decimal-places", "10"],
        vec!["--max-depth", "1", "--max-depth", "2"],
    ] {
        assert_eq!(
            cam()
                .arg("verify")
                .arg(&plan)
                .arg("--output")
                .arg(&report)
                .args(flags)
                .output()
                .unwrap()
                .status
                .code(),
            Some(2)
        );
    }
    assert_eq!(
        cam()
            .arg("inspect")
            .arg(&plan)
            .arg("--output")
            .arg(&report)
            .args(["--max-cells", "1"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
}
