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

#[test]
fn knife_job_adds_exports_and_publishes_evidence_through_the_cli() {
    let s = Scratch::new("knife");
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40mm\" height=\"30mm\" viewBox=\"0 0 40 30\"><path id=\"cut\" fill=\"none\" stroke=\"#000\" stroke-width=\"0.4\" d=\"M5 5 L25 5\"/></svg>";
    let job = serde_json::json!({
        "schema_version": 4,
        "name": "knife-cli",
        "source": {"filename": "cut.svg", "svg": svg},
        "import": {"mode": "centerline", "geometry_tolerance_mm": 0.001,
            "placement": {"origin_mm": {"x": 0.0, "y": 0.0}, "rotation_deg": 0.0, "scale": 1.0}},
        "setup": {
            "stock": {"thickness_mm": 3.0, "xy": {"min_x_mm": 0.0, "min_y_mm": 0.0, "width_mm": 40.0, "length_mm": 30.0}},
            "work_zero": {"xy": {"kind": "setup_origin"}, "z": "stock_top"},
            "clearance_above_stock_mm": 5.0,
        },
        "tools": [{"id": "blade", "name": "drag knife", "capabilities": {},
            "geometry": {"kind": "drag_knife", "dimensions": {"blade_offset_mm": 1.0, "max_cut_depth_mm": 2.0}}}],
        "operations": [{
            "id": "knife-1", "name": "knife-1", "enabled": true,
            "settings": {"kind": "drag_knife", "settings": {
                "chains": ["cut-chain-0"],
                "assignment": {"tool_id": "blade", "cutting_feed_mm_min": 150.0,
                    "plunge_feed_mm_min": 60.0, "swivel_feed_mm_min": 50.0, "max_stepdown_mm": 1.0},
                "top": {"reference": {"kind": "stock_top"}, "offset_mm": 0.0},
                "bottom": {"reference": {"kind": "operation_top"}, "offset_mm": -1.0},
                "stepdown_mm": 1.0, "swivel_depth_mm": 0.2, "corner_threshold_deg": 30.0,
                "start": {"kind": "automatic"}, "alignment": {"initial_heading_deg": 90.0},
            }},
        }],
        "tolerances": {"motion_tolerance_mm": 0.01},
    });
    let job_path = s.0.join("knife-job.json");
    fs::write(&job_path, serde_json::to_string_pretty(&job).unwrap()).unwrap();

    let legacy: Value = serde_json::from_str(
        &fs::read_to_string(fixture("../real_data/machine-profile.json")).unwrap(),
    )
    .unwrap();
    let profile = serde_json::json!({
        "schema_version": 2,
        "id": "knife-cli-profile",
        "work_offset": "G54",
        "clearance_z_mm": 5.0,
        "decimal_places": 3,
        "program_start_position_mm": null,
        "length_compensation": "macro_managed",
        "path_control": {"kind": "blend", "tolerance_mm": 0.01, "naive_cam_tolerance_mm": null},
        "tools": [{"tool_id": "blade", "tool_number": 3, "length_offset_number": null}],
        "spindle_spinup_seconds": 0.5,
        "coolant": "off",
        "m6": legacy["m6"],
    });
    let profile_path = s.0.join("knife-profile.json");
    fs::write(
        &profile_path,
        serde_json::to_string_pretty(&profile).unwrap(),
    )
    .unwrap();

    // Export: knife stages run spindle-off under exact path between nothing
    // else; the emitted bytes carry G61 and the knife tool change.
    let export_dir = s.0.join("export");
    let stderr = run_ok(
        cam()
            .args(["sequence", "export"])
            .arg(&job_path)
            .arg("--profile")
            .arg(&profile_path)
            .arg("--output")
            .arg(&export_dir),
    );
    assert!(stderr.contains("motions:"), "{stderr}");
    let gcode = fs::read_to_string(export_dir.join("sequence.ngc")).unwrap();
    assert!(gcode.contains("T3 M6"));
    assert!(gcode.contains("G61"));
    assert!(!gcode.contains("M3"));
    let report = json(&export_dir.join("sequence-export-report.json"));
    assert_eq!(
        report["knifeReplay"]["status"],
        Value::String("within".into())
    );

    // Knife evidence: the decoded bytes replay independently and the
    // bounded report binds to the exact program hash.
    let evidence_dir = s.0.join("evidence");
    let stderr = run_ok(
        cam()
            .args(["sequence", "knife-evidence"])
            .arg(&job_path)
            .arg("--profile")
            .arg(&profile_path)
            .arg("--samples")
            .arg("16")
            .arg("--output")
            .arg(&evidence_dir),
    );
    assert!(stderr.contains("knife evidence: within"), "{stderr}");
    let evidence = json(&evidence_dir.join("knife-evidence.json"));
    assert_eq!(evidence["status"], Value::String("within".into()));
    assert_eq!(evidence["initialHeadingDeg"], Value::from(90.0));
    assert!(evidence["samples"].as_array().unwrap().len() <= 16);

    // Typed creation: a second knife operation appears fully unconfigured.
    let grown = s.0.join("grown.json");
    run_ok(
        cam()
            .args(["sequence", "add-operation"])
            .arg(&job_path)
            .arg("--kind")
            .arg("drag_knife")
            .arg("--id")
            .arg("knife-2")
            .arg("--tool")
            .arg("blade")
            .arg("--output")
            .arg(&grown),
    );
    let grown_document = json(&grown);
    assert_eq!(grown_document["operations"].as_array().unwrap().len(), 2);
    assert_eq!(
        grown_document["operations"][1]["settings"]["kind"],
        Value::String("drag_knife".into())
    );
    assert_eq!(
        grown_document["operations"][1]["settings"]["settings"]["alignment"],
        serde_json::json!({}),
        "the never-defaulted initial heading stays unset"
    );
}
