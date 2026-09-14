//! ui-9 CLI workflow (H5): a stored schema-5 document gains the applied machine
//! configuration and exports through the retained runtime — the written files
//! match their manifest digests, and the one-program layout names its single
//! file exactly.
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

#[test]
fn a_stored_document_exports_retained_files_matching_their_manifest() {
    let s = Scratch::new("export");
    // The stored document is already the one document model, and the fixture
    // is copied in whole: the applied configuration is the profile's one
    // snapshot, and the profile file is never needed again afterwards.
    let configured = s.0.join("configured.json");
    let stderr = run_ok(
        cam()
            .args(["collection", "apply-machine"])
            .arg(fixture("fixtures/gui2/flower.job.json"))
            .arg("--profile")
            .arg(fixture("fixtures/gui2/machine.json"))
            .arg("--name")
            .arg("Workbench")
            .arg("--output")
            .arg(&configured),
    );
    assert!(
        stderr.contains("2 mapping rows"),
        "the readout counts the applied mappings: {stderr}"
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

#[test]
fn the_exported_library_catalog_drives_the_same_copy_on_the_command_line() {
    let s = Scratch::new("catalog");
    let applied = s.0.join("applied.json");
    // `fixtures/gui5/library.json` is the file the workspace exports: the
    // catalog wrapper, not the library object. The command takes it as it is.
    let stderr = run_ok(
        cam()
            .args(["collection", "apply-profile"])
            .arg(fixture("fixtures/gui4/lettering.job.json"))
            .arg("--library")
            .arg(fixture("fixtures/gui5/library.json"))
            .arg("--library-id")
            .arg("gui5-lettering-library")
            .arg("--operation")
            .arg("carving")
            .arg("--role")
            .arg("endmill")
            .arg("--tool")
            .arg("endmill")
            .arg("--preset")
            .arg("rough")
            .arg("--output")
            .arg(&applied),
    );
    assert!(
        stderr.contains("applied to operation 'carving'"),
        "{stderr}"
    );
    let job = json(&applied);
    assert_eq!(job["schema_version"], json!(5));
    let assignment = &job["operations"][0]["settings"]["settings"]["endmill"];
    assert_eq!(
        assignment["applied_profile"]["library_id"],
        json!("gui5-lettering-library")
    );
    assert_eq!(assignment["applied_profile"]["preset_id"], json!("rough"));
    // The copied values are the preset's, and the document is now a receipt:
    // the library file is not needed again.
    assert_eq!(assignment["spindle_rpm"], json!(12000.0));
    assert_eq!(assignment["cutting_feed_mm_min"], json!(1200.0));
    assert_eq!(assignment["max_stepdown_mm"], json!(0.5));
}
