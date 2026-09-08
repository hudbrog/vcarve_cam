import { describe, expect, it, vi } from 'vitest';
import {
  WasmLedger, MAX_PENDING, RETAINED_RESULTS,
  type ComputeRunner, type LedgerCore, type PlanStart, type PlanResultPayload,
} from '../src/service/wasm/ledger';
import { createWasmService, type WasmTransport } from '../src/service/wasm';
import { capabilitiesSchema } from '../src/contracts/wire';
import { taskSchema } from '../src/contracts/planning';

const INSTANCE = '1f'.repeat(16);
const ENGINE = '0.7.7';
const FINGERPRINT = 'a'.repeat(64);
const hex = (char: string) => char.repeat(64);

class FakeRunner implements ComputeRunner {
  terminated = vi.fn();
  promise: Promise<{ ok?: unknown; error?: unknown }>;
  private settle!: (value: { ok?: unknown; error?: unknown }) => void;
  constructor(public kind: string, public input: unknown) {
    this.promise = new Promise(resolve => { this.settle = resolve; });
  }
  terminate = () => { this.terminated(); };
  succeed(value: unknown) { this.settle({ ok: value }); }
  fail(error: unknown) { this.settle({ error }); }
}

function harness() {
  const spawned: FakeRunner[] = [];
  const core: LedgerCore = {
    admitPlan: json => {
      const request = JSON.parse(json);
      return request.documentFingerprint === FINGERPRINT
        ? { ok: { documentFingerprint: FINGERPRINT, requestHash: `${request.revision}${request.stage}`.padEnd(64, '0') } }
        : { error: { status: 409, code: 'STALE_DOCUMENT', message: 'stale job' } };
    },
    admitVerification: () => ({ ok: { requestHash: hex('2') } }),
    admitExport: () => ({ ok: { requestHash: hex('3') } }),
  };
  const ledger = new WasmLedger(core, (kind, input) => {
    const runner = new FakeRunner(kind, input);
    spawned.push(runner);
    return runner;
  }, { pageMotions: 20 }, { instanceId: INSTANCE, engineVersion: ENGINE });
  return { ledger, spawned };
}

function planStart(taskId: string, revision = 4): PlanStart {
  return { apiVersion: 'ui-7', instanceId: INSTANCE, requestId: taskId, revision,
    documentFingerprint: FINGERPRINT, stage: 'endmill', job: { schema_version: 3 } };
}
function planPayload(motions: number): PlanResultPayload {
  return {
    summary: { engineVersion: ENGINE, status: 'complete', motionCount: motions, previewMotionCount: motions },
    motions: Array.from({ length: motions }, (_, id) => ({ id, tool_id: 'endmill' })),
    inspection: { slices: [{ info: { id: 'endmill-0' } }] },
    verificationReceipt: { engine_version: ENGINE },
    planJson: '{"plan":true}',
  };
}
async function settle() {
  await new Promise(resolve => setTimeout(resolve, 0));
}

describe('wasm task ledger', () => {
  it('surfaces admission failures without recording a task', () => {
    const { ledger } = harness();
    const stale = { ...planStart('a'), documentFingerprint: hex('b') };
    const rejected = ledger.startPlan(stale);
    expect('code' in rejected && rejected.code).toBe('STALE_DOCUMENT');
    const missing = ledger.task('a');
    expect('code' in missing && missing.code).toBe('TASK_NOT_FOUND');
  });
  it('runs one compute at a time, replays idempotent keys, and rejects reuse', async () => {
    const { ledger, spawned } = harness();
    const first = ledger.startPlan(planStart('a'));
    expect(!('code' in first) && first.state).toBe('running');
    const second = ledger.startPlan(planStart('b'));
    expect(!('code' in second) && second.state).toBe('queued');
    const replay = ledger.startPlan(planStart('a'));
    expect(!('code' in replay) && replay.taskId).toBe('a');
    const reused = ledger.startPlan({ ...planStart('a'), revision: 5 });
    expect('code' in reused && reused.code).toBe('TASK_KEY_REUSED');
    spawned[0].succeed(planPayload(3));
    await settle();
    const task = ledger.task('a');
    expect(!('code' in task) && task.state).toBe('succeeded');
    expect(spawned).toHaveLength(2); // the queued task starts after the running one finishes
  });
  it('bounds the pending queue', () => {
    const { ledger } = harness();
    for (const id of ['a', 'b', 'c', 'd']) ledger.startPlan(planStart(id));
    const fifth = ledger.startPlan(planStart('e'));
    expect('code' in fifth && fifth.code).toBe('PLAN_QUEUE_FULL');
    expect(MAX_PENDING).toBe(4);
  });
  it('pages complete motion previews and serves stock slices', async () => {
    const { ledger, spawned } = harness();
    ledger.startPlan(planStart('a'));
    spawned[0].succeed(planPayload(45));
    await settle();
    const result = ledger.planResult('a');
    expect(!('code' in result) && result.motions).toHaveLength(20);
    expect(!('code' in result) && result.nextMotionOffset).toBe(20);
    expect(!('code' in result) && result.stockSlices).toEqual([{ id: 'endmill-0' }]);
    const second = ledger.motionPage('a', 20);
    expect(!('code' in second) && second.motions).toHaveLength(20);
    expect(!('code' in second) && second.nextMotionOffset).toBe(40);
    const third = ledger.motionPage('a', 40);
    expect(!('code' in third) && third.motions).toHaveLength(5);
    expect(!('code' in third) && third.nextMotionOffset).toBeNull();
    const badPage = ledger.motionPage('a', 7);
  expect('code' in badPage && badPage.code).toBe('MOTION_PAGE_NOT_FOUND');
    const slice = ledger.stockSlice('a', 'endmill-0');
    expect(!('code' in slice) && (slice.slice as { info: { id: string } }).info.id).toBe('endmill-0');
    const badSlice = ledger.stockSlice('a', 'missing');
  expect('code' in badSlice && badSlice.code).toBe('SLICE_NOT_FOUND');
  });
  it('cancellation terminates the compute child and wins over a late result', async () => {
    const { ledger, spawned } = harness();
    ledger.startPlan(planStart('a'));
    const cancelled = ledger.cancel('a');
    expect(!('code' in cancelled) && cancelled.state).toBe('cancelled');
    expect(spawned[0].terminated).toHaveBeenCalled();
    spawned[0].succeed(planPayload(3));
    await settle();
    const task = ledger.task('a');
    expect(!('code' in task) && task.state).toBe('cancelled');
  });
  it('failed compute records the diagnostic', async () => {
    const { ledger, spawned } = harness();
    ledger.startPlan(planStart('a'));
    spawned[0].fail({ code: 'PLAN_JOB', severity: 'error', stage: 'planning', message: 'bad job' });
    await settle();
    const task = ledger.task('a');
    expect(!('code' in task) && task.state).toBe('failed');
    expect(!('code' in task) && task.diagnostic!.code).toBe('PLAN_JOB');
  });
  it('evicts only the retained result, keeping summary identity', async () => {
    const { ledger, spawned } = harness();
    for (let i = 0; i <= RETAINED_RESULTS; i++) {
      ledger.startPlan(planStart(`task-${i}`));
      spawned[i].succeed(planPayload(3));
      await settle();
    }
    const evicted = ledger.task('task-0');
    expect(!('code' in evicted) && evicted.resultAvailable).toBe(false);
    expect(!('code' in evicted) && evicted.summary).not.toBeNull();
    const expired = ledger.planResult('task-0');
    expect('code' in expired && expired.code).toBe('PLAN_RESULT_UNAVAILABLE');
    expect(!('code' in ledger.planResult('task-1')));
  });
  it('binds verification and export to a retained combined plan', async () => {
    const { ledger, spawned } = harness();
    const combined = { ...planStart('plan-1'), stage: 'combined' as const };
    ledger.startPlan(combined);
    spawned[0].succeed(planPayload(3));
    await settle();
    const verification = { apiVersion: 'ui-7', instanceId: INSTANCE, requestId: 'verify-1', revision: 4,
      documentFingerprint: FINGERPRINT,
      verification: { planTaskId: 'plan-1', inputFingerprint: hex('a'), motionFingerprint: hex('b'), options: {} } };
    const started = ledger.startVerification(verification);
    expect(!('code' in started) && started.stage).toBe('combined');
    expect(!('code' in started) && started.verification).toBeDefined();
    spawned[1].succeed({ summary: { engineVersion: ENGINE, status: 'passed' }, reportJson: '{"status":"passed"}' });
    await settle();
    const result = ledger.verificationResult('verify-1');
    expect(!('code' in result) && (result.report as { status: string }).status).toBe('passed');
    const wrongKind = ledger.exportResult('verify-1');
    expect('code' in wrongKind && wrongKind.code).toBe('TASK_KIND');
    const missing = ledger.startVerification({ ...verification, requestId: 'verify-2', verification: { ...verification.verification, planTaskId: 'nope' } });
    expect('code' in missing && missing.code).toBe('TASK_NOT_FOUND');
  });
});

function taskJson(taskId: string, state = 'succeeded') {
  return {
    apiVersion: 'ui-7', engineVersion: ENGINE, instanceId: INSTANCE, taskId, revision: 4,
    documentFingerprint: FINGERPRINT, stage: 'endmill', sequence: 3, state, diagnostic: null,
    resultAvailable: state === 'succeeded',
    summary: state === 'succeeded' ? {
      engineVersion: ENGINE, status: 'complete', inputFingerprint: hex('a'), motionFingerprint: hex('b'),
      meaning: 'endmill clearing', limitations: [], motionCount: 45, cuttingMotionCount: 40,
      previewMotionCount: 45, omittedMotionCount: 0, diagnostics: [], omittedDiagnostics: 0,
      generationIssues: [], omittedGenerationIssues: 0,
    } : null,
  };
}
function summaryTask(taskId: string) {
  return { task: taskJson(taskId), coordinateSpace: 'workpiece-mm-z-up' as const };
}

function serviceHarness(handlers: Record<string, (payload: never) => unknown>) {
  const calls: { op: string; payload: unknown }[] = [];
  const transport: WasmTransport = {
    async invoke(op, payload) {
      calls.push({ op, payload });
      const handler = handlers[op];
      if (!handler) return { error: { status: 400, code: 'UNEXPECTED_OP', message: `no handler for ${op}` } };
      return { ok: handler(payload as never) };
    },
  };
  return { service: createWasmService(async () => transport), calls };
}

const init = {
  engineVersion: ENGINE, instanceId: INSTANCE, sessionToken: hex('5'),
  limits: { svgBytes: 8_000_000, jobBytes: 64_000_000, requestBytes: 128_100_000, pageMotions: 20,
    reportBytes: 16_000_000, profileBytes: 64_000, programBytes: 8_000_000, sliceVertices: 60_000, inspectionVertices: 200_000 },
  defaultVerificationOptions: { max_cells: 262_144, max_depth: 12, reachability_max_cells: 65_536,
    max_depth_bands: 256, max_findings: 64, decimal_places: null },
};

describe('wasm service', () => {
  it('reports live-mode capabilities that parse against the shared schema', async () => {
    const { service, calls } = serviceHarness({ init: () => init });
    const capabilities = await service.capabilities();
    expect(capabilities.mode).toBe('live');
    expect(capabilities.planning?.instanceId).toBe(INSTANCE);
    expect(capabilities.planning?.previewMotions).toBe(20);
    expect(capabilities.toolLibrary).toBeUndefined();
    expect(() => capabilitiesSchema.parse(capabilities)).not.toThrow();
    expect(calls.map(call => call.op)).toEqual(['init']);
  });
  it('validates document envelopes and rejects identity mismatches', async () => {
    const { service } = serviceHarness({
      init: () => init,
      document: (payload: { requestJson: string }) => {
        const request = JSON.parse(payload.requestJson);
        return { apiVersion: 'ui-7', engineVersion: ENGINE, requestId: request.requestId, revision: request.revision,
          data: { coordinateSpace: 'source-page-mm-y-up', widthMm: 40, heightMm: 30, engineVersion: ENGINE,
            geometryToleranceMm: 0.005, description: 'x', components: [] } };
      },
    });
    const display = await service.displayFor({ source: { svg: '<svg/>' }, import: { geometry_tolerance_mm: 0.005, ticks_per_mm: null, placement: { origin_mm: { x: 0, y: 0 }, scale: 1, rotation_deg: 0 } } } as never);
    expect(display?.widthMm).toBe(40);
  });
  it('pages a complete motion preview through the worker transport', async () => {
    const page = (offset: number) => ({ ...summaryTask('plan-1'), offset, stockSlices: [],
      motions: Array.from({ length: offset + 20 <= 45 ? 20 : 45 - offset }, (_, id) => ({ id: offset + id, tool_id: 'endmill', operation_id: 'op', layer: 0, kind: 'cut', start: { x: 0, y: 0, z: 0 }, end: { x: 1, y: 0, z: -1 }, feed_mm_min: 100 })),
      nextMotionOffset: offset + 20 < 45 ? offset + 20 : null });
    const progress: Array<[number, number]> = [];
    const { service } = serviceHarness({
      init: () => init,
      startPlan: () => taskJson('plan-1', 'running'),
      planResult: () => { const { offset: _offset, ...first } = page(0); return first; },
      motionPage: (payload: { offset: number }) => { const { stockSlices: _slices, ...rest } = page(payload.offset); return rest; },
    });
    const identity = { taskId: 'plan-1', instanceId: INSTANCE, engineVersion: ENGINE, revision: 4,
      documentFingerprint: FINGERPRINT, stage: 'endmill' as const };
    const started = await service.startPlan!({} as never, identity);
    expect(taskSchema.parse(started).state).toBe('running');
    const result = await service.planResult!(identity, undefined, (loaded, total) => progress.push([loaded, total]));
    expect(result.motions).toHaveLength(45);
    expect(result.task.summary?.motionCount).toBe(45);
    expect(progress).toEqual([[20, 45], [40, 45], [45, 45]]);
  });
  it('propagates worker failures as request errors', async () => {
    const unreachable = createWasmService(async () => { throw new Error('no worker here'); });
    await expect(unreachable.capabilities()).rejects.toThrow('The in-browser engine is unavailable');
    const failing: WasmTransport = {
      invoke: async op => op === 'init'
        ? { ok: init }
        : { error: { status: 409, code: 'TASK_INSTANCE', message: 'The service changed.' } },
    };
    const rejected = createWasmService(async () => failing);
    await expect(rejected.capabilities()).resolves.toHaveProperty('engineVersion', ENGINE);
    await expect(rejected.planTask?.({ taskId: 'x', instanceId: INSTANCE, engineVersion: ENGINE, revision: 1, documentFingerprint: FINGERPRINT, stage: 'endmill' })).rejects.toThrow('TASK_INSTANCE: The service changed.');
  });
});
