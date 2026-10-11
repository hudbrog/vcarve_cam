//! A separate disposable solver process/Worker; no CAM request or saved trust.
use cam_core::sketch::{ConstraintEvaluation, SketchDocument};
use cam_sketch_solver::DragPoint;
use serde::{Deserialize, Serialize};

pub const MAX_REQUEST_BYTES: usize = 2_500_000;
pub const MAX_REPLY_BYTES: usize = 4_000_000;
pub const TIMEOUT_MS: f64 = 10_000.;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: u64,
    pub revision: u64,
    pub model: SketchDocument,
    pub targets: Vec<DragPoint>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Issue {
    pub code: String,
    pub source_id: Option<String>,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Outcome {
    pub accepted: bool,
    pub candidate: SketchDocument,
    pub degrees_of_freedom: Option<usize>,
    pub iterations: usize,
    pub status: String,
    pub issues: Vec<Issue>,
    pub constraints: Vec<ConstraintEvaluation>,
    pub drag_error_mm: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reply {
    pub id: u64,
    pub revision: u64,
    pub result: Result<Outcome, String>,
}
pub fn execute(request: Request) -> Reply {
    let result =
        cam_sketch_solver::solve_drag_with_cancel(&request.model, &request.targets, || false)
            .map(|report| Outcome {
                accepted: report.accepted(),
                drag_error_mm: (!request.targets.is_empty()).then(|| {
                    request
                        .targets
                        .iter()
                        .filter_map(|t| {
                            report
                                .candidate
                                .point(&t.point)
                                .ok()
                                .map(|p| p.distance(t.target))
                        })
                        .fold(0., f64::max)
                }),
                candidate: report.candidate,
                degrees_of_freedom: report.degrees_of_freedom,
                iterations: report.iterations,
                status: report.solver_status,
                constraints: report.validation.constraints,
                issues: report
                    .validation
                    .issues
                    .into_iter()
                    .map(|d| Issue {
                        code: d.code,
                        source_id: d.source_id,
                        message: d.message,
                    })
                    .collect(),
            })
            .map_err(|e| e.to_string());
    Reply {
        id: request.id,
        revision: request.revision,
        result,
    }
}
pub fn run_json(input: &str) -> Result<String, String> {
    if input.len() > MAX_REQUEST_BYTES {
        return Err("Solver request exceeds its size limit".into());
    }
    let request: Request = serde_json::from_str(input).map_err(|e| e.to_string())?;
    let reply = serde_json::to_string(&execute(request)).map_err(|e| e.to_string())?;
    if reply.len() > MAX_REPLY_BYTES {
        return Err("Solver response exceeds its size limit".into());
    }
    Ok(reply)
}

#[cfg(test)]
mod limits_tests {
    use super::*;
    #[test]
    fn transport_and_entity_limits_reject_before_solving_and_allow_retry() {
        assert!(
            run_json(&" ".repeat(MAX_REQUEST_BYTES + 1))
                .unwrap_err()
                .contains("size limit")
        );
        assert!(run_json("{}").is_err());
        let mut model: SketchDocument =
            serde_json::from_str(include_str!("../../../../fixtures/sketch/rectangle.json"))
                .unwrap();
        model.entities = vec![model.entities[0].clone(); cam_core::sketch::MAX_ENTITIES + 1];
        let reply = execute(Request {
            id: 2,
            revision: 3,
            model,
            targets: vec![],
        });
        assert_eq!((reply.id, reply.revision), (2, 3));
        assert!(reply.result.unwrap_err().contains("budget"));
        let model =
            serde_json::from_str(include_str!("../../../../fixtures/sketch/rectangle.json"))
                .unwrap();
        let reply: Reply = serde_json::from_str(
            &run_json(
                &serde_json::to_string(&Request {
                    id: 4,
                    revision: 5,
                    model,
                    targets: vec![],
                })
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(reply.result.unwrap().accepted);
    }
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
        time::Duration,
    };
    struct Session {
        #[cfg(not(test))]
        send: mpsc::Sender<Request>,
        stop: Arc<AtomicBool>,
        join: Option<std::thread::JoinHandle<()>>,
    }
    impl Drop for Session {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
        }
    }
    pub struct Port {
        tx: mpsc::Sender<Reply>,
        rx: mpsc::Receiver<Reply>,
        session: Option<Session>,
        stopped: Vec<Session>,
    }
    impl Default for Port {
        fn default() -> Self {
            let (tx, rx) = mpsc::channel();
            Self {
                tx,
                rx,
                session: None,
                stopped: vec![],
            }
        }
    }
    impl Port {
        pub fn poll(&self) -> Option<Reply> {
            self.rx.try_recv().ok()
        }
        pub fn cancel(&mut self) {
            if let Some(s) = self.session.take() {
                s.stop.store(true, Ordering::SeqCst);
                self.stopped.push(s);
            }
        }
        pub fn start(&mut self, request: Request, ctx: egui::Context) -> Result<(), String> {
            #[cfg(test)]
            {
                let _ = self.tx.send(execute(request));
                ctx.request_repaint();
                Ok(())
            }
            #[cfg(not(test))]
            {
                self.stopped
                    .retain(|s| !s.join.as_ref().is_none_or(|j| j.is_finished()));
                if self
                    .session
                    .as_ref()
                    .is_some_and(|s| s.join.as_ref().is_none_or(|j| j.is_finished()))
                {
                    self.session = None;
                }
                if self.session.is_none() {
                    let (send, rx) = mpsc::channel::<Request>();
                    let stop = Arc::new(AtomicBool::new(false));
                    let flag = stop.clone();
                    let replies = self.tx.clone();
                    let join = std::thread::spawn(move || {
                        let nonce = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_nanos();
                        let folder = std::env::temp_dir()
                            .join(format!("cam-sketch-{}-{nonce}", std::process::id()));
                        let mut child = None;
                        let setup = (|| -> Result<(), String> {
                            std::fs::create_dir(&folder).map_err(|e| e.to_string())?;
                            let mut cmd = std::process::Command::new(
                                std::env::current_exe().map_err(|e| e.to_string())?,
                            );
                            cmd.arg("--sketch-worker").arg(&folder);
                            #[cfg(windows)]
                            {
                                use std::os::windows::process::CommandExt;
                                cmd.creation_flags(0x08000000);
                            }
                            child = Some(cmd.spawn().map_err(|e| e.to_string())?);
                            Ok(())
                        })();
                        while !flag.load(Ordering::SeqCst) {
                            let request = match rx.recv_timeout(Duration::from_millis(10)) {
                                Ok(r) => r,
                                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                                Err(_) => break,
                            };
                            let result = (|| -> Result<Reply, String> {
                                setup.clone()?;
                                let input =
                                    serde_json::to_vec(&request).map_err(|e| e.to_string())?;
                                if input.len() > MAX_REQUEST_BYTES {
                                    return Err("Solver request is too large".into());
                                }
                                std::fs::write(folder.join("pending"), input)
                                    .map_err(|e| e.to_string())?;
                                std::fs::rename(folder.join("pending"), folder.join("request"))
                                    .map_err(|e| e.to_string())?;
                                let timer = std::time::Instant::now();
                                loop {
                                    if flag.load(Ordering::SeqCst) {
                                        return Err("Solve cancelled".into());
                                    }
                                    if timer.elapsed().as_secs_f64() * 1000. > TIMEOUT_MS {
                                        return Err("Solver timed out; draft retained".into());
                                    }
                                    if child
                                        .as_mut()
                                        .unwrap()
                                        .try_wait()
                                        .map_err(|e| e.to_string())?
                                        .is_some()
                                    {
                                        return Err("Solver process exited; retry Solve".into());
                                    }
                                    let output = folder.join("response");
                                    if output.exists() {
                                        if std::fs::metadata(&output)
                                            .map_err(|e| e.to_string())?
                                            .len()
                                            > MAX_REPLY_BYTES as u64
                                        {
                                            return Err("Solver response is too large".into());
                                        }
                                        let bytes =
                                            std::fs::read(&output).map_err(|e| e.to_string())?;
                                        std::fs::remove_file(output).map_err(|e| e.to_string())?;
                                        return serde_json::from_slice(&bytes)
                                            .map_err(|e| e.to_string());
                                    }
                                    std::thread::sleep(Duration::from_millis(5));
                                }
                            })();
                            let failed = result.is_err();
                            if !flag.load(Ordering::SeqCst) {
                                let reply = result.unwrap_or_else(|e| Reply {
                                    id: request.id,
                                    revision: request.revision,
                                    result: Err(e),
                                });
                                let _ = replies.send(reply);
                                ctx.request_repaint();
                            }
                            if failed {
                                break;
                            }
                        }
                        if let Some(mut child) = child {
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                        for file in ["pending", "request", "result", "response"] {
                            let _ = std::fs::remove_file(folder.join(file));
                        }
                        let _ = std::fs::remove_dir(folder);
                    });
                    self.session = Some(Session {
                        send,
                        stop,
                        join: Some(join),
                    });
                }
                self.session
                    .as_ref()
                    .unwrap()
                    .send
                    .send(request)
                    .map_err(|e| e.to_string())
            }
        }
    }
    pub fn serve(folder: std::path::PathBuf) -> Result<(), String> {
        loop {
            let input = folder.join("request");
            if !input.exists() {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            if std::fs::metadata(&input).map_err(|e| e.to_string())?.len()
                > MAX_REQUEST_BYTES as u64
            {
                return Err("Oversized solver request".into());
            }
            let text = std::fs::read_to_string(&input).map_err(|e| e.to_string())?;
            std::fs::remove_file(input).map_err(|e| e.to_string())?;
            let reply = run_json(&text)?;
            std::fs::write(folder.join("result"), reply).map_err(|e| e.to_string())?;
            std::fs::rename(folder.join("result"), folder.join("response"))
                .map_err(|e| e.to_string())?;
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
pub use native::{Port, serve};

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};
    use wasm_bindgen::{JsCast, prelude::*};
    struct Running {
        worker: web_sys::Worker,
        _message: Closure<dyn FnMut(web_sys::MessageEvent)>,
        _error: Closure<dyn FnMut(web_sys::ErrorEvent)>,
    }
    impl Drop for Running {
        fn drop(&mut self) {
            self.worker.set_onmessage(None);
            self.worker.set_onerror(None);
            self.worker.terminate();
        }
    }
    #[derive(Default)]
    pub struct Port {
        queue: Rc<RefCell<VecDeque<Reply>>>,
        current: Rc<RefCell<(u64, u64)>>,
        running: Option<Running>,
    }
    impl Port {
        pub fn poll(&self) -> Option<Reply> {
            self.queue.borrow_mut().pop_front()
        }
        pub fn cancel(&mut self) {
            self.running = None;
        }
        pub fn start(&mut self, request: Request, ctx: egui::Context) -> Result<(), String> {
            *self.current.borrow_mut() = (request.id, request.revision);
            if self.running.is_none() {
                let options = web_sys::WorkerOptions::new();
                options.set_type(web_sys::WorkerType::Module);
                let worker = web_sys::Worker::new_with_options("./sketch-worker.js", &options)
                    .map_err(|e| format!("Cannot start solver worker: {e:?}"))?;
                let queue = self.queue.clone();
                let repaint = ctx.clone();
                let current = self.current.clone();
                let message = Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
                    let (id, revision) = *current.borrow();
                    let result = event
                        .data()
                        .as_string()
                        .ok_or("Invalid solver response".to_owned())
                        .and_then(|s| {
                            if s.len() > MAX_REPLY_BYTES {
                                Err("Solver response is too large".into())
                            } else {
                                serde_json::from_str::<Reply>(&s).map_err(|e| e.to_string())
                            }
                        });
                    queue
                        .borrow_mut()
                        .push_back(result.unwrap_or_else(|e| Reply {
                            id,
                            revision,
                            result: Err(e),
                        }));
                    repaint.request_repaint();
                }) as Box<dyn FnMut(_)>);
                let queue = self.queue.clone();
                let current = self.current.clone();
                let error = Closure::wrap(Box::new(move |event: web_sys::ErrorEvent| {
                    event.prevent_default();
                    let (id, revision) = *current.borrow();
                    queue.borrow_mut().push_back(Reply {
                        id,
                        revision,
                        result: Err(format!("Solver worker failed: {}", event.message())),
                    });
                    ctx.request_repaint();
                }) as Box<dyn FnMut(_)>);
                worker.set_onmessage(Some(message.as_ref().unchecked_ref()));
                worker.set_onerror(Some(error.as_ref().unchecked_ref()));
                self.running = Some(Running {
                    worker,
                    _message: message,
                    _error: error,
                });
            }
            let text = serde_json::to_string(&request).map_err(|e| e.to_string())?;
            if text.len() > MAX_REQUEST_BYTES {
                return Err("Solver request is too large".into());
            }
            self.running
                .as_ref()
                .unwrap()
                .worker
                .post_message(&JsValue::from_str(&text))
                .map_err(|e| format!("Cannot send solver request: {e:?}"))
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub use browser::Port;
