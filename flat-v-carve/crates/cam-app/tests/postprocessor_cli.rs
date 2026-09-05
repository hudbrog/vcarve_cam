use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("cam-m6-{name}-{}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn plan(&self) -> PathBuf {
        let path = self.0.join("plan.json");
        let out = cam()
            .args(["plan"])
            .arg(fixture("m4/narrow-channel.json"))
            .arg("--output")
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        path
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
fn fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(path)
}
fn export(plan: &Path, output: &Path) -> Command {
    let mut cmd = cam();
    cmd.arg("export")
        .arg(plan)
        .arg("--profile")
        .arg(fixture("m6/macro-stock-bottom.json"))
        .arg("--output")
        .arg(output);
    cmd
}
fn json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn export_bundle_contains_verified_bytes_and_saved_program_can_be_rechecked() {
    let s = Scratch::new("roundtrip");
    let plan = s.plan();
    let out = s.0.join("export");
    let result = export(&plan, &out).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
    let report = json(&out.join("export-report.json"));
    assert_eq!(report["status"], "passed");
    assert_eq!(report["machine_z_offset_mm"], 8.);
    let program = out.join("combined.ngc");
    assert!(
        fs::read_to_string(&program)
            .unwrap()
            .contains("G0 Z150.000000")
    );
    let report_path = s.0.join("readback.json");
    let result = cam()
        .arg("verify-gcode")
        .arg(&plan)
        .arg("--profile")
        .arg(fixture("m6/macro-stock-bottom.json"))
        .arg("--program")
        .arg(&program)
        .arg("--output")
        .arg(&report_path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        json(&report_path)["programs"][0]["sha256"],
        report["programs"][0]["sha256"]
    );
    assert_eq!(json(&report_path)["status"], "passed");
}

#[test]
fn failed_and_inconclusive_exports_publish_report_only_and_do_not_overwrite() {
    let s = Scratch::new("failure");
    let plan = s.plan();
    let out = s.0.join("limited");
    let result = export(&plan, &out)
        .args(["--max-cells", "1"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(
        json(&out.join("export-report.json"))["status"],
        "inconclusive"
    );
    assert_eq!(fs::read_dir(&out).unwrap().count(), 1);
    let before = fs::read(out.join("export-report.json")).unwrap();
    let result = export(&plan, &out).output().unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(fs::read(out.join("export-report.json")).unwrap(), before);
    let mut p = json(&fixture("m6/macro-stock-bottom.json"));
    p["decimal_places"] = 0.into();
    let profile = s.0.join("coarse.json");
    fs::write(&profile, p.to_string()).unwrap();
    let failed = s.0.join("failed");
    let result = cam()
        .arg("export")
        .arg(&plan)
        .arg("--profile")
        .arg(&profile)
        .arg("--output")
        .arg(&failed)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(json(&failed.join("export-report.json"))["status"], "failed");
    assert_eq!(fs::read_dir(&failed).unwrap().count(), 1);
}

#[test]
fn missing_or_unreviewed_profile_and_changed_saved_code_cannot_export() {
    let s = Scratch::new("contract");
    let plan = s.plan();
    let out = s.0.join("missing");
    let result = cam()
        .arg("export")
        .arg(&plan)
        .arg("--output")
        .arg(&out)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(!out.exists());
    let mut p = json(&fixture("m6/macro-stock-bottom.json"));
    p["m6"]["reviewed"] = false.into();
    let profile = s.0.join("unreviewed.json");
    fs::write(&profile, p.to_string()).unwrap();
    let result = cam()
        .arg("export")
        .arg(&plan)
        .arg("--profile")
        .arg(&profile)
        .arg("--output")
        .arg(&out)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(!out.exists());
    assert!(String::from_utf8_lossy(&result.stderr).contains("POST_M6_CONTRACT"));
    let out = s.0.join("good");
    assert!(export(&plan, &out).output().unwrap().status.success());
    let program = out.join("combined.ngc");
    let changed = fs::read_to_string(&program).unwrap().replace("G90", "G91");
    fs::write(&program, changed).unwrap();
    let readback = s.0.join("tampered.json");
    let result = cam()
        .arg("verify-gcode")
        .arg(&plan)
        .arg("--profile")
        .arg(fixture("m6/macro-stock-bottom.json"))
        .arg("--program")
        .arg(&program)
        .arg("--output")
        .arg(&readback)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(json(&readback)["status"], "failed");
}

#[test]
fn per_tool_empty_stage_and_input_output_aliases_are_handled() {
    let s = Scratch::new("per-tool");
    let plan = s.plan();
    let out = s.0.join("split");
    let result = export(&plan, &out)
        .args(["--layout", "per-tool"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(out.join("vbit.ngc").exists());
    assert!(!out.join("endmill.ngc").exists());
    let before = fs::read(&plan).unwrap();
    let alias = s.0.join(".").join("plan.json");
    let result = export(&plan, &alias).output().unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(fs::read(&plan).unwrap(), before);
}
