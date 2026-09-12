use crate::recovery::{Snapshot, Stored};
use crate::{
    compute::{Request, SceneMeta},
    state::Draft,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum IoValue {
    Job(String),
    Svg { filename: String, svg: String },
    Svgs(Vec<SvgFile>),
    Draft(Draft),
    Saved(String),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SvgFile {
    pub filename: String,
    pub content: Result<String, String>,
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
    Resources {
        id: u64,
        result: Result<Option<crate::resources::StoredCatalog>, String>,
    },
    RecoverySaved {
        edit: u64,
        result: Result<u64, String>,
    },
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use std::sync::mpsc;
    pub struct Port {
        tx: mpsc::Sender<Event>,
        rx: mpsc::Receiver<Event>,
        product: Option<crate::worker::Session>,
        stopped: Vec<crate::worker::Session>,
    }
    impl Default for Port {
        fn default() -> Self {
            let (tx, rx) = mpsc::channel();
            Self {
                tx,
                rx,
                product: None,
                stopped: Vec::new(),
            }
        }
    }
    impl Drop for Port {
        fn drop(&mut self) {
            self.cancel();
        }
    }
    impl Port {
        pub fn poll(&self) -> Option<Event> {
            self.rx.try_recv().ok()
        }
        pub fn cancel(&mut self) {
            if let Some(mut product) = self.product.take() {
                product.stop();
                self.stopped.push(product);
            }
        }
        pub fn start(&mut self, id: u64, request: Request, ctx: egui::Context) {
            self.stopped.retain(|s| !s.finished());
            if self.product.as_ref().is_some_and(|s| s.finished()) {
                self.product = None;
            }
            self.product
                .get_or_insert_with(|| crate::worker::Session::new(self.tx.clone()))
                .start(id, request, ctx);
        }
        pub fn open(&self, id: u64, svg: bool, multiple: bool, ctx: egui::Context) {
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let result = (|| -> Result<IoValue, String> {
                    if multiple {
                        let paths = rfd::FileDialog::new()
                            .add_filter("SVG", &["svg"])
                            .pick_files()
                            .ok_or("Open cancelled")?;
                        if paths.len() > 128 {
                            return Err("Import at most 128 files together".into());
                        }
                        let mut bytes = 0;
                        let files = paths
                            .into_iter()
                            .map(|path| {
                                let content =
                                    crate::file_io::read_text(&path, 8_000_000).and_then(|text| {
                                        bytes += text.len();
                                        if bytes > 8_000_000 {
                                            Err("Batch exceeds 8 MB input budget".into())
                                        } else {
                                            Ok(text)
                                        }
                                    });
                                SvgFile {
                                    filename: path.file_name().unwrap().to_string_lossy().into(),
                                    content,
                                }
                            })
                            .collect();
                        return Ok(IoValue::Svgs(files));
                    }
                    let path = rfd::FileDialog::new()
                        .add_filter(
                            if svg { "SVG" } else { "JSON" },
                            if svg { &["svg"] } else { &["json"] },
                        )
                        .pick_file()
                        .ok_or("Open cancelled")?;
                    let text = crate::file_io::read_text(&path, 8_000_000)?;
                    Ok(if svg {
                        IoValue::Svg {
                            filename: path.file_name().unwrap().to_string_lossy().into(),
                            svg: text,
                        }
                    } else {
                        IoValue::Job(text)
                    })
                })();
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
            let filename = file
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| file.name.clone());
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
                    result: result.map(|text| dropped_value(filename, text)),
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
        pub fn resources(
            &self,
            id: u64,
            save: Option<(Option<u64>, crate::resources::Catalog)>,
            ctx: egui::Context,
        ) {
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let result = crate::file_io::Store::resources_location().and_then(|s| match save {
                    Some((expected, draft)) => s.save_resources(expected, draft).map(Some),
                    None => s.load_resources(),
                });
                let _ = tx.send(Event::Resources { id, result });
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
        fn openFile(id: f64, svg: bool, multiple: bool);
        fn saveFile(id: f64, name: &str, bytes: &[u8], deny: bool);
        fn loadRecovery();
        fn saveRecovery(edit: f64, expected: &str, snapshot: &str);
        fn resourceStore(id: f64, save: &str);
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
        // The browser worker returns the same `Result<SceneMeta, String>`
        // metadata document as the native worker.
        let result = match serde_json::from_value::<Result<SceneMeta, String>>(meta) {
            Ok(Ok(meta)) => match payload {
                Some(payload) => Ok((meta, payload)),
                None => Err("Worker payload was not transferred".into()),
            },
            Ok(Err(error)) => Err(error),
            Err(error) => Err(format!("Worker metadata was invalid: {error}")),
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
        pub fn resources(
            &self,
            id: u64,
            save: Option<(Option<u64>, crate::resources::Catalog)>,
            ctx: egui::Context,
        ) {
            CONTEXT.with(|c| *c.borrow_mut() = Some(ctx));
            resourceStore(id as f64, &serde_json::to_string(&save).unwrap());
        }
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
        pub fn open(&self, id: u64, svg: bool, multiple: bool, ctx: egui::Context) {
            CONTEXT.with(|c| *c.borrow_mut() = Some(ctx));
            openFile(id as f64, svg, multiple);
        }
        pub fn save(&self, id: u64, name: String, bytes: Vec<u8>, deny: bool, ctx: egui::Context) {
            CONTEXT.with(|c| *c.borrow_mut() = Some(ctx));
            saveFile(id as f64, &name, &bytes, deny);
        }
        pub fn drop_file(&self, id: u64, file: egui::DroppedFile, ctx: egui::Context) {
            let filename = file
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| file.name.clone());
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
                .map(|text| dropped_value(filename, text));
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

fn dropped_value(filename: String, text: String) -> IoValue {
    if filename.to_ascii_lowercase().ends_with(".svg") {
        IoValue::Svg {
            filename,
            svg: text,
        }
    } else {
        IoValue::Job(text)
    }
}
