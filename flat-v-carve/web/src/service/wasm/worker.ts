// Parent worker: owns the engine instance, the task ledger, and one disposable
// compute child at a time. Mirrors the local service's process supervisor.
import init, * as core from '../../wasm/gen/cam_wasm';
import { WasmLedger, type ComputeRunner, type TaskKind } from './ledger';
import type { WasmResponse } from './protocol';

function randomHex(bytes: number): string {
  const value = new Uint8Array(bytes);
  crypto.getRandomValues(value);
  return Array.from(value, byte => byte.toString(16).padStart(2, '0')).join('');
}

function isFailure(value: unknown): value is { status: number; code: string; message: string } {
  return !!value && typeof value === 'object' && 'status' in value && 'code' in value && 'message' in value;
}

function spawnChild(kind: TaskKind, input: unknown): ComputeRunner {
  const child = new Worker(new URL('./child.ts', import.meta.url), { type: 'module' });
  const id = crypto.randomUUID();
  let settled = false;
  const promise = new Promise<{ ok?: unknown; error?: unknown }>((resolve, reject) => {
    child.addEventListener('message', event => {
      const data = event.data as { id: string; reply: { ok?: unknown; error?: unknown } };
      if (data.id !== id || settled) return;
      settled = true;
      resolve(data.reply);
    });
    child.addEventListener('error', () => {
      if (settled) return;
      settled = true;
      reject(new Error('compute worker failed'));
    });
  });
  child.postMessage({ id, kind: kind === 'verification' ? 'verify' : kind, input });
  // Termination is cancellation, not failure; a terminated child never settles.
  return { promise, terminate: () => { child.terminate(); } };
}

let instanceId = '';
let ledger: WasmLedger | undefined;

function reply(id: string, value: unknown, error?: { status: number; code: string; message: string }) {
  const message: WasmResponse = error ? { id, error } : { id, ok: value };
  (self as unknown as Worker).postMessage(message);
}

self.addEventListener('message', async event => {
  const { id, op, payload } = event.data as { id: string; op: string; payload?: unknown };
  if (op !== 'init' && !ledger) {
    reply(id, undefined, { status: 503, code: 'WORKER_NOT_READY', message: 'The engine worker is still starting. Retry.' });
    return;
  }
  if (op === 'init') {
    try {
      await init();
      const limits = (JSON.parse(core.limits()) as { ok: Record<string, number> }).ok;
      const defaultVerificationOptions = (JSON.parse(core.default_verification_options()) as { ok: unknown }).ok;
      instanceId = randomHex(16);
      ledger = new WasmLedger(
        {
          admitPlan: requestJson => JSON.parse(core.admit_plan(requestJson, instanceId)),
          admitVerification: (requestJson, sourceJson) => JSON.parse(core.admit_verification(requestJson, instanceId, sourceJson)),
          admitExport: (requestJson, sourceJson) => JSON.parse(core.admit_export(requestJson, instanceId, sourceJson)),
        },
        spawnChild,
        { pageMotions: limits.pageMotions },
        { instanceId, engineVersion: core.engine_version() },
      );
      reply(id, {
        engineVersion: core.engine_version(),
        instanceId,
        sessionToken: randomHex(32),
        limits,
        defaultVerificationOptions,
      });
    } catch (error) {
      reply(id, undefined, { status: 500, code: 'ENGINE_INIT', message: `The in-browser engine could not start: ${String(error)}` });
    }
    return;
  }
  const engine = ledger!;
  const request = (payload ?? {}) as Record<string, unknown>;
  const result = (op === 'document'
    ? (() => {
      const parsed = JSON.parse(core.document(String(request.requestJson))) as { ok?: unknown; error?: { status: number; code: string; message: string } };
      return parsed.error ?? parsed.ok;
    })()
    : op === 'startPlan' ? engine.startPlan(request as never)
    : op === 'startVerification' ? engine.startVerification(request as never)
    : op === 'startExport' ? engine.startExport(request as never)
    : op === 'task' ? engine.task(String(request.taskId))
    : op === 'cancel' ? engine.cancel(String(request.taskId))
    : op === 'planResult' ? engine.planResult(String(request.taskId))
    : op === 'motionPage' ? engine.motionPage(String(request.taskId), Number(request.offset))
    : op === 'stockSlice' ? engine.stockSlice(String(request.taskId), String(request.sliceId))
    : op === 'verificationResult' ? engine.verificationResult(String(request.taskId))
    : op === 'exportResult' ? engine.exportResult(String(request.taskId))
    : { status: 400, code: 'WORKER_OP', message: `Unknown worker operation ${op}.` });
  if (isFailure(result)) reply(id, undefined, result);
  else reply(id, result);
});
