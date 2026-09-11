//! ui-9 CLI workflow (H5): a migrated collection document gains the applied
//! machine configuration and exports through the retained runtime — the
//! written files match their manifest digests, and the one-program layout
//! names its single file exactly.
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
struct Scratch(PathBuf);
impl Scratch {
    fn new(id: &str) -> Self {
        let p = std::env::temp_dir().join(format!("cam-col-{id}-{}", std::process::id()));
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

/// The schema-2 fixture predates per-assignment spindle direction; give
/// every assignment one so preparation has complete process state.
fn set_spindle_direction(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let needs = map.contains_key("tool_id")
                && map.get("spindle_direction").is_none_or(Value::is_null);
            if needs {
                map.insert("spindle_direction".into(), json!("clockwise"));
            }
            for (_, child) in map.iter_mut() {
                set_spindle_direction(child);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                set_spindle_direction(item);
            }
        }
        _ => {}
    }
}

fn machine_profile(job: &Value, scratch: &Scratch) -> PathBuf {
    let tools: Vec<Value> = job["tools"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(index, tool)| {
            json!({"tool_id": tool["id"], "tool_number": 3 + index,
                "length_offset_number": null})
        })
        .collect();
    let profile = json!({
        "schema_version": 2, "id": "workbench", "work_offset": "G54",
        "clearance_z_mm": 5.0, "decimal_places": 3,
        "program_start_position_mm": null,
        "length_compensation": "macro_managed",
        "path_control": {"kind": "exact_path"},
        "tools": tools, "spindle_spinup_seconds": 0.5, "coolant": "off",
        "m6": {
            "reference": "test-contract", "reviewed": true,
            "return_position": {"kind": "caller_position"},
            "preserves_work_datum": true, "local_offsets_unused": true,
            "tool_offsets_z_only": true,
        },
    });
    let path = scratch.0.join("machine-profile.json");
    fs::write(&path, serde_json::to_string_pretty(&profile).unwrap()).unwrap();
    path
}

#[test]
fn migrated_document_exports_retained_files_matching_their_manifest() {
    let s = Scratch::new("export");
    // Open: the legacy schema-2 document migrates once into schema 5.
    let schema5 = s.0.join("schema5.json");
    let stderr = run_ok(
        cam()
            .args(["collection", "open"])
            .arg(fixture("fixtures/m3/rectangle.json"))
            .arg("--output")
            .arg(&schema5),
    );
    assert!(stderr.contains("migrated: true"), "{stderr}");
    let mut job = json(&schema5);
    assert_eq!(job["schema_version"], json!(5));
    set_spindle_direction(&mut job);
    let patched = s.0.join("patched.json");
    fs::write(&patched, serde_json::to_string_pretty(&job).unwrap()).unwrap();

    // Apply the machine configuration; the profile file is never needed
    // again afterwards.
    let configured = s.0.join("configured.json");
    run_ok(
        cam()
            .args(["collection", "apply-machine"])
            .arg(&patched)
            .arg("--profile")
            .arg(machine_profile(&job, &s))
            .arg("--name")
            .arg("Workbench")
            .arg("--output")
            .arg(&configured),
    );

    // Retained export: one generation, one preparation, ordered files whose
    // bytes match the manifest digests exactly.
    let sequential = s.0.join("sequential");
    let stderr = run_ok(
        cam()
            .args(["collection", "export"])
            .arg(&configured)
            .arg("--output")
            .arg(&sequential),
    );
    assert!(stderr.contains("exported"), "{stderr}");
    let manifest = json(&sequential.join("manifest.json"));
    assert_eq!(manifest["artifactKind"], json!("sequence_export_manifest"));
    assert_eq!(manifest["layout"], json!("sequential_files"));
    let files = manifest["files"].as_array().unwrap();
    assert!(!files.is_empty(), "one file per contiguous tool stage");
    for file in files {
        let filename = file["filename"].as_str().unwrap();
        let bytes = fs::read(sequential.join(filename)).unwrap();
        let digest = {
            use sha2::{Digest, Sha256};
            let mut hash = Sha256::new();
            hash.update(&bytes);
            format!("{:x}", hash.finalize())
        };
        assert_eq!(digest, file["sha256"].as_str().unwrap(), "{filename}");
        assert_eq!(bytes.len(), file["byteLength"].as_u64().unwrap() as usize);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.ends_with("M2\n") || text.ends_with("M2"), "{filename}");
    }
    // The order prefix and the tool number appear in the first filename.
    assert!(
        files[0]["filename"].as_str().unwrap().starts_with("01-"),
        "sequential files are numbered in execution order"
    );
    assert!(sequential.join("report.json").exists());

    // The one-program layout names its single file exactly.
    let one = s.0.join("one");
    let stderr = run_ok(
        cam()
            .args(["collection", "export"])
            .arg(&configured)
            .arg("--output")
            .arg(&one)
            .arg("--layout")
            .arg("one"),
    );
    assert!(stderr.contains("layout: one_program"), "{stderr}");
    let manifest = json(&one.join("manifest.json"));
    assert_eq!(manifest["layout"], json!("one_program"));
    assert_eq!(manifest["files"][0]["filename"], json!("sequence.ngc"));
    assert!(one.join("sequence.ngc").exists());

    // Unknown layouts are refused before any file is written.
    let refused = cam()
        .args(["collection", "export"])
        .arg(&configured)
        .arg("--output")
        .arg(s.0.join("refused"))
        .arg("--layout")
        .arg("sideways")
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("unknown layout"));
}
