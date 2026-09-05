use cam_app::tool_library::ToolLibraryStore;
use cam_core::{
    job::Job,
    tool_library::{LibraryChange, LibraryTool, MAX_LIBRARY_BYTES, ToolLibrary},
};
use serde_json::Value;
use std::{
    fs::{self, File},
    path::PathBuf,
    process::{Command, Output},
    sync::{
        Arc, Barrier,
        atomic::{AtomicU64, Ordering},
    },
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let parent = std::env::temp_dir().canonicalize().unwrap();
        let path = parent.join(format!(
            "cam-library-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn store(&self) -> ToolLibraryStore {
        ToolLibraryStore::new(self.0.join("store"))
    }
    fn cam(&self, command: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_cam"))
            .current_dir(&self.0)
            .args(["tool-library", command, "store"])
            .args(args)
            .output()
            .unwrap()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        // Only remove this test's canonical direct child of the temporary directory.
        let parent = std::env::temp_dir().canonicalize().unwrap();
        if let Ok(path) = self.0.canonicalize()
            && path.parent() == Some(parent.as_path())
            && path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("cam-library-")
        {
            let _ = fs::remove_dir_all(path);
        }
    }
}
fn tool(id: &str) -> LibraryTool {
    let job = Job::from_json(include_str!("../../../fixtures/m4/island.json")).unwrap();
    LibraryTool::from_settings(id.into(), "Synthetic cutter".into(), &job.tools[0]).unwrap()
}
fn ok(output: Output) -> Output {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn rejected(output: Output, code: &str) {
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn store_survives_reopen_and_rejected_edits_preserve_exact_bytes() {
    let s = Scratch::new();
    let store = s.store();
    assert!(store.load().is_err());
    assert_eq!(store.initialize().unwrap().revision, 0);
    store
        .change(0, LibraryChange::AddTool { tool: tool("mill") })
        .unwrap();
    let before = fs::read(s.0.join("store/library.json")).unwrap();
    assert_eq!(s.store().load().unwrap().tools[0].id, "mill");
    assert_eq!(store.initialize().unwrap_err().code, "LIBRARY_EXISTS");
    assert_eq!(
        store
            .change(
                0,
                LibraryChange::RemoveTool {
                    tool_id: "mill".into()
                }
            )
            .unwrap_err()
            .code,
        "LIBRARY_CONFLICT"
    );
    let mut invalid = tool("mill");
    invalid.name.clear();
    assert_eq!(
        store
            .change(1, LibraryChange::ReplaceTool { tool: invalid })
            .unwrap_err()
            .code,
        "LIBRARY_LABEL"
    );
    assert_eq!(
        store
            .import_json(1, &store.export_json().unwrap())
            .unwrap_err()
            .code,
        "LIBRARY_DUPLICATE_ID"
    );
    assert_eq!(fs::read(s.0.join("store/library.json")).unwrap(), before);
    assert_eq!(fs::read_dir(s.0.join("store")).unwrap().count(), 2); // Data and stable lock; no temporary leak.
}

#[test]
fn corrupt_oversized_or_future_library_is_never_reinitialized_or_replaced() {
    let s = Scratch::new();
    let store = s.store();
    store.initialize().unwrap();
    for contents in [
        "{truncated".into(),
        " ".repeat(MAX_LIBRARY_BYTES + 1),
        r#"{"schema_version":99,"revision":0,"tools":[]}"#.into(),
    ] {
        fs::write(s.0.join("store/library.json"), &contents).unwrap();
        assert!(store.load().is_err());
        assert!(
            store
                .change(0, LibraryChange::AddTool { tool: tool("mill") })
                .is_err()
        );
        assert_eq!(store.initialize().unwrap_err().code, "LIBRARY_EXISTS");
        assert_eq!(
            fs::read_to_string(s.0.join("store/library.json")).unwrap(),
            contents
        );
    }
}

#[test]
fn active_lock_blocks_writers_and_dropping_handle_releases_it() {
    let s = Scratch::new();
    let store = s.store();
    store.initialize().unwrap();
    let lock = File::options()
        .read(true)
        .write(true)
        .open(s.0.join("store/library.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert_eq!(
        store
            .change(0, LibraryChange::AddTool { tool: tool("mill") })
            .unwrap_err()
            .code,
        "LIBRARY_BUSY"
    );
    assert_eq!(store.load().unwrap().revision, 0); // Readers need no writer lock.
    drop(lock);
    assert_eq!(
        store
            .change(0, LibraryChange::AddTool { tool: tool("mill") })
            .unwrap()
            .revision,
        1
    );
}

#[test]
fn simultaneous_writers_cannot_commit_the_same_revision() {
    let s = Scratch::new();
    let store = s.store();
    store.initialize().unwrap();
    let barrier = Arc::new(Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|i| {
            let store = store.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let change = LibraryChange::AddTool {
                    tool: tool(&format!("mill-{i}")),
                };
                barrier.wait();
                store.change(0, change)
            })
        })
        .collect();
    let mut successes = 0;
    for worker in workers {
        match worker.join().unwrap() {
            Ok(_) => successes += 1,
            Err(e) => assert!(matches!(
                e.code.as_str(),
                "LIBRARY_BUSY" | "LIBRARY_CONFLICT"
            )),
        }
    }
    assert_eq!(successes, 1);
    let library = store.load().unwrap();
    assert_eq!(library.revision, 1);
    assert_eq!(library.tools.len(), 1);
}

#[test]
fn cli_capture_export_import_apply_and_job_validation_work_end_to_end() {
    let s = Scratch::new();
    let source = include_str!("../../../fixtures/m4/island.json");
    fs::write(s.0.join("job.json"), source).unwrap();
    let initialized: Value = serde_json::from_slice(&ok(s.cam("init", &[])).stdout).unwrap();
    assert_eq!(initialized["revision"], 0);
    let captured = ok(s.cam(
        "capture",
        &[
            "--expected-revision",
            "0",
            "--job",
            "job.json",
            "--slot",
            "endmill",
            "--tool",
            "mill",
            "--name",
            "Synthetic mill",
            "--preset",
            "test",
            "--preset-name",
            "Synthetic preset",
            "--material",
            "Test material",
            "--machine",
            "Test machine",
        ],
    ));
    let library: Value = serde_json::from_slice(&captured.stdout).unwrap();
    assert_eq!(
        library["tools"][0]["cutting_presets"][0]["material"],
        "Test material"
    );
    ok(s.cam("export", &["--output", "library-export.json"]));
    let exported = fs::read(s.0.join("library-export.json")).unwrap();
    ok(s.cam(
        "apply",
        &[
            "--expected-revision",
            "1",
            "--job",
            "job.json",
            "--slot",
            "endmill",
            "--tool",
            "mill",
            "--preset",
            "test",
            "--output",
            "applied.json",
        ],
    ));
    let applied = Job::from_json(&fs::read_to_string(s.0.join("applied.json")).unwrap()).unwrap();
    assert_eq!(
        applied.to_json().unwrap(),
        Job::from_json(source).unwrap().to_json().unwrap()
    );
    ok(Command::new(env!("CARGO_BIN_EXE_cam"))
        .arg("validate-job")
        .arg(s.0.join("applied.json"))
        .output()
        .unwrap());
    assert_eq!(fs::read_to_string(s.0.join("job.json")).unwrap(), source);
    let second = ToolLibraryStore::new(s.0.join("second"));
    second.initialize().unwrap();
    let imported = second
        .import_json(0, std::str::from_utf8(&exported).unwrap())
        .unwrap();
    assert_eq!(imported.revision, 1);
    assert_eq!(
        imported.tools[0].cutting_presets[0].spindle_rpm,
        Some(10000.)
    );
    ok(s.cam(
        "apply",
        &[
            "--expected-revision",
            "1",
            "--job",
            "job.json",
            "--slot",
            "endmill",
            "--tool",
            "mill",
            "--output",
            "geometry-only.json",
        ],
    ));
    let blank =
        Job::from_json(&fs::read_to_string(s.0.join("geometry-only.json")).unwrap()).unwrap();
    assert_eq!(blank.tools[0].spindle_rpm, None);
}

#[test]
fn cli_rejects_stale_apply_wrong_slot_bad_arguments_and_existing_outputs() {
    let s = Scratch::new();
    fs::write(
        s.0.join("job.json"),
        include_str!("../../../fixtures/m4/island.json"),
    )
    .unwrap();
    ok(s.cam("init", &[]));
    fs::write(
        s.0.join("change.json"),
        serde_json::to_string(&LibraryChange::AddTool { tool: tool("mill") }).unwrap(),
    )
    .unwrap();
    ok(s.cam(
        "change",
        &["--expected-revision", "0", "--input", "change.json"],
    ));
    rejected(
        s.cam(
            "change",
            &["--expected-revision", "0", "--input", "change.json"],
        ),
        "LIBRARY_CONFLICT",
    );
    rejected(
        s.cam(
            "apply",
            &[
                "--expected-revision",
                "0",
                "--job",
                "job.json",
                "--slot",
                "endmill",
                "--tool",
                "mill",
                "--output",
                "new.json",
            ],
        ),
        "LIBRARY_CONFLICT",
    );
    rejected(
        s.cam(
            "apply",
            &[
                "--expected-revision",
                "1",
                "--job",
                "job.json",
                "--slot",
                "vbit",
                "--tool",
                "mill",
                "--output",
                "new.json",
            ],
        ),
        "LIBRARY_TOOL_KIND",
    );
    rejected(s.cam("export", &["--output", "job.json"]), "CAM_ERROR");
    rejected(
        s.cam(
            "apply",
            &[
                "--expected-revision",
                "1",
                "--job",
                "job.json",
                "--slot",
                "endmill",
                "--tool",
                "mill",
                "--output",
                "job.json",
            ],
        ),
        "CAM_ERROR",
    );
    rejected(
        s.cam("list", &["--input", "change.json"]),
        "unknown/repeated",
    );
    rejected(
        s.cam(
            "change",
            &[
                "--expected-revision",
                "1",
                "--expected-revision",
                "1",
                "--input",
                "change.json",
            ],
        ),
        "unknown/repeated",
    );
    assert!(!s.0.join("new.json").exists());
    assert_eq!(s.store().load().unwrap().revision, 1);
    assert_eq!(
        fs::read_to_string(s.0.join("job.json")).unwrap(),
        include_str!("../../../fixtures/m4/island.json")
    );
}

#[test]
fn cli_import_rejects_collision_and_missing_revision_without_partial_save() {
    let s = Scratch::new();
    ok(s.cam("init", &[]));
    let library = ToolLibrary {
        revision: 40,
        tools: vec![tool("mill")],
        ..ToolLibrary::default()
    };
    fs::write(s.0.join("import.json"), library.to_json().unwrap()).unwrap();
    rejected(
        s.cam("import", &["--input", "import.json"]),
        "--expected-revision is required",
    );
    ok(s.cam(
        "import",
        &["--expected-revision", "0", "--input", "import.json"],
    ));
    rejected(
        s.cam(
            "import",
            &["--expected-revision", "1", "--input", "import.json"],
        ),
        "LIBRARY_DUPLICATE_ID",
    );
    let output = ok(s.cam("list", &[]));
    let listed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(listed["revision"], 1);
    assert_eq!(listed["tools"].as_array().unwrap().len(), 1);
}
