// Bounded in-memory task ledger for the in-browser engine, mirroring the local
// service's semantics: idempotent immutable tasks, one compute worker at a
// time, cancellation by terminating the disposable compute worker, and a
// four-result retention window with identity-preserving eviction.
//
// Report tasks (verification/export) capture their compute input at admission
// time — the in-memory equivalent of the native service's source-file lease —
// so later eviction of the source plan result cannot starve a queued report.
import type { WasmLimits, WasmReply } from './protocol';

export const MAX_PENDING = 4;
export const MAX_TASKS = 128;
export const RETAINED_RESULTS = 4;
export const TIMEOUT_SECONDS = 300;
export const TASK_KIND_MESSAGE =
  'One calculation is running and three are queued. Wait or cancel a task.';

export type TaskState = 'queued' | 'running' | 'cancelling' | 'cancelled' | 'succeeded' | 'failed';
export type TaskKind = 'plan' | 'verification' | 'export';
export type Stage = 'endmill' | 'combined';

export interface LedgerFailure {
  status: number;
  code: string;
  message: string;
}
interface Diagnostic {
  code: string;
  severity: 'info' | 'warning' | 'error';
  stage?: string;
  message: string;
  sourceId?: string;
}
export interface Snapshot {
  apiVersion: 'ui-7';
  engineVersion: string;
  instanceId: string;
  taskId: string;
  revision: number;
  documentFingerprint: string;
  stage: Stage;
  sequence: number;
  state: TaskState;
  diagnostic: Diagnostic | null;
  summary: Record<string, unknown> | null;
  resultAvailable: boolean;
  verification?: unknown;
  export?: unknown;
}

export interface PlanStart {
  apiVersion: string;
  instanceId: string;
  requestId: string;
  revision: number;
  documentFingerprint: string;
  stage: Stage;
  job: unknown;
}
export interface ReportStart {
  apiVersion: string;
  instanceId: string;
  requestId: string;
  revision: number;
  documentFingerprint: string;
  verification?: Record<string, unknown>;
  export?: Record<string, unknown>;
}

export interface PlanResultPayload {
  summary: Record<string, unknown> & { motionCount: number };
  motions: unknown[];
  inspection: { slices: { info: { id: string } }[] };
  verificationReceipt: unknown;
  planJson: string;
}
export interface VerificationResultPayload {
  summary: Record<string, unknown> & { status: string };
  reportJson: string;
}
export interface ExportResultPayload {
  summary: Record<string, unknown> & { status: string };
  reportJson: string;
  programs: { filename: string; gcode: string }[];
}

interface TaskRecord {
  kind: TaskKind;
  snapshot: Snapshot;
  requestHash: string;
  input: unknown;
  result?: PlanResultPayload | VerificationResultPayload | ExportResultPayload;
  runner?: ComputeRunner;
  timer?: ReturnType<typeof setTimeout>;
}

export interface ComputeRunner {
  promise: Promise<{ ok?: unknown; error?: unknown }>;
  terminate: () => void;
}
export interface LedgerCore {
  admitPlan(requestJson: string): WasmReply<{ documentFingerprint: string; requestHash: string }>;
  admitVerification(requestJson: string, sourceJson: string): WasmReply<{ requestHash: string }>;
  admitExport(requestJson: string, sourceJson: string): WasmReply<{ requestHash: string }>;
}
export type ComputeSpawn = (kind: TaskKind, input: unknown) => ComputeRunner;

export type LedgerResult<T> = T | LedgerFailure;
function failed(value: TaskRecord | LedgerFailure | undefined): value is LedgerFailure {
  return (value as LedgerFailure).code !== undefined;
}

export class WasmLedger {
  private readonly records = new Map<string, TaskRecord>();
  private readonly retained: string[] = [];

  constructor(
    private readonly core: LedgerCore,
    private readonly spawnCompute: ComputeSpawn,
    private readonly limits: Pick<WasmLimits, 'pageMotions'>,
    private readonly ids: { instanceId: string; engineVersion: string },
  ) {}

  private failure(status: number, code: string, message: string): LedgerFailure {
    return { status, code, message };
  }
  private lookup(id: string): TaskRecord | LedgerFailure {
    return this.records.get(id)
      ?? this.failure(404, 'TASK_NOT_FOUND', 'Task was not found in this service instance. It has not been restarted.');
  }
  private active(): number {
    let count = 0;
    for (const record of this.records.values()) {
      if (['queued', 'running', 'cancelling'].includes(record.snapshot.state)) count++;
    }
    return count;
  }
  private busy(): boolean {
    for (const record of this.records.values()) {
      if (record.snapshot.state === 'running') return true;
    }
    return false;
  }

  startPlan(request: PlanStart): LedgerResult<Snapshot> {
    const admitted = this.core.admitPlan(JSON.stringify(request));
    if ('error' in admitted) return admitted.error;
    return this.accept(request.requestId, admitted.ok.requestHash, 'plan', {
      apiVersion: 'ui-7',
      engineVersion: this.ids.engineVersion,
      instanceId: this.ids.instanceId,
      taskId: request.requestId,
      revision: request.revision,
      documentFingerprint: admitted.ok.documentFingerprint,
      stage: request.stage,
      sequence: 1,
      state: 'queued',
      diagnostic: null,
      summary: null,
      resultAvailable: false,
    }, { stage: request.stage, job: JSON.stringify(request.job) });
  }

  startVerification(request: ReportStart): LedgerResult<Snapshot> {
    const sourceId = String(request.verification?.planTaskId ?? '');
    const source = this.lookup(sourceId);
    if (failed(source)) return source;
    if (source.kind !== 'plan' || !source.result) {
      return this.failure(410, 'PLAN_RESULT_UNAVAILABLE', 'Result is unfinished or expired. The service retains only the latest four results.');
    }
    const payload = source.result as PlanResultPayload;
    const admitted = this.core.admitVerification(JSON.stringify(request), this.sourceJson(source));
    if ('error' in admitted) return admitted.error;
    return this.accept(request.requestId, admitted.ok.requestHash, 'verification', {
      apiVersion: 'ui-7',
      engineVersion: this.ids.engineVersion,
      instanceId: this.ids.instanceId,
      taskId: request.requestId,
      revision: source.snapshot.revision,
      documentFingerprint: source.snapshot.documentFingerprint,
      stage: 'combined',
      sequence: 1,
      state: 'queued',
      diagnostic: null,
      summary: null,
      resultAvailable: false,
      verification: request.verification,
    }, { planJson: payload.planJson, receipt: payload.verificationReceipt, identity: request.verification });
  }

  startExport(request: ReportStart): LedgerResult<Snapshot> {
    const sourceId = String(request.export?.planTaskId ?? '');
    const source = this.lookup(sourceId);
    if (failed(source)) return source;
    if (source.kind !== 'plan' || !source.result) {
      return this.failure(410, 'PLAN_RESULT_UNAVAILABLE', 'Result is unfinished or expired. The service retains only the latest four results.');
    }
    const payload = source.result as PlanResultPayload;
    const admitted = this.core.admitExport(JSON.stringify(request), this.sourceJson(source));
    if ('error' in admitted) return admitted.error;
    return this.accept(request.requestId, admitted.ok.requestHash, 'export', {
      apiVersion: 'ui-7',
      engineVersion: this.ids.engineVersion,
      instanceId: this.ids.instanceId,
      taskId: request.requestId,
      revision: source.snapshot.revision,
      documentFingerprint: source.snapshot.documentFingerprint,
      stage: 'combined',
      sequence: 1,
      state: 'queued',
      diagnostic: null,
      summary: null,
      resultAvailable: false,
      export: request.export,
    }, { planJson: payload.planJson, receipt: payload.verificationReceipt, identity: request.export });
  }

  private sourceJson(source: TaskRecord): string {
    return JSON.stringify({
      stage: source.snapshot.stage,
      revision: source.snapshot.revision,
      documentFingerprint: source.snapshot.documentFingerprint,
      verification: source.snapshot.verification ?? null,
      export: source.snapshot.export ?? null,
      summary: source.snapshot.summary,
    });
  }

  private accept(taskId: string, requestHash: string, kind: TaskKind, snapshot: Snapshot, input: unknown): LedgerResult<Snapshot> {
    const existing = this.records.get(taskId);
    if (existing) {
      return existing.requestHash === requestHash
        ? this.public(existing)
        : this.failure(409, 'TASK_KEY_REUSED', 'This request ID already belongs to another immutable input.');
    }
    if (this.records.size >= MAX_TASKS) {
      return this.failure(503, 'TASK_LEDGER_FULL', `This service has accepted ${MAX_TASKS} tasks. Reload the page to clear task history.`);
    }
    if (this.active() >= MAX_PENDING) {
      return this.failure(503, 'PLAN_QUEUE_FULL', TASK_KIND_MESSAGE);
    }
    const record: TaskRecord = { kind, snapshot, requestHash, input };
    this.records.set(taskId, record);
    this.schedule();
    return this.public(record);
  }

  private schedule() {
    if (this.busy()) return;
    for (const record of this.records.values()) {
      if (record.snapshot.state === 'queued') {
        this.run(record);
        return;
      }
    }
  }

  private run(record: TaskRecord) {
    record.snapshot.state = 'running';
    record.snapshot.sequence++;
    const runner = this.spawnCompute(record.kind, record.input);
    record.runner = runner;
    record.timer = setTimeout(() => {
      if (record.snapshot.state !== 'running') return;
      this.terminate(record);
      this.finish(record, { error: timeoutDiagnostic(record.kind) });
    }, TIMEOUT_SECONDS * 1000);
    runner.promise.then(
      reply => {
        if (record.snapshot.state !== 'running') return;
        clearTimeout(record.timer);
        this.finish(record, reply);
      },
      () => {
        if (record.snapshot.state !== 'running') return;
        clearTimeout(record.timer);
        this.finish(record, { error: workerFailureDiagnostic(record.kind) });
      },
    );
  }

  private terminate(record: TaskRecord) {
    clearTimeout(record.timer);
    record.runner?.terminate();
    record.runner = undefined;
  }

  private finish(record: TaskRecord, reply: { ok?: unknown; error?: unknown }) {
    record.runner = undefined;
    record.snapshot.sequence++;
    if (reply.ok !== undefined) {
      const payload = reply.ok as PlanResultPayload & VerificationResultPayload & ExportResultPayload;
      record.snapshot.state = 'succeeded';
      record.snapshot.summary = payload.summary;
      record.snapshot.resultAvailable = true;
      record.result = payload;
      this.retained.push(record.snapshot.taskId);
      while (this.retained.length > RETAINED_RESULTS) {
        const evicted = this.records.get(this.retained.shift()!);
        if (evicted?.result) {
          evicted.result = undefined;
          evicted.snapshot.resultAvailable = false;
          evicted.snapshot.sequence++;
        }
      }
    } else {
      record.snapshot.state = 'failed';
      record.snapshot.diagnostic = reply.error as Diagnostic;
    }
    this.schedule();
  }

  task(id: string): LedgerResult<Snapshot> {
    const record = this.lookup(id);
    return failed(record) ? record : this.public(record);
  }

  cancel(id: string): LedgerResult<Snapshot> {
    const record = this.lookup(id);
    if (failed(record)) return record;
    if (!['cancelled', 'succeeded', 'failed'].includes(record.snapshot.state) && record.snapshot.state !== 'cancelling') {
      record.snapshot.state = 'cancelling';
      record.snapshot.sequence++;
      this.terminate(record);
      record.snapshot.state = 'cancelled';
      record.snapshot.sequence++;
      this.schedule();
    }
    return this.public(record);
  }

  private public(record: TaskRecord): Snapshot {
    return structuredClone(record.snapshot);
  }

  planResult(id: string): LedgerResult<{
    task: Snapshot;
    coordinateSpace: 'workpiece-mm-z-up';
    motions: unknown[];
    nextMotionOffset: number | null;
    stockSlices: unknown[];
  }> {
    const record = this.lookup(id);
    if (failed(record)) return record;
    if (record.kind !== 'plan') {
      return this.failure(422, 'TASK_KIND', 'This task contains report evidence, not a plan preview.');
    }
    const payload = record.result;
    if (!payload) {
      return this.failure(410, 'PLAN_RESULT_UNAVAILABLE', 'Result is unfinished or expired. The service retains only the latest four results.');
    }
    const plan = payload as PlanResultPayload;
    const page = this.page(plan, 0);
    return {
      task: this.public(record),
      coordinateSpace: 'workpiece-mm-z-up',
      motions: page.motions,
      nextMotionOffset: page.nextMotionOffset,
      stockSlices: plan.inspection.slices.map(slice => slice.info),
    };
  }

  motionPage(id: string, offset: number): LedgerResult<{
    task: Snapshot;
    coordinateSpace: 'workpiece-mm-z-up';
    offset: number;
    motions: unknown[];
    nextMotionOffset: number | null;
  }> {
    const record = this.lookup(id);
    if (failed(record)) return record;
    if (record.kind !== 'plan') {
      return this.failure(422, 'TASK_KIND', 'This task has no motion preview.');
    }
    const plan = record.result as PlanResultPayload | undefined;
    if (!plan || !Number.isInteger(offset) || offset < 0 || offset % this.limits.pageMotions !== 0 || offset >= plan.motions.length) {
      return this.failure(404, 'MOTION_PAGE_NOT_FOUND', 'This plan has no motion page at that offset.');
    }
    const page = this.page(plan, offset);
    return {
      task: this.public(record),
      coordinateSpace: 'workpiece-mm-z-up',
      offset,
      motions: page.motions,
      nextMotionOffset: page.nextMotionOffset,
    };
  }

  private page(plan: PlanResultPayload, offset: number) {
    const end = Math.min(offset + this.limits.pageMotions, plan.motions.length);
    return {
      motions: plan.motions.slice(offset, end),
      nextMotionOffset: end < plan.summary.motionCount ? end : null,
    };
  }

  stockSlice(id: string, sliceId: string): LedgerResult<{
    task: Snapshot;
    coordinateSpace: 'workpiece-mm-z-up';
    slice: unknown;
  }> {
    const record = this.lookup(id);
    if (failed(record)) return record;
    const plan = record.result as PlanResultPayload | undefined;
    const slice = plan?.inspection.slices.find(entry => entry.info.id === sliceId);
    if (!slice) {
      return this.failure(404, 'SLICE_NOT_FOUND', 'This plan has no slice with that ID.');
    }
    return { task: this.public(record), coordinateSpace: 'workpiece-mm-z-up', slice };
  }

  verificationResult(id: string): LedgerResult<{
    task: Snapshot;
    coordinateSpace: 'workpiece-mm-z-up';
    report: unknown;
  }> {
    const record = this.lookup(id);
    if (failed(record)) return record;
    if (record.kind !== 'verification') {
      return this.failure(422, 'TASK_KIND', 'This task is not a verification.');
    }
    const payload = record.result as VerificationResultPayload | undefined;
    if (!payload) {
      return this.failure(410, 'PLAN_RESULT_UNAVAILABLE', 'Result is unfinished or expired. The service retains only the latest four results.');
    }
    return {
      task: this.public(record),
      coordinateSpace: 'workpiece-mm-z-up',
      report: JSON.parse(payload.reportJson),
    };
  }

  exportResult(id: string): LedgerResult<{
    task: Snapshot;
    report: unknown;
    reportJson: string;
    programs: { filename: string; gcode: string }[];
  }> {
    const record = this.lookup(id);
    if (failed(record)) return record;
    if (record.kind !== 'export') {
      return this.failure(422, 'TASK_KIND', 'This task is not a LinuxCNC export.');
    }
    const payload = record.result as ExportResultPayload | undefined;
    if (!payload) {
      return this.failure(410, 'PLAN_RESULT_UNAVAILABLE', 'Result is unfinished or expired. The service retains only the latest four results.');
    }
    return {
      task: this.public(record),
      report: JSON.parse(payload.reportJson),
      reportJson: payload.reportJson,
      programs: payload.summary.status === 'passed' ? payload.programs : [],
    };
  }
}

function stageName(kind: TaskKind): 'planning' | 'verification' | 'export' {
  return kind === 'plan' ? 'planning' : kind;
}
function taskPrefix(kind: TaskKind): 'PLAN' | 'VERIFICATION' | 'EXPORT' {
  return kind === 'plan' ? 'PLAN' : kind === 'verification' ? 'VERIFICATION' : 'EXPORT';
}
function timeoutDiagnostic(kind: TaskKind): Diagnostic {
  return {
    code: `${taskPrefix(kind)}_TIMEOUT`,
    severity: 'error',
    stage: stageName(kind),
    message: 'Calculation exceeded the five-minute service limit. Reduce the job or refine its resource limits.',
  };
}
function workerFailureDiagnostic(kind: TaskKind): Diagnostic {
  return {
    code: `${taskPrefix(kind)}_WORKER_FAILURE`,
    severity: 'error',
    stage: stageName(kind),
    message: 'The compute worker exited without a result.',
  };
}
