//! The job commands on the one document model (§0): `cam import` builds a
//! schema-5 document from one SVG and `cam inspect` reads a stored document
//! back. Planning, selection and export are the collection surface, and an
//! older or future document is refused by name — never converted.
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
struct Scratch(PathBuf);
impl Scratch {
    fn new(id: &str) -> Self {
        let p = std::env::temp_dir().join(format!("cam-job-{id}-{}", std::process::id()));
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
fn json(p: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn import_writes_the_one_document_model_holding_the_artwork() {
    let s = Scratch::new("import");
    let source = include_str!("../../../fixtures/m2/inkscape-export.svg");
    let svg = s.0.join("artwork.svg");
    fs::write(&svg, source).unwrap();
    let job = s.0.join("job.json");
    let output = cam()
        .arg("import")
        .arg(&svg)
        .arg("--output")
        .arg(&job)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The command reports the file it wrote, and that is all it writes.
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        job.display().to_string()
    );
    let document = json(&job);
    assert_eq!(document["schema_version"], json!(5));
    assert_eq!(
        document["operations"].as_array().unwrap().len(),
        0,
        "an import is artwork, not a machining step"
    );
    assert_eq!(document["tools"].as_array().unwrap().len(), 0);
    let artwork = document["artwork"].as_array().unwrap();
    assert_eq!(artwork.len(), 1);
    assert_eq!(artwork[0]["name"], json!("artwork.svg"));
    assert_eq!(artwork[0]["content"]["filename"], json!("artwork.svg"));
    assert_eq!(artwork[0]["content"]["svg"], json!(source));
    // Stock is the referenced page, measured through the placement, not the
    // bounds of the drawn paths.
    assert_eq!(
        document["setup"]["stock"]["xy"],
        json!({"min_x_mm": 0.0, "min_y_mm": 0.0, "width_mm": 100.0, "length_mm": 60.0})
    );

    // The document is the source of truth: the artwork survives losing the
    // file it was imported from.
    fs::remove_file(&svg).unwrap();
    let inspection = s.0.join("inspection.json");
    let output = cam()
        .arg("inspect")
        .arg(&job)
        .arg("--output")
        .arg(&inspection)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json(&inspection);
    assert_eq!(report["artwork"].as_array().unwrap().len(), 1);
    assert_eq!(
        report["artwork"][0]["name"],
        json!("artwork.svg"),
        "inspection reads the stored snapshot, not the source file"
    );
    assert_eq!(
        report["inspection"]["machiningOrder"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert!(report["documentFingerprint"].is_string());
}

#[test]
fn an_older_or_future_document_is_refused_by_name_without_being_converted() {
    let s = Scratch::new("refuse");
    let job = s.0.join("job.json");
    fs::write(&job, include_str!("../../../fixtures/gui2/flower.job.json")).unwrap();
    let inspection = s.0.join("inspection.json");
    let inspect = |input: &Path, output: &Path| {
        cam()
            .arg("inspect")
            .arg(input)
            .arg("--output")
            .arg(output)
            .output()
            .unwrap()
    };
    assert!(
        inspect(&job, &inspection).status.success(),
        "schema 5 is the document model"
    );
    for (version, label) in [(3, "schema 3"), (6, "schema 6")] {
        let mut document = json(&job);
        document["schema_version"] = json!(version);
        let foreign = s.0.join(format!("schema-{version}.json"));
        fs::write(&foreign, serde_json::to_string_pretty(&document).unwrap()).unwrap();
        let refused = s.0.join(format!("refused-{version}.json"));
        let output = inspect(&foreign, &refused);
        assert_eq!(
            output.status.code(),
            Some(2),
            "a document outside the one model is a command fault"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("COLLECTION_SCHEMA_UNSUPPORTED"), "{stderr}");
        assert!(
            stderr.contains(&format!("this is a {label} job")),
            "{stderr}"
        );
        assert!(
            !refused.exists(),
            "a refused document produces no inspection"
        );
    }
}

#[test]
fn unsupported_artwork_is_refused_without_writing_a_document() {
    let s = Scratch::new("reject");
    let source = include_str!("../../../fixtures/m2/inkscape-source.svg");
    let svg = s.0.join("artwork.svg");
    fs::write(&svg, source).unwrap();
    let job = s.0.join("job.json");
    let output = cam()
        .arg("import")
        .arg(&svg)
        .arg("--output")
        .arg(&job)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("SVG_IMPORT"), "{stderr}");
    assert!(stderr.contains("unsupported <text>"), "{stderr}");
    assert!(!job.exists(), "a refused import writes nothing");

    // The one document model never asks the caller to overwrite their source.
    let output = cam()
        .arg("import")
        .arg(&svg)
        .arg("--output")
        .arg(&svg)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("preserve the input"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(&svg).unwrap(), source);

    // A malformed tolerance is a usage fault, not a silent default.
    let output = cam()
        .arg("import")
        .arg(&svg)
        .arg("--tolerance")
        .arg("soon")
        .arg("--output")
        .arg(&job)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--tolerance"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!job.exists());
}
