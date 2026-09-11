use crate::recovery::{Snapshot, Stored};
use crate::{
    compute::{Request, SceneMeta},
    state::Draft,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum IoValue {
    Job(String),
    Draft(Draft),
    Saved(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[allow(
    clippy::large_enum_variant,
    reason = "the computed variant carries the scene metadata; boxing it would churn every consumer"
)]
pub enum Event {
    Notice(String),
    OfflineStatus(String),
    Computed {
        id: u64,
        elapsed_ms: f64,
        result: Result<(SceneMeta, Vec<u8>), String>,
    },
    Cancelled {
        id: u64,
        stop_ms: f64,
    },
    Io {
        id: u64,
        result: Result<IoValue, String>,
    },
    RecoveryLoaded(Result<Option<Stored>, String>),
    RecoverySaved {
        edit: u64,
        result: Result<u64, String>,
    },
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };
    pub struct Port {
        tx: mpsc::Sender<Event>,
        rx: mpsc::Receiver<Event>,
        cancel: Option<Arc<AtomicBool>>,
        workers: Vec<std::thread::JoinHandle<()>>,
    }
    impl Default for Port {
        fn default() -> Self {
            let (tx, rx) = mpsc::channel();
            Self {
                tx,
                rx,
                cancel: None,
                workers: Vec::new(),
            }
        }
    }
    impl Drop for Port {
        fn drop(&mut self) {
            self.cancel();
            // Orderly application shutdown must let supervisors kill/reap their children.
            // Joining is confined to shutdown, never a frame or Cancel command.
            for worker in self.workers.drain(..) {
                let _ = worker.join();
            }
        }
    }
    impl Port {
        pub fn poll(&self) -> Option<Event> {
            self.rx.try_recv().ok()
        }
        pub fn cancel(&mut self) {
            if let Some(c) = self.cancel.take() {
                c.store(true, Ordering::SeqCst);
            }
        }
        pub fn start(&mut self, id: u64, request: Request, ctx: egui::Context) {
            self.cancel();
            self.workers.retain(|worker| !worker.is_finished());
            let cancel = Arc::new(AtomicBool::new(false));
            self.cancel = Some(cancel.clone());
            let tx = self.tx.clone();
            let worker = std::thread::spawn(move || {
                let begin = Instant::now();
                let path =
                    std::env::temp_dir().join(format!("cam-gui1-{}-{id}.json", std::process::id()));
                let output = path.with_extension("result.json");
                let mut cancelled = None;
                let result = (|| -> Result<(SceneMeta, Vec<u8>), String> {
                    std::fs::write(
                        &path,
                        serde_json::to_vec(&request).map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                    let mut command = std::process::Command::new(
                        std::env::current_exe().map_err(|e| e.to_string())?,
                    );
                    command.arg("--worker").arg(&path).arg(&output);
                    #[cfg(windows)]
                    {
                        use std::os::windows::process::CommandExt;
                        command.creation_flags(0x08000000); // CREATE_NO_WINDOW, including disposable helpers.
                    }
                    let mut child = command.spawn().map_err(|e| e.to_string())?;
                    loop {
                        if cancel.load(Ordering::SeqCst) {
                            let stopped = Instant::now();
                            child.kill().map_err(|e| e.to_string())?;
                            child.wait().map_err(|e| e.to_string())?;
                            cancelled = Some(stopped.elapsed().as_secs_f64() * 1000.);
                            return Err("Cancelled".into());
                        }
                        match child.try_wait() {
                            Ok(Some(status)) => {
                                if !status.success() {
                                    return Err(format!(
                                        "Compute process exited {status}; result retained"
                                    ));
                                }
                                let bytes = std::fs::read(&output).map_err(|e| e.to_string())?;
                                if bytes.len() > 512_000_000 {
                                    return Err(
                                        "Worker result exceeded 512 MB transport admission limit"
                                            .into(),
                                    );
                                }
                                // `u32 metadata length | metadata JSON | binary payload`
                                let (metadata, payload) = crate::pages::parse_message(bytes)?;
                                let meta: Result<SceneMeta, String> =
                                    serde_json::from_slice(&metadata).map_err(|e| e.to_string())?;
                                return Ok((meta?, payload));
                            }
                            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                            Err(e) => {
                                let _ = child.kill();
                                let _ = child.wait();
                                return Err(e.to_string());
                            }
                        }
                    }
                })();
                let _ = std::fs::remove_file(path);
                let _ = std::fs::remove_file(output);
                let event = if let Some(stop_ms) = cancelled {
                    Event::Cancelled { id, stop_ms }
                } else {
                    Event::Computed {
                        id,
                        elapsed_ms: begin.elapsed().as_secs_f64() * 1000.,
                        result,
                    }
                };
                let _ = tx.send(event);
                ctx.request_repaint();
            });
            self.workers.push(worker);
        }
        pub fn open(&self, id: u64, recovery: bool, ctx: egui::Context) {
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let value = (|| -> Result<String, String> {
                    let path = rfd::FileDialog::new()
                        .add_filter("JSON", &["json"])
                        .pick_file()
                        .ok_or("Open cancelled")?;
                    crate::file_io::read_text(&path, 8_000_000)
                })();
                let result = if recovery {
                    value.and_then(|v| Draft::recover(&v)).map(IoValue::Draft)
                } else {
                    value.map(IoValue::Job)
                };
                let _ = tx.send(Event::Io { id, result });
                ctx.request_repaint();
            });
        }
        pub fn save(&self, id: u64, name: String, bytes: Vec<u8>, deny: bool, ctx: egui::Context) {
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let result = (|| -> Result<String, String> {
                    if deny {
                        return Err("Injected denied write; retained bytes and draft remain available for retry".into());
                    }
                    let path = rfd::FileDialog::new()
                        .set_file_name(&name)
                        .save_file()
                        .ok_or("Save cancelled; bytes retained")?;
                    crate::file_io::atomic_write(&path, &bytes)?;
                    Ok(format!(
                        "Saved {} exact bytes; SHA256 {}",
                        bytes.len(),
                        crate::compute::hash(&bytes)
                    ))
                })();
                let _ = tx.send(Event::Io {
                    id,
                    result: result.map(IoValue::Saved),
                });
                ctx.request_repaint();
            });
        }
        pub fn drop_file(&self, id: u64, file: egui::DroppedFile, ctx: egui::Context) {
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let result = if let Some(path) = file.path {
                    crate::file_io::read_text(&path, 8_000_000)
                } else if let Some(bytes) = file.bytes {
                    if bytes.len() > 8_000_000 {
                        Err("Drop exceeds 8 MB".into())
                    } else {
                        String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string())
                    }
                } else {
                    Err("Dropped file is unavailable".into())
                };
                let _ = tx.send(Event::Io {
                    id,
                    result: result.map(IoValue::Job),
                });
                ctx.request_repaint();
            });
        }
        pub fn load_recovery(&self, ctx: egui::Context) {
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let result = crate::file_io::Store::default_location().and_then(|s| s.load());
                let _ = tx.send(Event::RecoveryLoaded(result));
                ctx.request_repaint();
            });
        }
        pub fn save_recovery(
            &self,
            edit: u64,
            expected: Option<u64>,
            snapshot: Snapshot,
            ctx: egui::Context,
        ) {
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let result = crate::file_io::Store::default_location()
                    .and_then(|s| s.save(expected, snapshot));
                let _ = tx.send(Event::RecoverySaved { edit, result });
                ctx.request_repaint();
            });
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub use native::Port;

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use std::cell::RefCell;
    use wasm_bindgen::prelude::*;
    thread_local! {
        static EVENTS: RefCell<std::collections::VecDeque<Event>> = const { RefCell::new(std::collections::VecDeque::new()) };
        static CONTEXT: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
        static PAYLOAD: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    }
    #[wasm_bindgen(module = "/web/ports.js")]
    extern "C" {
        fn startWorker(id: f64, request: &str);
        fn cancelWorker();
        fn openFile(id: f64, recovery: bool);
        fn saveFile(id: f64, name: &str, bytes: &[u8], deny: bool);
        fn loadRecovery();
        fn saveRecovery(edit: f64, expected: &str, snapshot: &str);
    }
    /// Binary scene payload for the next computed event. The worker transfers
    /// an `ArrayBuffer`, so the heavy part never becomes a JSON string.
    #[wasm_bindgen]
    pub fn receive_payload(bytes: &[u8]) {
        PAYLOAD.with(|payload| *payload.borrow_mut() = Some(bytes.to_vec()));
    }

    #[wasm_bindgen]
    pub fn receive_event(text: &str) {
        let event = match binary_event(text) {
            Some(event) => event,
            None => serde_json::from_str(text).unwrap_or_else(|e| {
                Event::RecoveryLoaded(Err(format!("Invalid platform event: {e}")))
            }),
        };
        EVENTS.with(|q| q.borrow_mut().push_back(event));
        CONTEXT.with(|c| {
            if let Some(ctx) = c.borrow().as_ref() {
                ctx.request_repaint();
            }
        });
    }

    /// `{"ComputedBinary":{id,elapsed_ms,meta:<metadata JSON>}}` plus a payload
    /// already handed over through `receive_payload`.
    fn binary_event(text: &str) -> Option<Event> {
        let value: serde_json::Value = serde_json::from_str(text).ok()?;
        let computed = value.get("ComputedBinary")?;
        let id = computed.get("id")?.as_f64()? as u64;
        let elapsed_ms = computed.get("elapsed_ms")?.as_f64()?;
        let meta = computed.get("meta")?.clone();
        let payload = PAYLOAD.with(|payload| payload.borrow_mut().take());
        let result = match (serde_json::from_value::<SceneMeta>(meta), payload) {
            (Ok(meta), Some(payload)) => Ok((meta, payload)),
            (Ok(_), None) => Err("Worker payload was not transferred".into()),
            (Err(error), _) => Err(format!("Worker metadata was invalid: {error}")),
        };
        Some(Event::Computed {
            id,
            elapsed_ms,
            result,
        })
    }
    #[derive(Default)]
    pub struct Port;
    impl Port {
        pub fn poll(&self) -> Option<Event> {
            EVENTS.with(|q| q.borrow_mut().pop_front())
        }
        pub fn cancel(&mut self) {
            cancelWorker();
        }
        pub fn start(&mut self, id: u64, request: Request, ctx: egui::Context) {
            CONTEXT.with(|c| *c.borrow_mut() = Some(ctx));
            startWorker(
                id as f64,
                &serde_json::to_string(&request).expect("request serializes"),
            );
        }
        pub fn open(&self, id: u64, recovery: bool, ctx: egui::Context) {
            CONTEXT.with(|c| *c.borrow_mut() = Some(ctx));
            openFile(id as f64, recovery);
        }
        pub fn save(&self, id: u64, name: String, bytes: Vec<u8>, deny: bool, ctx: egui::Context) {
            CONTEXT.with(|c| *c.borrow_mut() = Some(ctx));
            saveFile(id as f64, &name, &bytes, deny);
        }
        pub fn drop_file(&self, id: u64, file: egui::DroppedFile, ctx: egui::Context) {
            CONTEXT.with(|c| *c.borrow_mut() = Some(ctx));
            let result = file
                .bytes
                .ok_or_else(|| "Dropped file bytes unavailable".into())
                .and_then(|b| {
                    if b.len() > 8_000_000 {
                        return Err("Drop exceeds 8 MB".into());
                    }
                    std::str::from_utf8(&b)
                        .map(str::to_owned)
                        .map_err(|e| e.to_string())
                })
                .map(IoValue::Job);
            EVENTS.with(|q| q.borrow_mut().push_back(Event::Io { id, result }));
        }
        pub fn load_recovery(&self, ctx: egui::Context) {
            CONTEXT.with(|c| *c.borrow_mut() = Some(ctx));
            loadRecovery();
        }
        pub fn save_recovery(
            &self,
            edit: u64,
            expected: Option<u64>,
            snapshot: Snapshot,
            ctx: egui::Context,
        ) {
            CONTEXT.with(|c| *c.borrow_mut() = Some(ctx));
            saveRecovery(
                edit as f64,
                &serde_json::to_string(&expected).unwrap(),
                &serde_json::to_string(&snapshot).unwrap(),
            );
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub use browser::Port;
