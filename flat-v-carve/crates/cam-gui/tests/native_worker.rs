#![cfg(not(target_arch = "wasm32"))]
use cam_gui_runtime::{
    compute::{self, Request, SceneMeta},
    session::{self as gui, Command},
};
use std::{
    path::PathBuf,
    process::{Child, Command as Process},
    time::{Duration, Instant},
};
struct Worker {
    child: Child,
    folder: PathBuf,
}
impl Worker {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let folder = std::env::temp_dir().join(format!(
            "gui2-ipc-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&folder).unwrap();
        let mut command = Process::new(env!("CARGO_BIN_EXE_cam-gui"));
        command.arg("--worker").arg(&folder);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        Self {
            child: command.spawn().unwrap(),
            folder,
        }
    }
    fn send(&self, request: Request) {
        std::fs::write(
            self.folder.join("pending"),
            serde_json::to_vec(&request).unwrap(),
        )
        .unwrap();
        std::fs::rename(self.folder.join("pending"), self.folder.join("request")).unwrap();
    }
    fn request(&self, command: Command) -> (SceneMeta, Vec<u8>) {
        self.send(Request::Gui2(command));
        let start = Instant::now();
        let response = self.folder.join("response");
        while !response.exists() {
            assert!(start.elapsed() < Duration::from_secs(180));
            std::thread::sleep(Duration::from_millis(10));
        }
        let bytes = std::fs::read(&response).unwrap();
        std::fs::remove_file(response).unwrap();
        let (meta, payload) = compute::parse_worker_message(bytes).unwrap();
        (meta.unwrap(), payload)
    }
}

#[test]
fn native_resource_commands_copy_profiles_generate_and_prepare() {
    use cam_gui_runtime::resources::{Catalog, ResourceCommand};
    let worker = Worker::new();
    let mut job = include_str!("../../../fixtures/gui4/lettering.job.json").to_string();
    let catalog = Catalog::decode(include_str!("../../../fixtures/gui5/library.json")).unwrap();
    for (role, tool, preset) in [
        (
            cam_core::project::v5::resources::AssignmentRole::Endmill,
            "endmill",
            "rough",
        ),
        (
            cam_core::project::v5::resources::AssignmentRole::Vbit,
            "vbit",
            "finish",
        ),
    ] {
        let (copied, _) = worker.request(Command::Resource {
            job,
            action: Box::new(ResourceCommand::Apply {
                operation: "carving".into(),
                role,
                catalog: catalog.clone(),
                tool: tool.into(),
                preset: preset.into(),
            }),
        });
        assert_eq!(copied.report["gui2"]["kind"], "resource");
        job = copied.job;
    }
    let mut doc = gui::open(&job).unwrap();
    cam_gui_runtime::authoring::set_mode(&mut doc, cam_core::project::FlatVcarveMode::Combined);
    job = doc.to_json().unwrap();
    let (generated, _) = worker.request(Command::Generate { job: job.clone() });
    let handle = generated.report["gui2"]["handle"]
        .as_str()
        .unwrap()
        .to_string();
    let (seek, _) = worker.request(Command::Seek {
        handle: handle.clone(),
        prefix: generated.motions,
    });
    assert_eq!(seek.report["gui2"]["prefix"], generated.motions);
    let (prepared, _) = worker.request(Command::Prepare { job, handle });
    assert_eq!(prepared.report["gui2"]["kind"], "prepared");
}
#[test]
fn native_collection_commands_generate_seek_prepare_and_reopen() {
    use cam_gui_runtime::authoring;
    let worker = Worker::new();
    let (opened, _) = worker.request(Command::Open {
        json: include_str!("../../../fixtures/gui4/lettering.job.json").into(),
    });
    let (added, _) = worker.request(Command::Artwork {
        job: opened.job,
        action: gui::ArtworkCommand::Add {
            filename: "second.svg".into(),
            svg: include_str!("../../../fixtures/gui4/second.svg").into(),
        },
    });
    let mut job = gui::open(&added.job).unwrap();
    assert_eq!(job.artwork.len(), 2);
    job.artwork[1].placement.origin_mm = cam_core::geometry::Point::new(-25., -3.);
    let catalogue = cam_core::project::v5::artwork::inspect_artwork(&job).unwrap();
    authoring::settings_mut(&mut job).components = authoring::catalogue_components(&catalogue)
        .into_iter()
        .filter(|c| c.reference.local_geometry_id == "letter-l::0")
        .map(|c| c.reference)
        .collect();
    authoring::set_mode(&mut job, cam_core::project::FlatVcarveMode::Combined);
    let portable = job.to_json().unwrap();
    let (generated, _) = worker.request(Command::Generate {
        job: portable.clone(),
    });
    assert_eq!(generated.report["gui2"]["checks"]["exportReady"], true);
    let handle = generated.report["gui2"]["handle"]
        .as_str()
        .unwrap()
        .to_string();
    let (seek, _) = worker.request(Command::Seek {
        handle: handle.clone(),
        prefix: generated.motions,
    });
    assert_eq!(seek.report["gui2"]["prefix"], generated.motions);
    let (output, _) = worker.request(Command::Prepare {
        job: portable.clone(),
        handle,
    });
    let file = &output.report["gui2"]["file"];
    assert_eq!(
        compute::hash(file["gcode"].as_str().unwrap().as_bytes()),
        file["sha256"]
    );
    let (reopened, _) = worker.request(Command::Open {
        json: portable.clone(),
    });
    assert_eq!(reopened.job, portable);
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        for file in [
            "request",
            "pending",
            "result",
            "response",
            "saved.job.json",
            "checked.ngc",
        ] {
            let _ = std::fs::remove_file(self.folder.join(file));
        }
        let _ = std::fs::remove_dir(&self.folder);
    }
}

#[test]
fn persistent_native_process_open_edit_generate_seek_prepare_save_reopen_and_stop() {
    let mut worker = Worker::new();
    let (imported, _) = worker.request(Command::ImportSvg {
        filename: "new-carving.svg".into(),
        svg: include_str!("../../../fixtures/gui2/new-carving.svg").into(),
    });
    let incomplete = gui::open(&imported.job).unwrap();
    assert_eq!(incomplete.setup.stock.thickness_mm, Some(18.));
    assert_eq!(incomplete.setup.clearance_above_stock_mm, Some(5.));
    assert!(incomplete.setup.stock.xy.is_some());
    assert!(gui::settings(&incomplete).components.is_empty());
    let (preview, _) = worker.request(Command::Preview {
        job: imported.job.clone(),
    });
    assert_eq!(
        preview.report["gui2"]["components"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let (reopened, _) = worker.request(Command::Open { json: imported.job });
    assert_eq!(gui::open(&reopened.job).unwrap(), incomplete);
    let (opened, _) = worker.request(Command::Open {
        json: gui::FLOWER.into(),
    });
    assert!(opened.contour_vertices > 0);
    let job =
        cam_gui_runtime::app::set_value(&gui::open(&opened.job).unwrap(), 2, Some(1900.)).unwrap();
    let (applied, _) = worker.request(Command::ApplyProfile {
        job: job.to_json().unwrap(),
        json: gui::PROFILE.into(),
    });
    let (generated, _) = worker.request(Command::Generate {
        job: applied.job.clone(),
    });
    let handle = generated.report["gui2"]["handle"]
        .as_str()
        .unwrap()
        .to_string();
    let (seek, _) = worker.request(Command::Seek {
        handle: handle.clone(),
        prefix: 1234,
    });
    assert_eq!(seek.report["gui2"]["prefix"], 1234);
    let (prepared, _) = worker.request(Command::Prepare {
        job: applied.job.clone(),
        handle,
    });
    assert_eq!(prepared.report["gui2"]["retained"]["plansRun"], 1);
    let bytes = prepared.report["gui2"]["file"]["gcode"]
        .as_str()
        .unwrap()
        .as_bytes();
    assert!(cam_gui_runtime::file_io::atomic_write(&worker.folder, bytes).is_err());
    let path = worker.folder.join("checked.ngc");
    cam_gui_runtime::file_io::atomic_write(&path, bytes).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    let path = worker.folder.join("saved.job.json");
    cam_gui_runtime::file_io::atomic_write(&path, applied.job.as_bytes()).unwrap();
    let (reopened, _) = worker.request(Command::Open {
        json: std::fs::read_to_string(path).unwrap(),
    });
    assert_eq!(reopened.job, applied.job);
    worker.send(Request::Busy);
    std::thread::sleep(Duration::from_millis(100));
    let begin = Instant::now();
    worker.child.kill().unwrap();
    worker.child.wait().unwrap();
    assert!(begin.elapsed() < Duration::from_secs(2));
}
