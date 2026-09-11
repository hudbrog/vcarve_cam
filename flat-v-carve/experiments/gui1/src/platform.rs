use crate::{
    compute::{Request, Scene},
    state::Draft,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum Event {
    Computed {
        id: u64,
        elapsed_ms: f64,
        result: Result<Scene, String>,
    },
    Cancelled {
        id: u64,
        stop_ms: f64,
    },
    Opened(Result<String, String>),
    Recovered(Result<Draft, String>),
    Saved(Result<String, String>),
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use std::{
        io::Read,
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
                let result = (|| -> Result<Scene, String> {
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
                                let file =
                                    std::fs::File::open(&output).map_err(|e| e.to_string())?;
                                if file.metadata().map_err(|e| e.to_string())?.len() > 256_000_000 {
                                    return Err(
                                        "Worker result exceeded 256 MB transport admission limit"
                                            .into(),
                                    );
                                }
                                return serde_json::from_reader(std::io::BufReader::new(file))
                                    .map_err(|e| e.to_string())?;
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
        pub fn open(&self, recovery: bool, ctx: egui::Context) {
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let value = (|| -> Result<String, String> {
                    let path = rfd::FileDialog::new()
                        .add_filter("JSON", &["json"])
                        .pick_file()
                        .ok_or("Open cancelled")?;
                    let mut text = String::new();
                    std::fs::File::open(path)
                        .map_err(|e| e.to_string())?
                        .take(8_000_001)
                        .read_to_string(&mut text)
                        .map_err(|e| e.to_string())?;
                    if text.len() > 8_000_000 {
                        return Err("Input exceeds 8 MB spike limit".into());
                    }
                    Ok(text)
                })();
                let event = if recovery {
                    Event::Recovered(value.and_then(|v| Draft::recover(&v)))
                } else {
                    Event::Opened(value)
                };
                let _ = tx.send(event);
                ctx.request_repaint();
            });
        }
        pub fn save(&self, name: String, bytes: Vec<u8>, deny: bool, ctx: egui::Context) {
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
                    std::fs::write(path, &bytes).map_err(|e| e.to_string())?;
                    Ok(format!(
                        "Saved {} exact bytes; SHA256 {}",
                        bytes.len(),
                        crate::compute::hash(&bytes)
                    ))
                })();
                let _ = tx.send(Event::Saved(result));
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
    }
    #[wasm_bindgen(module = "/web/ports.js")]
    extern "C" {
        fn startWorker(id: f64, request: &str);
        fn cancelWorker();
        fn openFile(recovery: bool);
        fn saveFile(name: &str, bytes: &[u8], deny: bool);
    }
    #[wasm_bindgen]
    pub fn receive_event(text: &str) {
        let event = serde_json::from_str(text)
            .unwrap_or_else(|e| Event::Saved(Err(format!("Invalid platform event: {e}"))));
        EVENTS.with(|q| q.borrow_mut().push_back(event));
        CONTEXT.with(|c| {
            if let Some(ctx) = c.borrow().as_ref() {
                ctx.request_repaint();
            }
        });
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
        pub fn open(&self, recovery: bool, ctx: egui::Context) {
            CONTEXT.with(|c| *c.borrow_mut() = Some(ctx));
            openFile(recovery);
        }
        pub fn save(&self, name: String, bytes: Vec<u8>, deny: bool, ctx: egui::Context) {
            CONTEXT.with(|c| *c.borrow_mut() = Some(ctx));
            saveFile(&name, &bytes, deny);
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub use browser::Port;
