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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification: Option<crate::verification::Identity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export: Option<crate::exporting::Identity>,
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
    pub(crate) fn new(status: u16, code: &'static str, message: &str) -> Self {
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
        self.validate_identity(
            &request.api_version,
            &request.instance_id,
            &request.request_id,
            request.revision,
        )?;
        let raw = request.job.to_string();
        if raw.len() > JOB_BYTES {
            return Err(Failure::new(
                413,
                "JOB_RESOURCE_LIMIT",
                "Job exceeds 64 MB.",
            ));
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
                serde_json::to_vec(&(
                    "plan",
                    request.revision,
                    request.stage,
                    &document_fingerprint
                ))
                .unwrap()
            )
        );
        self.enqueue(
            Snapshot {
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
                verification: None,
                export: None,
            },
            request_hash,
            Input {
                stage: request.stage,
                job: raw,
                verification: None,
                export: None,
                output_path: None,
                source_artifact: None,
            },
        )
    }
    pub(crate) fn validate_identity(
        &self,
        api_version: &str,
        instance_id: &str,
        request_id: &str,
        revision: u64,
    ) -> Result<(), Failure> {
        if api_version != API_VERSION || instance_id != self.instance_id {
            return Err(Failure::new(
                409,
                "TASK_INSTANCE",
                "The service changed. Reconnect; previous tasks are not replayed.",
            ));
        }
        if request_id.is_empty()
            || request_id.len() > 128
            || !request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || revision > 9_007_199_254_740_991
        {
            return Err(Failure::new(
                400,
                "REQUEST_IDENTITY",
                "A short request ID and safe revision are required.",
            ));
        }
        Ok(())
    }
    pub(crate) fn replay(&self, id: &str, hash: &str) -> Result<Option<Snapshot>, Failure> {
        let ledger = self.ledger.lock().unwrap();
        if ledger.closed {
            return Err(Failure::new(
                503,
                "SERVICE_STOPPING",
                "The service is shutting down.",
            ));
        }
        match ledger.records.get(id) {
            Some(record) if record.request_hash == hash => Ok(Some(record.snapshot.clone())),
            Some(_) => Err(Failure::new(
                409,
                "TASK_KEY_REUSED",
                "This request ID already belongs to another immutable input.",
            )),
            None => Ok(None),
        }
    }
    pub(crate) fn enqueue(
        self: &Arc<Self>,
        snapshot: Snapshot,
        request_hash: String,
        input: Input,
    ) -> Result<Snapshot, Failure> {
        let mut ledger = self.ledger.lock().unwrap();
        if ledger.closed {
            return Err(Failure::new(
                503,
                "SERVICE_STOPPING",
                "The service is shutting down and no longer accepts computations.",
            ));
        }
        if let Some(record) = ledger.records.get(&snapshot.task_id) {
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
                "One calculation is running and three are queued. Wait or cancel a task.",
            )
        })?;
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
            let result = run_worker(input, &mut cancelled).await;
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
        let mut ledger = self.ledger.lock().unwrap();
        for record in ledger.records.values_mut() {
            record.result = None;
            if record.snapshot.result_available {
                record.snapshot.result_available = false;
                record.snapshot.sequence += 1;
            }
        }
        ledger.results.clear();
    }
}
fn diagnostic(code: &str, message: &str) -> Value {
    json!({ "code": code, "severity": "error", "stage": if code.starts_with("EXPORT_") {"export"} else if code.starts_with("VERIFICATION_") {"verification"} else {"planning"}, "message": message })
}
async fn run_worker(
    mut input: Input,
    cancel: &mut watch::Receiver<bool>,
) -> Option<Result<Output, Value>> {
    let verifying = input.verification.is_some();
    let exporting = input.export.is_some();
    // Keep the source lease until the child is reaped, including read errors,
    // timeout and cancellation. Serde deliberately omits this parent-only field.
    let _source_artifact = input.source_artifact.take();
    let operation = async {
        let artifact = if !verifying && !exporting {
            let file = crate::artifact::PlanFile::create()?;
            input.output_path = Some(file.path().to_owned());
            Some(file)
        } else {
            None
        };
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
                    return Err(io::Error::other(format!(
                        "Worker response exceeds the {} byte limit",
                        planning_worker::WORKER_BYTES
                    )));
                }
                Ok::<_, io::Error>(output)
            };
            let (_, output, status) = tokio::try_join!(write, read, child.wait())?;
            if !status.success() {
                return Err(io::Error::other("Planning worker exited without a result"));
            }
            let mut reply = serde_json::from_slice::<Result<Output, Value>>(&output)
                .map_err(io::Error::other)?;
            if let (Ok(result), Some(artifact)) = (&mut reply, &artifact) {
                if artifact.byte_len()? == 0 || !result.artifact.is_empty() {
                    return Err(io::Error::other("Worker returned no complete plan file"));
                }
                result.plan_artifact = Some(artifact.clone());
            }
            Ok(reply)
        };
        let outcome = tokio::select! {
            biased;
            _ = cancel.changed() => None,
            result = tokio::time::timeout(Duration::from_secs(TIMEOUT_SECONDS), work) => Some(match result {
                Ok(result) => result,
                Err(_) => Ok(Err(diagnostic(if exporting {"EXPORT_TIMEOUT"} else if verifying {"VERIFICATION_TIMEOUT"} else {"PLAN_TIMEOUT"}, "Calculation exceeded the five-minute service limit. Reduce the job or refine its resource limits."))),
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
        Ok(Some(Err(error))) | Err(error) => Some(Err(diagnostic(
            if exporting {
                "EXPORT_WORKER_FAILURE"
            } else if verifying {
                "VERIFICATION_WORKER_FAILURE"
            } else {
                "PLAN_WORKER_FAILURE"
            },
            &error.to_string(),
        ))),
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
                    verification: None,
                    export: None,
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
            programs: vec![],
            artifact: "{}".into(),
            plan_artifact: Some(crate::artifact::PlanFile::create().unwrap()),
            inspection: crate::inspection::Inspection::default(),
        }))
    }
    #[tokio::test]
    async fn verification_binds_a_combined_source_and_retries_after_source_expiry() {
        use crate::verification::{self, Identity};
        use cam_core::verification::VerificationOptions;
        let service = Planning::new().unwrap();
        let _worker = service.worker.acquire().await.unwrap();
        running(&service, "source");
        let mut output = result().unwrap().unwrap();
        let artifact_path = output.plan_artifact.as_ref().unwrap().path().to_owned();
        output.summary =
            json!({"inputFingerprint":"a".repeat(64),"motionFingerprint":"b".repeat(64)});
        service.finish("source", Some(Ok(output)));
        let source = service.snapshot("source").unwrap();
        let make = || verification::Start {
            api_version: API_VERSION.into(),
            instance_id: service.instance_id.clone(),
            request_id: "verify".into(),
            revision: source.revision,
            document_fingerprint: source.document_fingerprint.clone(),
            verification: Identity {
                plan_task_id: "source".into(),
                input_fingerprint: "a".repeat(64),
                motion_fingerprint: "b".repeat(64),
                options: VerificationOptions::default(),
            },
        };
        assert!(matches!(
            verification::start(&service, make()),
            Err(Failure(422, "VERIFICATION_STAGE", _))
        ));
        service
            .ledger
            .lock()
            .unwrap()
            .records
            .get_mut("source")
            .unwrap()
            .snapshot
            .stage = Stage::Combined;
        let mut wrong = make();
        wrong.revision += 1;
        assert!(matches!(
            verification::start(&service, wrong),
            Err(Failure(409, "VERIFICATION_PLAN_IDENTITY", _))
        ));
        let mut wrong = make();
        wrong.verification.motion_fingerprint = "c".repeat(64);
        assert!(matches!(
            verification::start(&service, wrong),
            Err(Failure(409, "VERIFICATION_PLAN_IDENTITY", _))
        ));
        let mut wrong = make();
        wrong.verification.options.max_cells = 0;
        assert!(matches!(
            verification::start(&service, wrong),
            Err(Failure(422, "VERIFICATION_OPTIONS", _))
        ));
        assert!(verification::start(&service, make()).unwrap().state == Status::Queued);
        assert_eq!(service.pending.available_permits(), MAX_PENDING - 1);
        let mut reused = make();
        reused.verification.options.max_cells = 1;
        assert!(matches!(
            verification::start(&service, reused),
            Err(Failure(409, "TASK_KEY_REUSED", _))
        ));
        for n in 0..RETAINED_RESULTS {
            let id = format!("other-{n}");
            running(&service, &id);
            service.finish(&id, result());
        }
        assert!(service.result("source").is_err());
        assert!(artifact_path.exists()); // The queued verifier still owns a lease.
        assert!(verification::start(&service, make()).unwrap().state == Status::Queued);
        service.cancel("verify").unwrap();
        tokio::time::timeout(Duration::from_secs(2), service.shutdown())
            .await
            .unwrap();
        assert!(service.snapshot("verify").unwrap().state == Status::Cancelled);
        assert!(service.result("verify").is_err());
        assert!(!artifact_path.exists());
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
    #[tokio::test]
    async fn export_admission_binds_profile_and_plan_and_retries_after_source_expiry() {
        use crate::exporting::{self, Identity};
        use cam_core::{
            post::{LinuxCncProfile, ProgramLayout},
            verification::VerificationOptions,
        };
        let service = Planning::new().unwrap();
        let _worker = service.worker.acquire().await.unwrap();
        running(&service, "source");
        let mut output = result().unwrap().unwrap();
        let artifact_path = output.plan_artifact.as_ref().unwrap().path().to_owned();
        output.summary =
            json!({"inputFingerprint":"a".repeat(64),"motionFingerprint":"b".repeat(64)});
        service.finish("source", Some(Ok(output)));
        let source = service.snapshot("source").unwrap();
        let make = || exporting::Start {
            api_version: API_VERSION.into(),
            instance_id: service.instance_id.clone(),
            request_id: "export".into(),
            revision: source.revision,
            document_fingerprint: source.document_fingerprint.clone(),
            export: Identity {
                plan_task_id: "source".into(),
                input_fingerprint: "a".repeat(64),
                motion_fingerprint: "b".repeat(64),
                profile: LinuxCncProfile::from_json(include_str!(
                    "../../../fixtures/m6/macro-stock-bottom.json"
                ))
                .unwrap(),
                layout: ProgramLayout::Combined,
                options: VerificationOptions::default(),
            },
        };
        assert!(matches!(
            exporting::start(&service, make()),
            Err(Failure(422, "EXPORT_STAGE", _))
        ));
        service
            .ledger
            .lock()
            .unwrap()
            .records
            .get_mut("source")
            .unwrap()
            .snapshot
            .stage = Stage::Combined;
        let mut wrong = make();
        wrong.export.motion_fingerprint = "c".repeat(64);
        assert!(matches!(
            exporting::start(&service, wrong),
            Err(Failure(409, "EXPORT_PLAN_IDENTITY", _))
        ));
        let mut oversized = make();
        oversized.export.profile.m6.reference = "x".repeat(64_000);
        assert!(matches!(
            exporting::start(&service, oversized),
            Err(Failure(413, "EXPORT_PROFILE_LIMIT", _))
        ));
        let mut wrong = make();
        wrong.export.options.max_cells = 0;
        assert!(matches!(
            exporting::start(&service, wrong),
            Err(Failure(422, "EXPORT_OPTIONS", _))
        ));
        assert!(exporting::start(&service, make()).unwrap().state == Status::Queued);
        assert_eq!(service.pending.available_permits(), MAX_PENDING - 1);
        let mut changed = make();
        changed.export.profile.decimal_places = 0;
        assert!(matches!(
            exporting::start(&service, changed),
            Err(Failure(409, "TASK_KEY_REUSED", _))
        ));
        for n in 0..RETAINED_RESULTS {
            let id = format!("evict-{n}");
            running(&service, &id);
            service.finish(&id, result());
        }
        assert!(service.result("source").is_err());
        assert!(artifact_path.exists()); // The queued exporter still owns a lease.
        assert!(exporting::start(&service, make()).unwrap().state == Status::Queued);
        service.cancel("export").unwrap();
        tokio::time::timeout(Duration::from_secs(2), service.shutdown())
            .await
            .unwrap();
        assert!(service.snapshot("export").unwrap().state == Status::Cancelled);
        assert!(service.result("export").is_err());
        assert!(!artifact_path.exists());
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
