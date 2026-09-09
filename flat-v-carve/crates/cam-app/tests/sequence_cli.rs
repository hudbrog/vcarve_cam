//! ui-8 CLI workflow: a migrated V-carve operation opens, plans and exports
//! through the sequence pipeline.
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
struct Scratch(PathBuf);
impl Scratch {
    fn new(id: &str) -> Self {
        let p = std::env::temp_dir().join(format!("cam-seq-{id}-{}", std::process::id()));
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
fn fixture(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../{relative}"))
}
fn json(p: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}
fn run_ok(command: &mut Command) -> String {
    let output = command.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "command failed: {stderr}");
    stderr
}

fn sequence_profile(scratch: &Scratch) -> PathBuf {
    let legacy: Value = serde_json::from_str(
        &fs::read_to_string(fixture("../real_data/machine-profile.json")).unwrap(),
    )
    .unwrap();
    let profile = serde_json::json!({
        "schema_version": 2,
        "id": legacy["id"],
        "work_offset": legacy["work_offset"],
        "clearance_z_mm": legacy["clearance_z_mm"],
        "decimal_places": legacy["decimal_places"],
        "program_start_position_mm": null,
        "length_compensation": legacy["length_compensation"],
        "path_control": {"kind": "exact_path"},
        "tools": legacy["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| serde_json::json!({
                "tool_id": t["tool_id"], "tool_number": t["tool_number"],
                "length_offset_number": t["length_offset_number"],
            }))
            .collect::<Vec<_>>(),
        "spindle_spinup_seconds": legacy["spindle_spinup_seconds"],
        "coolant": legacy["coolant"],
        "m6": legacy["m6"],
    });
    let path = scratch.0.join("sequence-profile.json");
    fs::write(&path, serde_json::to_string_pretty(&profile).unwrap()).unwrap();
    path
}

#[test]
fn migrated_vcarve_opens_plans_and_exports_through_the_sequence_cli() {
    let s = Scratch::new("workflow");
    let legacy = fixture("fixtures/m3/rectangle.json");

    // Open: legacy schema-2 document migrates to schema 4.
    let job4 = s.0.join("job4.json");
    let stderr = run_ok(
        cam()
            .args(["sequence", "open"])
            .arg(&legacy)
            .arg("--output")
            .arg(&job4),
    );
    assert!(stderr.contains("flat-v-carve"), "{stderr}");
    let document = json(&job4);
    assert_eq!(document["schema_version"], Value::from(4));

    // Re-open the canonical document; identity is stable.
    let reopened = s.0.join("reopened.json");
    run_ok(
        cam()
            .args(["sequence", "open"])
            .arg(&job4)
            .arg("--output")
            .arg(&reopened),
    );
    assert_eq!(json(&reopened), document);

    // Export before the profile is applied fails on unresolved process state.
    let early = cam()
        .args(["sequence", "export"])
        .arg(&job4)
        .arg("--profile")
        .arg(sequence_profile(&s))
        .arg("--output")
        .arg(s.0.join("early"))
        .output()
        .unwrap();
    assert!(!early.status.success());
    assert!(String::from_utf8_lossy(&early.stderr).contains("PROCESS_SPINDLE_STATE"));

    // Apply the legacy machine profile, then export end to end.
    let applied = s.0.join("applied.json");
    run_ok(
        cam()
            .args(["sequence", "apply-profile"])
            .arg(&job4)
            .arg("--profile")
            .arg(fixture("../real_data/machine-profile.json"))
            .arg("--output")
            .arg(&applied),
    );
    let applied_document = json(&applied);
    assert_eq!(
        applied_document["setup"]["work_zero"]["z"],
        Value::String("stock_bottom".into())
    );

    // Plan the whole enabled sequence.
    let summary_path = s.0.join("plan-summary.json");
    let stderr = run_ok(
        cam()
            .args(["sequence", "plan"])
            .arg(&applied)
            .arg("--output")
            .arg(&summary_path),
    );
    assert!(stderr.contains("checks: passed"), "{stderr}");
    let summary = json(&summary_path)["summary"].clone();
    let motion_count = summary["motionCount"].as_u64().unwrap();
    assert!(motion_count > 0);

    // Prefix scope plans only up to the named operation.
    let prefix_path = s.0.join("plan-prefix.json");
    run_ok(
        cam()
            .args(["sequence", "plan"])
            .arg(&applied)
            .arg("--through")
            .arg("flat-v-carve")
            .arg("--output")
            .arg(&prefix_path),
    );
    assert_eq!(
        json(&prefix_path)["summary"]["motionCount"]
            .as_u64()
            .unwrap(),
        motion_count,
        "single-operation job: prefix equals full plan"
    );

    // Export writes the ordered program and report.
    let export_dir = s.0.join("export");
    let stderr = run_ok(
        cam()
            .args(["sequence", "export"])
            .arg(&applied)
            .arg("--profile")
            .arg(sequence_profile(&s))
            .arg("--output")
            .arg(&export_dir),
    );
    assert!(stderr.contains("motions:"), "{stderr}");
    let gcode = fs::read_to_string(export_dir.join("sequence.ngc")).unwrap();
    assert!(gcode.contains("T1 M6"));
    assert!(gcode.contains("M3 S10000"));
    assert!(gcode.ends_with("M2\n"));
    let report = json(&export_dir.join("sequence-export-report.json"));
    assert_eq!(report["motionCount"].as_u64().unwrap(), motion_count);
    assert_eq!(
        report["basicChecks"]["status"],
        Value::String("passed".into())
    );
    // Stock-bottom datum: clearance 5 over 8 mm exports as machine Z 13.000.
    assert!(gcode.contains("Z13.000"));
}
