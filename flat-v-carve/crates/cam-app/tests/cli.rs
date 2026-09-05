use std::{fs, path::PathBuf, process::Command};

struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("cam-m0-{name}-{}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
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

#[test]
fn bundled_suite_exports_and_replays_a_reproducer() {
    let scratch = Scratch::new("artifacts");
    let output = cam()
        .args(["geometry-spike", "--output"])
        .arg(scratch.0.join("all"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(scratch.0.join("all/report.json")).unwrap())
            .unwrap();
    assert_eq!(report["summary"]["failed"], 0);
    assert_eq!(report["schema_version"], 1);
    let repro = scratch.0.join("all/repro/voronoi_concave.json");
    let replay = cam()
        .args(["geometry-spike", "--fixture"])
        .arg(&repro)
        .arg("--output")
        .arg(scratch.0.join("replay"))
        .output()
        .unwrap();
    assert!(replay.status.success());
    assert_eq!(
        fs::read(scratch.0.join("all/voronoi_concave.json")).unwrap(),
        fs::read(scratch.0.join("replay/voronoi_concave.json")).unwrap()
    );
    let svg = fs::read_to_string(scratch.0.join("all/voronoi_concave.svg")).unwrap();
    assert!(svg.contains("<polyline"));
    assert!(svg.contains("<svg xmlns=\"http://www.w3.org/2000/svg\""));

    // Challenge the harness: incorrect evidence must produce a failed report and exit 1.
    let mut fixture: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&repro).unwrap()).unwrap();
    fixture["expected"]["area_mm2"] = serde_json::json!(999);
    fs::write(&repro, serde_json::to_string(&fixture).unwrap()).unwrap();
    let fail = cam()
        .args(["geometry-spike", "--fixture"])
        .arg(&repro)
        .arg("--output")
        .arg(scratch.0.join("failed"))
        .output()
        .unwrap();
    assert_eq!(fail.status.code(), Some(1));
    assert!(scratch.0.join("failed/voronoi_concave.svg").exists());
}

#[test]
fn malformed_arguments_and_unsafe_fixture_ids_are_rejected() {
    for args in [
        vec!["plan"],
        vec!["geometry-spike"],
        vec!["geometry-spike", "--output"],
    ] {
        assert_eq!(cam().args(args).output().unwrap().status.code(), Some(2));
    }
    let scratch = Scratch::new("validation");
    let fixture = serde_json::json!({"id":"../escape","description":"bad id","tolerance_mm":0.001,"rings":[],"operation":{"kind":"normalize"},"expected":{}});
    let input = scratch.0.join("input.json");
    fs::write(&input, fixture.to_string()).unwrap();
    let output = cam()
        .args(["geometry-spike", "--fixture"])
        .arg(input)
        .arg("--output")
        .arg(scratch.0.join("out"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(!scratch.0.join("out").exists());
}

#[test]
fn model_validation_is_machine_readable_and_rejects_incompatible_tool_changes() {
    let scratch = Scratch::new("m1-validation");
    let input = scratch.0.join("model.json");
    let mut model: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/m1/wide_channel.json")).unwrap();
    fs::write(&input, model.to_string()).unwrap();
    let valid = cam()
        .args(["validate-model", "--input"])
        .arg(&input)
        .output()
        .unwrap();
    assert!(valid.status.success());
    let result: serde_json::Value = serde_json::from_slice(&valid.stdout).unwrap();
    assert_eq!(result["valid"], true);
    assert_eq!(result["normalized_area_mm2"], 200.0);
    model["vbit"]["cutting_height_mm"] = serde_json::json!(1.0);
    fs::write(&input, model.to_string()).unwrap();
    let invalid = cam()
        .args(["validate-model", "--input"])
        .arg(input)
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(1));
    let result: serde_json::Value = serde_json::from_slice(&invalid.stdout).unwrap();
    assert_eq!(result["valid"], false);
    assert_eq!(result["diagnostics"][0]["code"], "VBIT_CUTTING_HEIGHT");
}

#[test]
fn target_previews_replay_and_failed_edits_invalidate_previous_geometry() {
    let scratch = Scratch::new("m1-preview");
    let input = scratch.0.join("model.json");
    let mut model: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/m1/finite_tip_corner.json")).unwrap();
    fs::write(&input, model.to_string()).unwrap();
    let preview = cam()
        .args(["target-preview", "--input"])
        .arg(&input)
        .arg("--output")
        .arg(scratch.0.join("first"))
        .output()
        .unwrap();
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let replay = cam()
        .args(["target-preview", "--input"])
        .arg(scratch.0.join("first/input.json"))
        .arg("--output")
        .arg(scratch.0.join("replay"))
        .output()
        .unwrap();
    assert!(replay.status.success());
    assert_eq!(
        fs::read(scratch.0.join("first/report.json")).unwrap(),
        fs::read(scratch.0.join("replay/report.json")).unwrap()
    );
    model["max_reachability_cells"] = serde_json::json!(1);
    fs::write(&input, model.to_string()).unwrap();
    let unresolved = cam()
        .args(["target-preview", "--input"])
        .arg(&input)
        .arg("--output")
        .arg(scratch.0.join("first"))
        .output()
        .unwrap();
    assert_eq!(unresolved.status.code(), Some(1));
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(scratch.0.join("first/report.json")).unwrap())
            .unwrap();
    assert_eq!(report["preview"]["status"], "inconclusive");
    model["vbit"]["cutting_height_mm"] = serde_json::json!(1.0);
    fs::write(&input, model.to_string()).unwrap();
    let invalid = cam()
        .args(["target-preview", "--input"])
        .arg(&input)
        .arg("--output")
        .arg(scratch.0.join("first"))
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(1));
    let svg = fs::read_to_string(scratch.0.join("first/preview.svg")).unwrap();
    assert!(svg.contains("Invalid model"));
    assert!(svg.contains("VBIT_CUTTING_HEIGHT"));
    assert!(!svg.contains("<polyline"));
}

#[test]
fn target_demo_exports_all_procedural_models() {
    let scratch = Scratch::new("m1-demo");
    let output = cam()
        .args(["target-demo", "--output"])
        .arg(&scratch.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(scratch.0.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["complete"], true);
    assert_eq!(report["models"].as_array().unwrap().len(), 8);
    for model in report["models"].as_array().unwrap() {
        assert!(scratch.0.join(model["svg"].as_str().unwrap()).is_file());
    }
}
