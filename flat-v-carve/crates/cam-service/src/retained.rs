//! Retained execution and ordered output (H5, plan section 22.8): the
//! service-owned runtime behind the ui-9 retained commands.
//!
//! `Generate`/`PrepareOutput` register cancellable tasks; plan and bundle
//! handles are opaque service-owned identities with bounded retention, not
//! receipts a client may upload. Paging and inspection read the retained
//! generated execution and never replan; preparation re-checks the current
//! document's machining identity for the plan's scope, and the emitted
//! bundle keeps its exact checked bytes so a save can be retried with
//! identical bytes. A completion arriving after cancellation is discarded:
//! late replies cannot issue handles, and evicted handles cannot authorize
//! output.
//!
//! The runtime is deliberately free of threads and async: an adapter claims
//! a task, computes outside any lock and delivers the outcome — so the
//! native service can plan on a blocking worker while `CancelTask`/
//! `TaskStatus` stay responsive, and a single-threaded adapter (the WASM
//! worker, the CLI) drives tasks synchronously with
//! [`Retained::run_task`]/[`Retained::execute_driven`].
use crate::collection::{
    CollectionCommand, CollectionScope, error, fingerprint, inspect_plan_value, motion_page,
    parse_job, plan_summary,
};
use cam_core::{
    checks::{BasicCheckReport, check_plan_v5},
    geometry::{Diagnostic, Result},
    post::sequence::{OutputLayout, PreparedExecution, SequenceProgram},
    project::v5::{self, machine, references::ReadinessScope},
    sequence::{OperationPlanV5, PlanLimits, TrustedPlanV5},
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Retained plans kept live at once; older handles expire (oldest first,
/// numbers never reused) when newer plans arrive.
pub const RETAINED_PLANS: usize = 8;
/// Retained prepared bundles kept live at once.
pub const RETAINED_BUNDLES: usize = 8;
/// Task records kept at once (terminal records are pruned first).
pub const RETAINED_TASKS: usize = 64;

fn retained_error(code: &str, message: impl Into<String>) -> Diagnostic {
    error(code, message)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskState {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl TaskState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
    fn terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

enum TaskKind {
    Generate {
        job: v5::CamJobV5,
        scope: ReadinessScope,
        scope_wire: CollectionScope,
    },
    PrepareOutput {
        plan_handle: String,
        job: v5::CamJobV5,
        scope: ReadinessScope,
        scope_wire: CollectionScope,
        layout: OutputLayout,
    },
}

impl TaskKind {
    fn name(&self) -> &'static str {
        match self {
            Self::Generate { .. } => "generate",
            Self::PrepareOutput { .. } => "prepare_output",
        }
    }
}

struct TaskRecord {
    task_id: String,
    kind: TaskKind,
    state: TaskState,
    diagnostic: Option<Diagnostic>,
    /// Plan handle issued by this task's completion, when it is a succeeded
    /// generation whose plan is still retained.
    plan: Option<String>,
    /// Bundle handle issued by this task's completion, when it is a
    /// succeeded preparation whose bundle is still retained.
    bundle: Option<String>,
}

/// A retained generated plan: the trusted in-process execution plus the
/// identities paging, inspection and preparation check against.
pub struct RetainedPlan {
    pub handle: String,
    pub task_id: String,
    pub scope: CollectionScope,
    pub document_fingerprint: String,
    pub machining_identity: String,
    pub trusted: TrustedPlanV5,
    pub checks: BasicCheckReport,
}

/// A retained prepared bundle: the exact checked bytes of every ordered
/// file with its manifest and aggregate report. The bytes are immutable —
/// every read, including retries after a failed save, returns them
/// unchanged.
pub struct RetainedBundle {
    pub handle: String,
    pub task_id: String,
    pub plan_handle: String,
    pub layout: OutputLayout,
    pub files: Vec<SequenceProgram>,
    pub manifest: Value,
    pub report: Value,
    pub machining_identity: String,
}

/// The outcome of one computed task, delivered back under the runtime lock.
pub enum TaskOutcome {
    Plan(Arc<RetainedPlan>),
    Bundle(Arc<RetainedBundle>),
}

/// A task an executor claimed: compute this outside any runtime lock, then
/// deliver it with [`Retained::complete`]. The handle the completion will
/// issue is pre-allocated at claim time, so it is unique even when a
/// cancelled claim never delivers. `plan: None` marks a preparation whose
/// plan handle expired between registration and execution.
pub enum ClaimedTask {
    Generate {
        task_id: String,
        plan_handle: String,
        job: v5::CamJobV5,
        scope: ReadinessScope,
        scope_wire: CollectionScope,
    },
    PrepareOutput {
        task_id: String,
        bundle_handle: String,
        plan: Option<Arc<RetainedPlan>>,
        job: v5::CamJobV5,
        scope: ReadinessScope,
        scope_wire: CollectionScope,
        layout: OutputLayout,
    },
}

/// The retained runtime state. Wrap it in the adapter's synchronization
/// (native: one mutex around the whole struct; WASM worker: the thread's
/// single instance) — every method takes `&mut self` and none of them
/// computes.
#[derive(Default)]
pub struct Retained {
    tasks: Vec<TaskRecord>,
    plans: Vec<(String, Arc<RetainedPlan>)>,
    bundles: Vec<(String, Arc<RetainedBundle>)>,
    next_task: u64,
    next_plan: u64,
    next_bundle: u64,
    plans_run: u64,
    prepares_run: u64,
}

impl Retained {
    /// Read the generated execution for an in-process display adapter. The
    /// returned immutable plan has the same lifetime/admission checks as paging.
    /// This is not a deserialization or trust-registration entry point.
    pub fn generated_plan(&self, handle: &str) -> Result<Arc<RetainedPlan>> {
        self.live_plan(handle)
    }
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a command registers a task the adapter should drive to
    /// completion after replying (or inline, for synchronous adapters).
    pub fn starts_task(command: &CollectionCommand) -> bool {
        matches!(
            command,
            CollectionCommand::Generate { .. } | CollectionCommand::PrepareOutput { .. }
        )
    }

    /// Whether a command owns service-side state and needs this runtime
    /// (the stateless entry refuses it with `RETAINED_STATE_REQUIRED`).
    pub fn is_retained(command: &CollectionCommand) -> bool {
        matches!(
            command,
            CollectionCommand::Generate { .. }
                | CollectionCommand::TaskStatus { .. }
                | CollectionCommand::CancelTask { .. }
                | CollectionCommand::ReadMotions { .. }
                | CollectionCommand::ReadInspection { .. }
                | CollectionCommand::PrepareOutput { .. }
                | CollectionCommand::PreparedOutput { .. }
                | CollectionCommand::ReadPreparedBytes { .. }
        )
    }

    /// Execute one ui-9 command against the runtime: the retained commands
    /// here, every stateless command unchanged through
    /// [`crate::collection::execute`]. Task-starting commands only register
    /// and answer with the pending snapshot.
    pub fn execute(&mut self, command: CollectionCommand) -> Result<Value> {
        match command {
            CollectionCommand::Generate { job, scope } => self.generate(job, scope),
            CollectionCommand::TaskStatus { task_id } => self.task_status(&task_id),
            CollectionCommand::CancelTask { task_id } => self.cancel_task(&task_id),
            CollectionCommand::ReadMotions {
                plan_handle,
                offset,
            } => self.read_motions(&plan_handle, offset),
            CollectionCommand::ReadInspection { plan_handle } => self.read_inspection(&plan_handle),
            CollectionCommand::PrepareOutput {
                plan_handle,
                job,
                layout,
            } => self.prepare_output(plan_handle, job, layout),
            CollectionCommand::PreparedOutput { task_id } => self.prepared_output(&task_id),
            CollectionCommand::ReadPreparedBytes {
                bundle_handle,
                filename,
            } => self.read_prepared_bytes(&bundle_handle, &filename),
            stateless => crate::collection::execute(stateless),
        }
    }

    /// Execute one ui-9 command, driving any task it registers to
    /// completion inline and answering with its terminal snapshot — the
    /// synchronous-adapter entry (WASM worker, CLI, tests). The cancel
    /// window of the asynchronous contract is exercised between
    /// [`Retained::execute`] and [`Retained::run_task`] by adapters and
    /// tests that interleave them.
    pub fn execute_driven(&mut self, command: CollectionCommand) -> Result<Value> {
        let reply = self.execute(command)?;
        if let Some(task_id) = reply
            .get("task")
            .and_then(|task| task["taskId"].as_str())
            .map(str::to_string)
        {
            self.run_task(&task_id);
            return self.task_status(&task_id);
        }
        Ok(reply)
    }

    fn generate(&mut self, job_value: Value, scope: CollectionScope) -> Result<Value> {
        let job = parse_job(&job_value)?;
        let readiness = scope.readiness();
        if let ReadinessScope::ThroughOperation { operation_id } = &readiness
            && !job.operations.iter().any(|op| op.id == *operation_id)
        {
            return Err(retained_error(
                "READINESS_SCOPE",
                format!("prefix scope references unknown operation '{operation_id}'"),
            ));
        }
        self.prune_tasks();
        if self.tasks.len() >= RETAINED_TASKS {
            return Err(retained_error(
                "RETAINED_LEDGER_FULL",
                format!(
                    "this instance already tracks {RETAINED_TASKS} live tasks; \
                     its plans and bundles stay readable"
                ),
            ));
        }
        self.next_task += 1;
        let task_id = format!("task-{}", self.next_task);
        self.tasks.push(TaskRecord {
            task_id: task_id.clone(),
            kind: TaskKind::Generate {
                job,
                scope: readiness,
                scope_wire: scope,
            },
            state: TaskState::Pending,
            diagnostic: None,
            plan: None,
            bundle: None,
        });
        self.task_status(&task_id)
    }

    fn prepare_output(
        &mut self,
        plan_handle: String,
        job_value: Value,
        layout: OutputLayout,
    ) -> Result<Value> {
        let plan = self.live_plan(&plan_handle)?;
        let job = parse_job(&job_value)?;
        self.prune_tasks();
        if self.tasks.len() >= RETAINED_TASKS {
            return Err(retained_error(
                "RETAINED_LEDGER_FULL",
                format!(
                    "this instance already tracks {RETAINED_TASKS} live tasks; \
                     its plans and bundles stay readable"
                ),
            ));
        }
        self.next_task += 1;
        let task_id = format!("task-{}", self.next_task);
        self.tasks.push(TaskRecord {
            task_id: task_id.clone(),
            kind: TaskKind::PrepareOutput {
                plan_handle,
                job,
                scope: plan.scope.readiness(),
                scope_wire: plan.scope.clone(),
                layout,
            },
            state: TaskState::Pending,
            diagnostic: None,
            plan: None,
            bundle: None,
        });
        self.task_status(&task_id)
    }

    fn task_status(&self, task_id: &str) -> Result<Value> {
        let record = self.task_record(task_id)?;
        let task = self.task_value(record);
        Ok(json!({ "task": task, "retained": self.stats_value() }))
    }

    fn cancel_task(&mut self, task_id: &str) -> Result<Value> {
        let Some(record) = self
            .tasks
            .iter_mut()
            .find(|record| record.task_id == task_id)
        else {
            return Err(self.task_not_found(task_id));
        };
        // Cancelling a running task wins over any completion still in
        // flight: `complete` only stores outcomes while the task is running.
        if !record.state.terminal() {
            record.state = TaskState::Cancelled;
            record.diagnostic = None;
        }
        let cancelled = (
            record.task_id.clone(),
            record.kind.name(),
            record.state,
            record.diagnostic.clone(),
            record.plan.clone(),
            record.bundle.clone(),
        );
        Ok(json!({
            "task": task_value_of(&cancelled),
            "retained": self.stats_value(),
        }))
    }

    fn read_motions(&self, plan_handle: &str, offset: usize) -> Result<Value> {
        let plan = self.live_plan(plan_handle)?;
        let page = motion_page(plan.trusted.plan(), offset)?;
        let stages: Vec<Value> = plan
            .trusted
            .plan()
            .stages
            .iter()
            .map(|stage| {
                json!({
                    "stageId": stage.stage_id,
                    "operationId": stage.operation_id,
                    "toolId": stage.tool_id,
                    "role": stage.role,
                    "motionRange": [stage.motion_range.0, stage.motion_range.1],
                })
            })
            .collect();
        Ok(json!({
            "motions": page,
            "stages": stages,
            "scope": plan.scope,
            "documentFingerprint": plan.document_fingerprint,
            "machiningIdentity": plan.machining_identity,
            "executionFingerprint": plan.trusted.plan().execution_fingerprint,
            "retained": self.stats_value(),
        }))
    }

    fn read_inspection(&self, plan_handle: &str) -> Result<Value> {
        let plan = self.live_plan(plan_handle)?;
        Ok(json!({
            "inspection": inspect_plan_value(plan.trusted.plan())?,
            "summary": plan_summary(plan.trusted.plan(), &plan.checks),
            "scope": plan.scope,
            "documentFingerprint": plan.document_fingerprint,
            "machiningIdentity": plan.machining_identity,
            "retained": self.stats_value(),
        }))
    }

    fn prepared_output(&self, task_id: &str) -> Result<Value> {
        let record = self.task_record(task_id)?;
        if !matches!(record.kind, TaskKind::PrepareOutput { .. }) {
            return Err(retained_error(
                "RETAINED_TASK_KIND",
                format!("task '{task_id}' is not an output preparation"),
            ));
        }
        match record.state {
            TaskState::Succeeded => {
                let Some(handle) = record.bundle.clone() else {
                    return Err(retained_error(
                        "RETAINED_HANDLE_EXPIRED",
                        format!("the bundle of task '{task_id}' was evicted from the retained set"),
                    ));
                };
                let bundle = self.live_bundle(&handle)?;
                Ok(json!({
                    "bundle": self.bundle_value(&bundle),
                    "retained": self.stats_value(),
                }))
            }
            TaskState::Failed => Err(record
                .diagnostic
                .clone()
                .unwrap_or_else(|| retained_error("RETAINED_TASK_FAILED", "preparation failed"))),
            TaskState::Cancelled => Err(retained_error(
                "RETAINED_TASK_CANCELLED",
                "the preparation was cancelled; register it again",
            )),
            TaskState::Pending | TaskState::Running => Err(retained_error(
                "RETAINED_TASK_NOT_FINISHED",
                "the preparation has not finished yet",
            )),
        }
    }

    fn read_prepared_bytes(&self, bundle_handle: &str, filename: &str) -> Result<Value> {
        let bundle = self.live_bundle(bundle_handle)?;
        let file = bundle
            .files
            .iter()
            .find(|file| file.filename == filename)
            .ok_or_else(|| {
                retained_error(
                    "RETAINED_FILE_NOT_FOUND",
                    format!("bundle '{bundle_handle}' has no file named '{filename}'"),
                )
            })?;
        let sha256 = bundle.manifest["files"]
            .as_array()
            .and_then(|files| {
                files
                    .iter()
                    .find(|entry| entry["filename"] == json!(file.filename))
            })
            .map(|entry| entry["sha256"].clone())
            .unwrap_or(Value::Null);
        Ok(json!({
            "file": {
                "filename": file.filename,
                "gcode": file.gcode,
                "sha256": sha256,
                "byteLength": file.gcode.len(),
            },
            "bundleHandle": bundle_handle,
            "retained": self.stats_value(),
        }))
    }

    /// Claim a pending task for execution. `Ok(None)` means the task is
    /// already running elsewhere or terminal. A claimed task computes
    /// outside the runtime lock and delivers its outcome with
    /// [`Retained::complete`], where a cancellation that arrived meanwhile
    /// is the single authoritative winner.
    pub fn claim(&mut self, task_id: &str) -> Result<Option<ClaimedTask>> {
        let Some(record) = self
            .tasks
            .iter_mut()
            .find(|record| record.task_id == task_id)
        else {
            return Err(self.task_not_found(task_id));
        };
        if record.state != TaskState::Pending {
            return Ok(None);
        }
        record.state = TaskState::Running;
        match &mut record.kind {
            TaskKind::Generate {
                job,
                scope,
                scope_wire,
                ..
            } => {
                self.plans_run += 1;
                self.next_plan += 1;
                Ok(Some(ClaimedTask::Generate {
                    task_id: task_id.to_string(),
                    plan_handle: format!("plan-{}", self.next_plan),
                    job: job.clone(),
                    scope: scope.clone(),
                    scope_wire: scope_wire.clone(),
                }))
            }
            TaskKind::PrepareOutput {
                plan_handle,
                job,
                scope,
                scope_wire,
                layout,
            } => {
                self.prepares_run += 1;
                self.next_bundle += 1;
                let plan = self
                    .plans
                    .iter()
                    .find(|(live, _)| live == plan_handle)
                    .map(|(_, plan)| plan.clone());
                Ok(Some(ClaimedTask::PrepareOutput {
                    task_id: task_id.to_string(),
                    bundle_handle: format!("bundle-{}", self.next_bundle),
                    plan,
                    job: job.clone(),
                    scope: scope.clone(),
                    scope_wire: scope_wire.clone(),
                    layout: *layout,
                }))
            }
        }
    }

    /// Deliver a computed outcome. A cancellation that arrived between
    /// [`Retained::claim`] and this call is the authoritative winner: the
    /// outcome is discarded and no handle is issued. Duplicate deliveries
    /// and deliveries to dead tasks change nothing.
    pub fn complete(&mut self, task_id: &str, outcome: Result<TaskOutcome>) {
        let Some(record) = self
            .tasks
            .iter_mut()
            .find(|record| record.task_id == task_id)
        else {
            return;
        };
        if record.state != TaskState::Running {
            return;
        }
        match outcome {
            Err(diagnostic) => {
                record.state = TaskState::Failed;
                record.diagnostic = Some(diagnostic);
            }
            Ok(TaskOutcome::Plan(plan)) => {
                record.state = TaskState::Succeeded;
                record.plan = Some(plan.handle.clone());
                self.plans.push((plan.handle.clone(), plan));
                while self.plans.len() > RETAINED_PLANS {
                    let (evicted, _) = self.plans.remove(0);
                    for record in &mut self.tasks {
                        if record.plan.as_deref() == Some(evicted.as_str()) {
                            record.plan = None;
                        }
                    }
                }
            }
            Ok(TaskOutcome::Bundle(bundle)) => {
                record.state = TaskState::Succeeded;
                record.bundle = Some(bundle.handle.clone());
                self.bundles.push((bundle.handle.clone(), bundle));
                while self.bundles.len() > RETAINED_BUNDLES {
                    let (evicted, _) = self.bundles.remove(0);
                    for record in &mut self.tasks {
                        if record.bundle.as_deref() == Some(evicted.as_str()) {
                            record.bundle = None;
                        }
                    }
                }
            }
        }
    }

    /// Claim, compute and complete one task synchronously.
    pub fn run_task(&mut self, task_id: &str) {
        let claimed = match self.claim(task_id) {
            Ok(Some(claimed)) => claimed,
            _ => return,
        };
        // Compute outside the record borrow; `claimed` owns its inputs.
        let outcome = compute(claimed);
        self.complete(task_id, outcome);
    }

    /// Run every pending task (registration order), one at a time.
    pub fn run_pending(&mut self) {
        let ids: Vec<String> = self
            .tasks
            .iter()
            .filter(|record| record.state == TaskState::Pending)
            .map(|record| record.task_id.clone())
            .collect();
        for id in ids {
            self.run_task(&id);
        }
    }

    fn live_plan(&self, handle: &str) -> Result<Arc<RetainedPlan>> {
        self.plans
            .iter()
            .find(|(live, _)| live == handle)
            .map(|(_, plan)| plan.clone())
            .ok_or_else(|| self.handle_error("plan", handle))
    }

    fn live_bundle(&self, handle: &str) -> Result<Arc<RetainedBundle>> {
        self.bundles
            .iter()
            .find(|(live, _)| live == handle)
            .map(|(_, bundle)| bundle.clone())
            .ok_or_else(|| self.handle_error("bundle", handle))
    }

    /// Unknown and expired are distinguished by the generation counters:
    /// handle numbers are never reused, so a well-formed number above the
    /// highest issued one was never issued by this instance.
    fn handle_error(&self, kind: &str, handle: &str) -> Diagnostic {
        let issued = match kind {
            "plan" => self.next_plan,
            _ => self.next_bundle,
        };
        let well_formed = handle
            .rsplit('-')
            .next()
            .and_then(|n| n.parse::<u64>().ok())
            .is_some_and(|n| n >= 1);
        if well_formed && handle.starts_with(kind) {
            let number = handle.rsplit('-').next().unwrap().parse::<u64>().unwrap();
            if number <= issued {
                retained_error(
                    "RETAINED_HANDLE_EXPIRED",
                    format!(
                        "{kind} handle '{handle}' is no longer in the retained set \
                         (evicted, or its task was cancelled before issuing)"
                    ),
                )
            } else {
                retained_error(
                    "RETAINED_HANDLE_UNKNOWN",
                    format!("{kind} handle '{handle}' was never issued by this instance"),
                )
            }
        } else {
            retained_error(
                "RETAINED_HANDLE_UNKNOWN",
                format!("{kind} handle '{handle}' was never issued by this instance"),
            )
        }
    }

    fn task_record(&self, task_id: &str) -> Result<&TaskRecord> {
        self.tasks
            .iter()
            .find(|record| record.task_id == task_id)
            .ok_or_else(|| self.task_not_found(task_id))
    }

    fn task_not_found(&self, task_id: &str) -> Diagnostic {
        retained_error(
            "RETAINED_TASK_NOT_FOUND",
            format!("task '{task_id}' was not found in this instance"),
        )
    }

    fn task_value(&self, record: &TaskRecord) -> Value {
        task_value_of(&(
            record.task_id.clone(),
            record.kind.name(),
            record.state,
            record.diagnostic.clone(),
            record.plan.clone(),
            record.bundle.clone(),
        ))
    }

    fn bundle_value(&self, bundle: &RetainedBundle) -> Value {
        json!({
            "bundleHandle": bundle.handle,
            "task": bundle.task_id,
            "planHandle": bundle.plan_handle,
            "layout": bundle.layout,
            "machiningIdentity": bundle.machining_identity,
            "files": bundle.manifest["files"],
            "manifest": bundle.manifest,
            "report": bundle.report,
        })
    }

    fn stats_value(&self) -> Value {
        json!({
            "plansRun": self.plans_run,
            "preparesRun": self.prepares_run,
            "retainedPlans": self.plans.len(),
            "retainedBundles": self.bundles.len(),
            "tasks": self.tasks.len(),
        })
    }

    /// Drop oldest terminal task records when the ledger is full; running
    /// and pending tasks are never pruned.
    fn prune_tasks(&mut self) {
        while self.tasks.len() >= RETAINED_TASKS {
            let Some(index) = self.tasks.iter().position(|record| record.state.terminal()) else {
                break;
            };
            self.tasks.remove(index);
        }
    }
}

fn task_value_of(
    (task_id, kind, state, diagnostic, plan, bundle): &(
        String,
        &'static str,
        TaskState,
        Option<Diagnostic>,
        Option<String>,
        Option<String>,
    ),
) -> Value {
    json!({
        "taskId": task_id,
        "kind": kind,
        "state": state.as_str(),
        "diagnostic": diagnostic.as_ref().map(|d| json!({
            "code": d.code, "message": d.message,
        })),
        "planHandle": plan,
        "bundleHandle": bundle,
    })
}

/// Compute a claimed task. Runs without any runtime lock; the outcome is
/// delivered with [`Retained::complete`].
pub fn compute(claimed: ClaimedTask) -> Result<TaskOutcome> {
    match claimed {
        ClaimedTask::Generate {
            task_id,
            plan_handle,
            job,
            scope,
            scope_wire,
        } => {
            let plan = OperationPlanV5::plan_job_v5(&job, &scope, &PlanLimits::default())?;
            let checks = check_plan_v5(&plan)?;
            Ok(TaskOutcome::Plan(Arc::new(RetainedPlan {
                handle: plan_handle,
                task_id,
                scope: scope_wire,
                document_fingerprint: fingerprint(&job),
                machining_identity: plan.machining_identity.clone(),
                trusted: TrustedPlanV5::from_generated(plan),
                checks,
            })))
        }
        ClaimedTask::PrepareOutput {
            task_id,
            bundle_handle,
            plan,
            job,
            scope,
            scope_wire: _,
            layout,
        } => {
            // The submitted document must still match the retained plan's
            // machining identity for the plan's scope: a used-source or
            // effective-value edit since generation makes this preparation
            // stale; output-only edits pass and bind through preparation.
            let Some(plan) = plan else {
                return Err(retained_error(
                    "RETAINED_HANDLE_EXPIRED",
                    "the plan handle expired before preparation started; generate again",
                ));
            };
            let current = OperationPlanV5::machining_identity(&job, &scope)?;
            if current != plan.machining_identity {
                return Err(retained_error(
                    "RETAINED_PLAN_STALE",
                    "the document's machining identity changed since this plan was \
                     generated; generate a new plan before preparing output",
                ));
            }
            let profile = machine::resolve_sequence_profile(&job, &scope)?;
            // Work zero is output-only (H3 excludes it from the machining
            // identity): bind the current selection to the retained
            // setup-coordinate motions instead of regenerating them. The
            // execution fingerprint is unaffected; the prepared-output
            // identity changes exactly as intended.
            let mut snapshot = plan.trusted.plan().clone();
            snapshot.job_snapshot.setup.work_zero = job.setup.work_zero.clone();
            let trusted = TrustedPlanV5::from_generated(snapshot);
            let prepared = PreparedExecution::prepare(&trusted, &profile)?;
            let bundle = prepared.export_bundle(&trusted, &profile, layout)?;
            let mut report = serde_json::to_value(&bundle.report)
                .map_err(|e| retained_error("RETAINED_JSON", e.to_string()))?;
            // GUI6 inspection is tied to these retained bytes and this retained
            // execution. Never replan/re-export through the stateless evidence
            // route just to draw the blade. Bundled-file inspection is GUI10.
            if layout == OutputLayout::OneProgram
                && trusted
                    .plan()
                    .stages
                    .iter()
                    .any(|stage| stage.role == cam_core::sequence::StageRole::Knife)
            {
                let file = &bundle.files[0];
                let decoded = prepared.decode_program(
                    trusted.plan(),
                    prepared.output_decimal_places,
                    &file.gcode,
                )?;
                let evidence = cam_core::operations::drag_knife::evidence::build_evidence(
                    trusted.plan(),
                    &prepared,
                    &decoded,
                    &bundle.manifest.files[0].sha256,
                    2_048,
                )?;
                report["knifeEvidence"] = serde_json::to_value(evidence)
                    .map_err(|e| retained_error("RETAINED_JSON", e.to_string()))?;
            }
            for file in &bundle.files {
                if file.gcode.len() > crate::export::PROGRAM_BYTES {
                    return Err(retained_error(
                        "COLLECTION_PROGRAM_LIMIT",
                        "exported program exceeds the 8 MB service limit",
                    ));
                }
            }
            Ok(TaskOutcome::Bundle(Arc::new(RetainedBundle {
                handle: bundle_handle,
                task_id,
                plan_handle: plan.handle.clone(),
                layout,
                files: bundle.files,
                manifest: serde_json::to_value(&bundle.manifest)
                    .map_err(|e| retained_error("RETAINED_JSON", e.to_string()))?,
                report,
                machining_identity: plan.machining_identity.clone(),
            })))
        }
    }
}
