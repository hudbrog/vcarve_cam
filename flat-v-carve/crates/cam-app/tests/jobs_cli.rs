use serde_json::Value;
use std::{fs, path::PathBuf, process::Command};
struct Scratch(PathBuf);
impl Scratch {
    fn new(id: &str) -> Self {
        let p = std::env::temp_dir().join(format!("cam-m2-{id}-{}", std::process::id()));
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
fn input(s: &Scratch) -> PathBuf {
    let p = s.0.join("artwork.svg");
    fs::write(&p, include_str!("../../../fixtures/m2/inkscape-export.svg")).unwrap();
    p
}
fn json(p: PathBuf) -> Value {
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn import_inspect_and_select_work_after_original_artwork_is_removed() {
    let s = Scratch::new("portable");
    let svg = input(&s);
    let job = s.0.join("job.json");
    let result = cam()
        .arg("import")
        .arg(&svg)
        .arg("--output")
        .arg(&job)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
    let data = json(job.clone());
    assert_eq!(data["schema_version"], 3);
    assert!(data["operation"]["max_depth_mm"].is_null());
    assert!(data["tools"][0]["geometry"].is_null());
    fs::remove_file(&svg).unwrap();
    let preview = s.0.join("preview.svg");
    let report = s.0.join("report.json");
    let result = cam()
        .arg("inspect")
        .arg(&job)
        .arg("--output")
        .arg(&preview)
        .arg("--report")
        .arg(&report)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let data = json(report.clone());
    assert_eq!(data["valid"], true);
    assert_eq!(
        data["inspection"]["geometry"]["sources"]
            .as_array()
            .unwrap()
            .len(),
        7
    );
    assert_eq!(data["inspection"]["planning_available"], true);
    let selected = s.0.join("selected.json");
    let result = cam()
        .arg("select")
        .arg(&job)
        .args(["--select", "letter-o::0", "--output"])
        .arg(&selected)
        .output()
        .unwrap();
    assert!(result.status.success());
    let result = cam()
        .arg("inspect")
        .arg(&selected)
        .arg("--output")
        .arg(&preview)
        .arg("--report")
        .arg(&report)
        .output()
        .unwrap();
    assert!(result.status.success());
    let data = json(report);
    assert_eq!(
        data["inspection"]["geometry"]["selected_region_ids"],
        serde_json::json!(["letter-o::0"])
    );
    assert_eq!(
        data["inspection"]["geometry"]["components"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let svg = fs::read_to_string(preview).unwrap();
    assert!(svg.contains("letter-o::0"));
    assert!(svg.contains("M2 SVG import"));
    assert!(svg.contains("300.000000 mm²"));
}

#[test]
fn editable_jobs_validate_without_machining_settings_and_plan_requires_explicit_settings() {
    let s = Scratch::new("plan");
    let svg = input(&s);
    let job = s.0.join("job.json");
    assert!(
        cam()
            .arg("import")
            .arg(&svg)
            .arg("--output")
            .arg(&job)
            .output()
            .unwrap()
            .status
            .success()
    );
    let out = cam().arg("validate-job").arg(&job).output().unwrap();
    assert!(out.status.success());
    let data: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(data["valid"], true);
    assert!(
        !data["inspection"]["missing_machining_fields"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let plan = s.0.join("plan.json");
    let out = cam()
        .arg("plan")
        .arg(&job)
        .arg("--output")
        .arg(&plan)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let data = json(plan);
    assert_eq!(data["valid"], false);
    assert!(data["plan"].is_null());
    assert_eq!(data["diagnostics"][0]["code"], "MISSING_PLANNING_SETTINGS");
}

#[test]
fn invalid_saved_edits_replace_the_previous_preview_and_report() {
    let s = Scratch::new("invalid");
    let svg = input(&s);
    let job = s.0.join("job.json");
    let preview = s.0.join("preview.svg");
    let report = s.0.join("report.json");
    assert!(
        cam()
            .arg("import")
            .arg(&svg)
            .arg("--output")
            .arg(&job)
            .output()
            .unwrap()
            .status
            .success()
    );
    let args = || {
        let mut c = cam();
        c.arg("inspect")
            .arg(&job)
            .arg("--output")
            .arg(&preview)
            .arg("--report")
            .arg(&report);
        c
    };
    assert!(args().output().unwrap().status.success());
    let mut data = json(job.clone());
    data["selected_region_ids"] = serde_json::json!(["deleted::0"]);
    fs::write(&job, data.to_string()).unwrap();
    assert_eq!(args().output().unwrap().status.code(), Some(1));
    assert_eq!(
        json(report.clone())["diagnostics"][0]["code"],
        "SVG_SELECTION"
    );
    assert!(
        fs::read_to_string(&preview)
            .unwrap()
            .contains("Job preview unavailable")
    );
    fs::write(&job, "{ invalid JSON").unwrap();
    assert_eq!(args().output().unwrap().status.code(), Some(1));
    assert_eq!(json(report)["diagnostics"][0]["code"], "JOB_JSON");
}

#[test]
fn unsupported_artwork_cannot_be_imported_as_a_successful_partial_job() {
    let s = Scratch::new("reject");
    let svg = s.0.join("text.svg");
    let job = s.0.join("job.json");
    fs::write(
        &svg,
        include_str!("../../../fixtures/m2/inkscape-source.svg"),
    )
    .unwrap();
    let out = cam()
        .arg("import")
        .arg(&svg)
        .arg("--output")
        .arg(&job)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(!job.exists());
    assert!(String::from_utf8_lossy(&out.stderr).contains("SVG_TEXT"));
    let out = cam()
        .arg("import")
        .arg(&svg)
        .arg("--output")
        .arg(&svg)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}
