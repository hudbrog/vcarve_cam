#![cfg(not(target_arch = "wasm32"))]
use cam_gui1::{
    file_io::{Store, atomic_write, read_text},
    recovery::{Snapshot, Stored, Tracker},
    state::Draft,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
static ID: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf, PathBuf);
impl Directory {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts/file-probes");
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let path = root.join(format!(
            "{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path, root)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let path = self.0.canonicalize().unwrap();
        assert!(path.starts_with(&self.1) && path != self.1);
        fs::remove_dir_all(path).unwrap();
    }
}
fn snapshot() -> Snapshot {
    let mut d = Draft::default();
    d.raw.insert(d.key(0), "-".into());
    d.raw.insert(d.key(39), "工具 · Инструмент".into());
    Snapshot::new(d, Some(cam_gui1::compute::SMALL.into()))
}
#[test]
fn dropped_native_file_uses_bounded_background_read_and_shared_planning() {
    use cam_gui1::platform::{Event, IoValue, Port};
    let dir = Directory::new();
    let path = dir.0.join("small.json");
    fs::write(&path, cam_gui1::compute::SMALL).unwrap();
    let port = Port::default();
    port.drop_file(
        41,
        egui::DroppedFile {
            path: Some(path),
            ..Default::default()
        },
        egui::Context::default(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(event) = port.poll() {
            let Event::Io {
                id: 41,
                result: Ok(IoValue::Job(json)),
            } = event
            else {
                panic!("Unexpected drop event: {event:?}")
            };
            assert_eq!(json, cam_gui1::compute::SMALL);
            let (meta, payload) =
                cam_gui1::compute::run(cam_gui1::compute::Request::Open { json }).unwrap();
            let scene = cam_gui1::compute::Scene {
                meta,
                payload: std::sync::Arc::new(payload),
            };
            assert_eq!(scene.motion_count(), 37);
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Drop read timed out");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
#[test]
fn real_store_recovers_raw_text_and_exact_job_and_rejects_stale_writers() {
    let dir = Directory::new();
    let a = Store::new(dir.0.clone());
    let b = Store::new(dir.0.clone());
    assert_eq!(a.load().unwrap(), None);
    let original = snapshot();
    assert_eq!(a.save(None, original.clone()).unwrap(), 1);
    assert_eq!(b.load().unwrap().unwrap().snapshot, original);
    let mut updated = original.clone();
    updated.draft.raw.insert(updated.draft.key(0), "1.".into());
    assert_eq!(a.save(Some(1), updated.clone()).unwrap(), 2);
    assert!(
        b.save(Some(1), original)
            .unwrap_err()
            .contains("Revision conflict")
    );
    assert_eq!(a.load().unwrap().unwrap().snapshot, updated);
}
#[test]
fn os_lock_contention_and_corrupt_recovery_preserve_previous_data() {
    let dir = Directory::new();
    let store = Store::new(dir.0.clone());
    store.save(None, snapshot()).unwrap();
    let lock = fs::File::options()
        .read(true)
        .write(true)
        .open(dir.0.join("session.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert!(store.save(Some(1), snapshot()).is_err());
    drop(lock);
    assert_eq!(store.load().unwrap().unwrap().revision, 1);
    fs::write(dir.0.join("session.json"), b"broken").unwrap();
    assert!(store.load().is_err());
    assert!(store.save(Some(1), snapshot()).is_err());
    assert_eq!(fs::read(dir.0.join("session.json")).unwrap(), b"broken");
}
#[test]
fn atomic_save_retries_identical_checked_flower_bytes_after_real_destination_failure() {
    let dir = Directory::new();
    let scene = cam_gui1::compute::run(cam_gui1::compute::Request::Reference {
        flower: true,
        export: true,
    })
    .unwrap();
    assert_eq!(scene.0.programs.len(), 1);
    let bytes = scene.0.programs[0].gcode.as_bytes();
    let hash = cam_gui1::compute::hash(bytes);
    assert_eq!(
        hash,
        "c190feced004bb42e67a5da97e1c108b4897a750132a8008551bc1ce05d3997e"
    );
    let blocked = dir.0.join("occupied");
    fs::create_dir(&blocked).unwrap();
    assert!(atomic_write(&blocked, bytes).is_err());
    let destination = dir.0.join("combined.ngc");
    atomic_write(&destination, bytes).unwrap();
    assert_eq!(
        cam_gui1::compute::hash(&fs::read(&destination).unwrap()),
        hash
    );
    assert!(read_text(&destination, 2).is_err());
}
#[cfg(windows)]
#[test]
fn denied_replacement_keeps_existing_file_until_successful_retry() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = Directory::new();
    let path = dir.0.join("job.json");
    fs::write(&path, b"original").unwrap();
    let locked = fs::File::options()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    assert!(atomic_write(&path, b"replacement").is_err());
    drop(locked);
    assert_eq!(fs::read(&path).unwrap(), b"original");
    atomic_write(&path, b"replacement").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"replacement");
}
#[test]
fn recovery_admission_and_late_save_acknowledgement_do_not_erase_newer_edits() {
    let mut invalid = snapshot();
    invalid
        .draft
        .raw
        .insert("101/1/not-a-field".into(), "hidden".into());
    assert!(invalid.validate().is_err());
    assert!(Stored::decode("{\"revision\":0,\"snapshot\":{}}").is_err());
    let mut tracker = Tracker {
        enabled: true,
        ready: true,
        ..Default::default()
    };
    tracker.changed(1.);
    assert!(tracker.due(2.));
    tracker.pending = Some(1);
    tracker.changed(2.);
    tracker.written(1, Ok(1));
    assert_eq!(tracker.saved_edit, 1);
    assert!(tracker.due(3.));
    tracker.pending = Some(2);
    tracker.written(2, Err("quota exceeded".into()));
    assert!(!tracker.due(9.));
    assert_eq!(tracker.saved_edit, 1);
    tracker.failed = false;
    assert!(tracker.due(9.));
}
