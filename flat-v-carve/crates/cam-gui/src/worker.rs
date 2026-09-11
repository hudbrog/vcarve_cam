//! Native persistent, disposable compute process. A supervisor owns kill/reap;
//! the UI only signals cancellation. Each private mailbox accepts one command
//! at a time. No retained execution ever crosses back into the child.
use crate::{
    compute::{Request, SceneMeta},
    platform::Event,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub struct Session {
    commands: mpsc::Sender<(u64, Request, egui::Context)>,
    cancel: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}
impl Session {
    pub fn new(events: mpsc::Sender<Event>) -> Self {
        let (commands, rx) = mpsc::channel::<(u64, Request, egui::Context)>();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let join = std::thread::spawn(move || {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let folder =
                std::env::temp_dir().join(format!("cam-gui-{}-{nonce}", std::process::id()));
            let mut child = None;
            let setup = (|| -> Result<(), String> {
                std::fs::create_dir(&folder).map_err(|e| e.to_string())?;
                let mut command =
                    std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
                command.arg("--worker").arg(&folder);
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    command.creation_flags(0x08000000);
                }
                child = Some(command.spawn().map_err(|e| e.to_string())?);
                Ok(())
            })();
            let mut last = None;
            while !flag.load(Ordering::SeqCst) {
                let (id, request, ctx) = match rx.recv_timeout(Duration::from_millis(10)) {
                    Ok(v) => v,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                };
                last = Some((id, ctx.clone()));
                let begin = Instant::now();
                let result = (|| -> Result<(SceneMeta, Vec<u8>), String> {
                    setup.clone()?;
                    let bytes = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
                    std::fs::write(folder.join("pending"), bytes).map_err(|e| e.to_string())?;
                    std::fs::rename(folder.join("pending"), folder.join("request"))
                        .map_err(|e| e.to_string())?;
                    loop {
                        if flag.load(Ordering::SeqCst) {
                            return Err("Cancelled; retained execution expired".into());
                        }
                        if child
                            .as_mut()
                            .unwrap()
                            .try_wait()
                            .map_err(|e| e.to_string())?
                            .is_some()
                        {
                            return Err(
                                "Compute process exited; regenerate to obtain a new execution"
                                    .into(),
                            );
                        }
                        let output = folder.join("response");
                        if output.exists() {
                            let size = std::fs::metadata(&output).map_err(|e| e.to_string())?.len();
                            if size > 128_000_000 {
                                return Err("GUI2 response exceeds 128 MB".into());
                            }
                            let bytes = std::fs::read(&output).map_err(|e| e.to_string())?;
                            std::fs::remove_file(output).map_err(|e| e.to_string())?;
                            let (meta, payload) = crate::compute::parse_worker_message(bytes)?;
                            return Ok((meta?, payload));
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                })();
                if !flag.load(Ordering::SeqCst) {
                    let _ = events.send(Event::Computed {
                        id,
                        elapsed_ms: begin.elapsed().as_secs_f64() * 1000.,
                        result,
                    });
                    ctx.request_repaint();
                }
                if setup.is_err()
                    || child
                        .as_mut()
                        .is_none_or(|c| c.try_wait().ok().flatten().is_some())
                {
                    break;
                }
            }
            let stopped = Instant::now();
            if let Some(mut child) = child {
                let _ = child.kill();
                let _ = child.wait();
            }
            if flag.load(Ordering::SeqCst)
                && let Some((id, ctx)) = last
            {
                let _ = events.send(Event::Cancelled {
                    id,
                    stop_ms: stopped.elapsed().as_secs_f64() * 1000.,
                });
                ctx.request_repaint();
            }
            for name in ["request", "pending", "response", "result"] {
                let _ = std::fs::remove_file(folder.join(name));
            }
            let _ = std::fs::remove_dir(folder);
        });
        Self {
            commands,
            cancel,
            join: Some(join),
        }
    }
    pub fn start(&self, id: u64, request: Request, ctx: egui::Context) {
        let _ = self.commands.send((id, request, ctx));
    }
    pub fn stop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
    pub fn finished(&self) -> bool {
        self.join.as_ref().is_none_or(|j| j.is_finished())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub fn serve(folder: PathBuf) -> Result<(), String> {
    loop {
        let input = folder.join("request");
        if !input.exists() {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        let bytes = std::fs::read(&input).map_err(|e| e.to_string())?;
        std::fs::remove_file(input).map_err(|e| e.to_string())?;
        let request = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let result = crate::compute::worker_message(request)?;
        std::fs::write(folder.join("result"), result).map_err(|e| e.to_string())?;
        std::fs::rename(folder.join("result"), folder.join("response"))
            .map_err(|e| e.to_string())?;
    }
}
