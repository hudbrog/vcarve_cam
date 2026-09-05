//! Bounded in-memory task ledger. HTTP disconnects do not cancel calculations.
use crate::{
    document::{API_VERSION, ENGINE_VERSION, JOB_BYTES, fingerprint},
    planning_worker::{self, Input, Output, Stage},
};
use cam_core::job::Job;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    io,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::{Semaphore, watch},
};

pub const MAX_PENDING: usize = 4;
pub const MAX_TASKS: usize = 128;
pub const RETAINED_RESULTS: usize = 4;
pub const TIMEOUT_SECONDS: u64 = 300;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Start {
    pub api_version: String,
    pub instance_id: String,
    pub request_id: String,
    pub revision: u64,
    pub document_fingerprint: String,
    pub stage: Stage,
    pub job: Value,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Queued,
    Running,
    Cancelling,
    Cancelled,
    Succeeded,
    Failed,
}
impl Status {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Cancelled | Self::Succeeded | Self::Failed)
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub api_version: &'static str,
    pub engine_version: &'static str,
    pub instance_id: String,
    pub task_id: String,
    pub revision: u64,
    pub document_fingerprint: String,
    pub stage: Stage,
    pub sequence: u64,
    pub state: Status,
    pub diagnostic: Option<Value>,
    pub summary: Option<Value>,
    pub result_available: bool,
}
struct Record {
    snapshot: Snapshot,
    request_hash: String,
    cancel: watch::Sender<bool>,
    result: Option<Arc<Output>>,
}
#[derive(Default)]
struct Ledger {
    records: HashMap<String, Record>,
    results: VecDeque<String>,
    closed: bool,
}
pub struct Planning {
    pub instance_id: String,
    ledger: Mutex<Ledger>,
    pending: Arc<Semaphore>,
    worker: Arc<Semaphore>,
}
#[derive(Debug)]
pub struct Failure(pub u16, pub &'static str, pub String);
impl Failure {
    fn new(status: u16, code: &'static str, message: &str) -> Self {
        Self(status, code, message.into())
    }
}
impl Planning {
    pub fn new() -> io::Result<Arc<Self>> {
        let mut id = [0u8; 16];
        getrandom::fill(&mut id).map_err(io::Error::other)?;
        Ok(Arc::new(Self {
            instance_id: id.iter().map(|b| format!("{b:02x}")).collect(),
            ledger: Mutex::new(Ledger::default()),
            pending: Arc::new(Semaphore::new(MAX_PENDING)),
            worker: Arc::new(Semaphore::new(1)),
        }))
    }
    pub fn start(self: &Arc<Self>, request: Start) -> Result<Snapshot, Failure> {
        if request.api_version != API_VERSION || request.instance_id != self.instance_id {
            return Err(Failure::new(
                409,
                "TASK_INSTANCE",
                "The service changed. Reconnect; previous tasks are not replayed.",
            ));
        }
        if request.request_id.is_empty()
            || request.request_id.len() > 128
            || !request
                .request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || request.revision > 9_007_199_254_740_991
        {
            return Err(Failure::new(
                400,
                "REQUEST_IDENTITY",
                "A short request ID and safe revision are required.",
            ));
        }
        let raw = request.job.to_string();
        if raw.len() > JOB_BYTES {
            return Err(Failure::new(413, "JOB_RESOURCE_LIMIT", "Job exceeds 8 MB."));
        }
        let job = Job::from_json(&raw)
            .map_err(|d| Failure(422, "PLAN_JOB", format!("{}: {}", d.code, d.message)))?;
        let document_fingerprint = fingerprint(&job);
        if document_fingerprint != request.document_fingerprint {
            return Err(Failure::new(
                409,
                "STALE_DOCUMENT",
                "The submitted job differs from its Rust validation receipt.",
            ));
        }
        let request_hash = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(request.revision, request.stage, &document_fingerprint))
                    .unwrap()
            )
        );
        let mut ledger = self.ledger.lock().unwrap();
        if ledger.closed {
            return Err(Failure::new(
                503,
                "SERVICE_STOPPING",
                "The service is shutting down and no longer accepts plans.",
            ));
        }
        if let Some(record) = ledger.records.get(&request.request_id) {
            return if record.request_hash == request_hash {
                Ok(record.snapshot.clone())
            } else {
                Err(Failure::new(
                    409,
                    "TASK_KEY_REUSED",
                    "This request ID already belongs to another immutable input.",
                ))
            };
        }
        if ledger.records.len() >= MAX_TASKS {
            return Err(Failure::new(
                503,
                "TASK_LEDGER_FULL",
                "This service has accepted 128 tasks. Save your jobs and restart it to clear task history.",
            ));
        }
        let slot = self.pending.clone().try_acquire_owned().map_err(|_| {
            Failure::new(
                503,
                "PLAN_QUEUE_FULL",
                "One plan is running and three are queued. Wait or cancel a task.",
            )
        })?;
        let snapshot = Snapshot {
            api_version: API_VERSION,
            engine_version: ENGINE_VERSION,
            instance_id: self.instance_id.clone(),
            task_id: request.request_id,
            revision: request.revision,
            document_fingerprint,
            stage: request.stage,
            sequence: 1,
            state: Status::Queued,
            diagnostic: None,
            summary: None,
            result_available: false,
        };
        let (cancel, mut cancelled) = watch::channel(false);
        ledger.records.insert(
            snapshot.task_id.clone(),
            Record {
                snapshot: snapshot.clone(),
                request_hash,
                cancel,
                result: None,
            },
        );
        drop(ledger);
        let service = self.clone();
        let id = snapshot.task_id.clone();
        tokio::spawn(async move {
            let _slot = slot;
            let permit = tokio::select! {
                biased;
                _ = cancelled.changed() => { service.finish(&id, None); return; },
                permit = service.worker.clone().acquire_owned() => permit,
            };
            let Ok(_permit) = permit else {
                return;
            };
            {
                let mut ledger = service.ledger.lock().unwrap();
                let record = ledger.records.get_mut(&id).unwrap();
                if *record.cancel.borrow() {
                    drop(ledger);
                    service.finish(&id, None);
                    return;
                }
                record.snapshot.state = Status::Running;
                record.snapshot.sequence += 1;
            }
            let result = run_worker(
                Input {
                    stage: request.stage,
                    job: raw,
                },
                &mut cancelled,
            )
            .await;
            service.finish(&id, result);
        });
        Ok(snapshot)
    }
    fn finish(&self, id: &str, reply: Option<Result<Output, Value>>) {
        let mut ledger = self.ledger.lock().unwrap();
        let record = ledger.records.get_mut(id).unwrap();
        record.snapshot.sequence += 1;
        if *record.cancel.borrow() || reply.is_none() {
            record.snapshot.state = Status::Cancelled;
            return;
        }
        match reply.unwrap() {
            Err(diagnostic) => {
                record.snapshot.state = Status::Failed;
                record.snapshot.diagnostic = Some(diagnostic);
            }
            Ok(result) => {
                record.snapshot.state = Status::Succeeded;
                record.snapshot.summary = Some(result.summary.clone());
                record.snapshot.result_available = true;
                record.result = Some(Arc::new(result));
                ledger.results.push_back(id.into());
                while ledger.results.len() > RETAINED_RESULTS {
                    let old = ledger.results.pop_front().unwrap();
                    let record = ledger.records.get_mut(&old).unwrap();
                    record.result = None;
                    record.snapshot.result_available = false;
                    record.snapshot.sequence += 1;
                }
            }
        }
    }
    pub fn snapshot(&self, id: &str) -> Result<Snapshot, Failure> {
        self.ledger
            .lock()
            .unwrap()
            .records
            .get(id)
            .map(|r| r.snapshot.clone())
            .ok_or_else(|| {
                Failure::new(
                    404,
                    "TASK_NOT_FOUND",
                    "Task was not found in this service instance. It has not been restarted.",
                )
            })
    }
    pub fn cancel(&self, id: &str) -> Result<Snapshot, Failure> {
        let mut ledger = self.ledger.lock().unwrap();
        let record = ledger.records.get_mut(id).ok_or_else(|| {
            Failure::new(
                404,
                "TASK_NOT_FOUND",
                "Task was not found in this service instance.",
            )
        })?;
        if !record.snapshot.state.terminal() && record.snapshot.state != Status::Cancelling {
            record.cancel.send_replace(true);
            record.snapshot.state = Status::Cancelling;
            record.snapshot.sequence += 1;
        }
        Ok(record.snapshot.clone())
    }
    pub fn result(&self, id: &str) -> Result<(Snapshot, Arc<Output>), Failure> {
        let ledger = self.ledger.lock().unwrap();
        let record = ledger
            .records
            .get(id)
            .ok_or_else(|| Failure::new(404, "TASK_NOT_FOUND", "Task was not found."))?;
        let result = record.result.clone().ok_or_else(|| Failure::new(410, "PLAN_RESULT_UNAVAILABLE", "Result is unfinished or expired. The service retains only the latest four results."))?;
        Ok((record.snapshot.clone(), result))
    }
    pub async fn shutdown(&self) {
        {
            let mut ledger = self.ledger.lock().unwrap();
            ledger.closed = true;
            for record in ledger.records.values() {
                if !record.snapshot.state.terminal() {
                    record.cancel.send_replace(true);
                }
            }
        }
        // All task futures release their slots only after the child exits.
        let _ = self.pending.acquire_many(MAX_PENDING as u32).await;
    }
}
fn diagnostic(code: &str, message: &str) -> Value {
    json!({ "code": code, "severity": "error", "stage": "planning", "message": message })
}
async fn run_worker(
    input: Input,
    cancel: &mut watch::Receiver<bool>,
) -> Option<Result<Output, Value>> {
    let operation = async {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg("--planning-worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        let mut child = command.spawn()?;
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let bytes = serde_json::to_vec(&input)?;
        let work = async {
            let write = async move {
                stdin.write_all(&bytes).await?;
                drop(stdin); // EOF is the worker's input framing, on Windows and Unix.
                Ok::<_, io::Error>(())
            };
            let read = async {
                let mut output = Vec::new();
                stdout
                    .take(planning_worker::WORKER_BYTES as u64 + 1)
                    .read_to_end(&mut output)
                    .await?;
                if output.len() > planning_worker::WORKER_BYTES {
                    return Err(io::Error::other("Worker response exceeds 32 MB"));
                }
                Ok::<_, io::Error>(output)
            };
            let (_, output, status) = tokio::try_join!(write, read, child.wait())?;
            if !status.success() {
                return Err(io::Error::other("Planning worker exited without a result"));
            }
            serde_json::from_slice::<Result<Output, Value>>(&output).map_err(io::Error::other)
        };
        let outcome = tokio::select! {
            biased;
            _ = cancel.changed() => None,
            result = tokio::time::timeout(Duration::from_secs(TIMEOUT_SECONDS), work) => Some(match result {
                Ok(result) => result,
                Err(_) => Ok(Err(diagnostic("PLAN_TIMEOUT", "Planning exceeded the five-minute service limit. Reduce the job or refine its resource limits."))),
            }),
        };
        // Also reap on read errors/timeouts. A terminal cancellation means actual work stopped.
        if child.try_wait()?.is_none() {
            child.kill().await?;
        }
        Ok::<_, io::Error>(outcome)
    };
    match operation.await {
        Ok(None) => None,
        Ok(Some(Ok(reply))) => Some(reply),
        Ok(Some(Err(error))) | Err(error) => {
            Some(Err(diagnostic("PLAN_WORKER_FAILURE", &error.to_string())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(service: &Planning, id: &str) -> Start {
        let job = Job::from_json(include_str!("../../../fixtures/m3/rectangle.json")).unwrap();
        Start {
            api_version: API_VERSION.into(),
            instance_id: service.instance_id.clone(),
            request_id: id.into(),
            revision: 7,
            document_fingerprint: fingerprint(&job),
            stage: Stage::Endmill,
            job: serde_json::to_value(job).unwrap(),
        }
    }
    #[tokio::test]
    async fn queue_is_bounded_idempotent_and_cancellable_before_a_worker_starts() {
        let service = Planning::new().unwrap();
        let _worker = service.worker.acquire().await.unwrap();
        for id in ["a", "b", "c", "d"] {
            assert!(service.start(request(&service, id)).is_ok());
        }
        assert!(matches!(
            service.start(request(&service, "e")),
            Err(Failure(503, "PLAN_QUEUE_FULL", _))
        ));
        assert!(service.start(request(&service, "a")).is_ok()); // Idempotent even when full.
        let mut changed = request(&service, "a");
        changed.revision += 1;
        assert!(matches!(
            service.start(changed),
            Err(Failure(409, "TASK_KEY_REUSED", _))
        ));
        let cancelling = service.cancel("a").unwrap();
        assert!(cancelling.state == Status::Cancelling);
        tokio::time::timeout(Duration::from_secs(2), async {
            while service.snapshot("a").unwrap().state != Status::Cancelled {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let cancelled = service.snapshot("a").unwrap();
        assert!(cancelled.sequence > cancelling.sequence);
        assert!(service.start(request(&service, "e")).is_ok());
        assert!(service.start(request(&service, "a")).unwrap().state == Status::Cancelled);
        tokio::time::timeout(Duration::from_secs(2), service.shutdown())
            .await
            .unwrap();
        assert!(matches!(
            service.start(request(&service, "after-shutdown")),
            Err(Failure(503, "SERVICE_STOPPING", _))
        ));
    }
    fn running(service: &Planning, id: &str) {
        let request = request(service, id);
        let (cancel, _) = watch::channel(false);
        service.ledger.lock().unwrap().records.insert(
            id.into(),
            Record {
                snapshot: Snapshot {
                    api_version: API_VERSION,
                    engine_version: ENGINE_VERSION,
                    instance_id: service.instance_id.clone(),
                    task_id: id.into(),
                    revision: request.revision,
                    document_fingerprint: request.document_fingerprint,
                    stage: request.stage,
                    sequence: 2,
                    state: Status::Running,
                    diagnostic: None,
                    summary: None,
                    result_available: false,
                },
                request_hash: String::new(),
                cancel,
                result: None,
            },
        );
    }
    fn result() -> Option<Result<Output, Value>> {
        Some(Ok(Output {
            summary: json!({ "status": "complete" }),
            motions: vec![],
            artifact: "{}".into(),
        }))
    }
    #[test]
    fn cancellation_and_completion_have_one_authoritative_winner() {
        let service = Planning::new().unwrap();
        running(&service, "cancel-first");
        service.cancel("cancel-first").unwrap();
        service.finish("cancel-first", result());
        let cancelled = service.snapshot("cancel-first").unwrap();
        assert!(cancelled.state == Status::Cancelled);
        assert!(cancelled.summary.is_none());
        assert!(service.result("cancel-first").is_err());
        running(&service, "finish-first");
        service.finish("finish-first", result());
        assert!(service.cancel("finish-first").unwrap().state == Status::Succeeded);
        assert!(service.result("finish-first").is_ok());
    }
    #[test]
    fn result_eviction_keeps_summary_identity_and_updates_sequence() {
        let service = Planning::new().unwrap();
        for i in 0..=RETAINED_RESULTS {
            let id = i.to_string();
            running(&service, &id);
            service.finish(&id, result());
        }
        let expired = service.snapshot("0").unwrap();
        assert!(expired.state == Status::Succeeded);
        assert!(!expired.result_available);
        assert_eq!(expired.sequence, 4);
        assert!(expired.summary.is_some());
        assert!(matches!(
            service.result("0"),
            Err(Failure(410, "PLAN_RESULT_UNAVAILABLE", _))
        ));
        assert!(service.result("1").is_ok());
    }
}
