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
        let folder = std::env::temp_dir().join(format!("gui2-ipc-test-{}", std::process::id()));
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
