// In-browser CamService backed by the wasm engine running in a Web Worker.
// Speaks the same wire contracts as the local HTTP service, so the UI's zod
// schemas and acceptance helpers validate unchanged.
import { z } from 'zod';
import type { CamService, Capabilities } from '../contracts/service';
import { apiVersion, capabilitiesSchema, displaySchema, envelopeSchema, openedSchema, validationSchema } from '../contracts/wire';
import { fixtureService } from './fixture';
import { acceptTask, motionPageSchema, planResultPageSchema, sliceResponseSchema, taskSchema, type TaskIdentity } from '../contracts/planning';
import { sliceInfoSchema } from '../contracts/stock';
import { acceptVerification, verificationResultSchema, verificationTaskSchema, type VerificationIdentity } from '../contracts/verification';
import { acceptExport, checkExportBytes, exportResultSchema, exportTaskSchema, type ExportIdentity } from '../contracts/export';
import { verificationOptionsSchema } from '../contracts/verificationOptions';
import { MAX_PENDING, MAX_TASKS, RETAINED_RESULTS, TIMEOUT_SECONDS } from './wasm/ledger';
import type { WasmInit, WasmReply } from './wasm/protocol';

export interface WasmTransport {
  invoke(op: string, payload?: unknown): Promise<WasmReply>;
  dispose?(): void;
}
export type WasmConnect = () => Promise<WasmTransport>;

// The generated engine module is imported lazily inside the worker so test
// bundles without a built artifact never load it.
export const defaultConnect: WasmConnect = async () => {
  const { spawnParentWorker } = await import('./wasm/transport');
  return spawnParentWorker();
};

function parse<T>(schema: z.ZodType<T>, value: unknown): T {
  const result = schema.safeParse(value);
  if (!result.success) throw new Error('The in-browser engine returned an incompatible response. Rebuild the UI and refresh.');
  return result.data;
}

export function createWasmService(connect: WasmConnect = defaultConnect): CamService {
  let connection: { transport: WasmTransport; init: WasmInit } | undefined;
  async function connectOnce(signal?: AbortSignal) {
    connection ??= await (async () => {
      let transport: WasmTransport;
      try {
        transport = await connect();
      } catch (error) {
        throw new Error(`The in-browser engine is unavailable. Build it with pnpm build:wasm and reload. (${String(error)})`);
      }
      const reply = await transport.invoke('init');
      if ('error' in reply) throw new Error(`${reply.error.code}: ${reply.error.message}`);
      return { transport, init: reply.ok as WasmInit };
    })();
    signal?.throwIfAborted();
    return connection;
  }
  async function request<T>(command: object, revision: number, schema: z.ZodType<T>, signal?: AbortSignal): Promise<T> {
    const { transport, init } = await connectOnce(signal);
    const requestId = crypto.randomUUID();
    const requestJson = JSON.stringify({ apiVersion, requestId, revision, command });
    if (new TextEncoder().encode(requestJson).length > init.limits.requestBytes) throw new Error('Request exceeds the engine input limit.');
    const reply = await transport.invoke('document', { requestJson });
    if ('error' in reply) throw new Error(`${reply.error.code}: ${reply.error.message}`);
    signal?.throwIfAborted();
    const envelope = parse(envelopeSchema, reply.ok);
    if (envelope.requestId !== requestId || envelope.revision !== revision || envelope.engineVersion !== init.engineVersion) {
      throw new Error('The engine returned a different request, revision, or engine identity. This response was discarded.');
    }
    if (envelope.diagnostic) throw new Error(`${envelope.diagnostic.code}: ${envelope.diagnostic.message}${envelope.diagnostic.sourceId ? ` (source: ${envelope.diagnostic.sourceId})` : ''}`);
    const result = parse(schema, envelope.data);
    const display = displaySchema.safeParse(result);
    const opened = openedSchema.safeParse(result);
    const engine = display.success ? display.data.engineVersion : opened.success ? opened.data.display.engineVersion : null;
    if (engine !== null && engine !== envelope.engineVersion) throw new Error('Display engine identity differs from the accepted response.');
    return result;
  }
  async function taskRequest<T>(identity: TaskIdentity | VerificationIdentity | ExportIdentity, op: string, payload: object | undefined, schema: z.ZodType<T>, signal?: AbortSignal): Promise<T> {
    const { transport, init } = await connectOnce(signal);
    if (identity.instanceId !== init.instanceId || identity.engineVersion !== init.engineVersion) {
      throw new Error('This task belongs to a previous engine session. Reload the page and start a new plan.');
    }
    const reply = await transport.invoke(op, payload);
    if ('error' in reply) throw new Error(`${reply.error.code}: ${reply.error.message}`);
    signal?.throwIfAborted();
    return parse(schema, reply.ok);
  }
  const taskPayload = (identity: TaskIdentity) => ({ taskId: identity.taskId });
  return {
    async capabilities(signal) {
      const { init } = await connectOnce(signal);
      const capabilities: Capabilities = {
        apiVersion, mode: 'live', engineVersion: init.engineVersion,
        importArtwork: true, openJob: true, validateDraft: true,
        planningStages: ['endmill', 'combined'], verificationScopes: ['continuous-stock'], exportFormats: ['linuxcnc'],
        planning: {
          instanceId: init.instanceId, concurrentPlans: 1,
          maxPending: MAX_PENDING, maxTasks: MAX_TASKS, retainedResults: RETAINED_RESULTS,
          timeoutSeconds: TIMEOUT_SECONDS, previewMotions: init.limits.pageMotions, artifactBytes: null,
          stockSlices: true, sliceVertices: init.limits.sliceVertices, inspectionVertices: init.limits.inspectionVertices,
        },
        verification: { defaultOptions: verificationOptionsSchema.parse(init.defaultVerificationOptions) },
        export: { profileBytes: init.limits.profileBytes, programBytes: init.limits.programBytes, layouts: ['combined', 'per_tool'] },
        limits: {
          svgBytes: init.limits.svgBytes, jobBytes: init.limits.jobBytes,
          requestBytes: init.limits.requestBytes, concurrentInspections: 1,
        },
      };
      return parse(capabilitiesSchema, capabilities);
    },
    async openExample(signal) {
      const { job } = await fixtureService.openExample(signal);
      return request({ operation: 'open', json: JSON.stringify(job) }, 0, openedSchema, signal);
    },
    displayFor: (job, signal) => request({ operation: 'display', svg: job.source.svg, options: job.import }, 0, displaySchema, signal),
    async validateDraft(job, revision, signal) {
      return { ...await request({ operation: 'validate', job }, revision, validationSchema, signal), revision };
    },
    openJob: (json, revision, signal) => request({ operation: 'open', json }, revision, openedSchema, signal),
    importArtwork: (filename, svg, options, revision, signal) =>
      request({ operation: 'import', filename, svg, options }, revision, openedSchema, signal),
    async startPlan(job, identity, signal) {
      const task = await taskRequest(identity, 'startPlan', {
        apiVersion, instanceId: identity.instanceId, requestId: identity.taskId,
        revision: identity.revision, documentFingerprint: identity.documentFingerprint,
        stage: identity.stage, job,
      }, taskSchema, signal);
      return acceptTask(null, task, identity);
    },
    async planTask(identity, signal) {
      return acceptTask(null, await taskRequest(identity, 'task', taskPayload(identity), taskSchema, signal), identity);
    },
    async cancelPlan(identity, signal) {
      return acceptTask(null, await taskRequest(identity, 'cancel', taskPayload(identity), taskSchema, signal), identity);
    },
    async planResult(identity, signal, onProgress) {
      const { nextMotionOffset: firstOffset, ...result } = await taskRequest(identity, 'planResult', taskPayload(identity), planResultPageSchema, signal);
      acceptTask(null, result.task, identity);
      const summary = JSON.stringify(result.task.summary);
      let next = firstOffset;
      onProgress?.(result.motions.length, result.task.summary!.previewMotionCount);
      while (next !== null) {
        const page = await taskRequest(identity, 'motionPage', { ...taskPayload(identity), offset: next }, motionPageSchema, signal);
        acceptTask(result.task, page.task, identity);
        if (page.offset !== next || JSON.stringify(page.task.summary) !== summary) {
          throw new Error('The engine returned a different motion page or plan summary. The preview was discarded.');
        }
        for (const motion of page.motions) result.motions.push(motion);
        next = page.nextMotionOffset;
        onProgress?.(result.motions.length, result.task.summary!.previewMotionCount);
      }
      signal?.throwIfAborted();
      return result;
    },
    async stockSlice(identity, slice, signal) {
      const result = await taskRequest(identity, 'stockSlice', { ...taskPayload(identity), sliceId: slice.id }, sliceResponseSchema, signal);
      acceptTask(null, result.task, identity);
      if (JSON.stringify(result.slice.info) !== JSON.stringify(parse(sliceInfoSchema, slice))) {
        throw new Error('The engine returned different slice metadata. This response was discarded.');
      }
      return result;
    },
    async startVerification(identity, signal) {
      const task = await taskRequest(identity, 'startVerification', {
        apiVersion, instanceId: identity.instanceId, requestId: identity.taskId,
        revision: identity.revision, documentFingerprint: identity.documentFingerprint,
        verification: identity.verification,
      }, verificationTaskSchema, signal);
      return acceptVerification(null, task, identity);
    },
    async verificationTask(identity, signal) {
      return acceptVerification(null, await taskRequest(identity, 'task', taskPayload(identity), verificationTaskSchema, signal), identity);
    },
    async cancelVerification(identity, signal) {
      return acceptVerification(null, await taskRequest(identity, 'cancel', taskPayload(identity), verificationTaskSchema, signal), identity);
    },
    async verificationResult(identity, signal) {
      const result = await taskRequest(identity, 'verificationResult', taskPayload(identity), verificationResultSchema, signal);
      acceptVerification(null, result.task, identity);
      return result;
    },
    async startExport(identity, signal) {
      const task = await taskRequest(identity, 'startExport', {
        apiVersion, instanceId: identity.instanceId, requestId: identity.taskId,
        revision: identity.revision, documentFingerprint: identity.documentFingerprint,
        export: identity.export,
      }, exportTaskSchema, signal);
      return acceptExport(null, task, identity);
    },
    async exportTask(identity, signal) {
      return acceptExport(null, await taskRequest(identity, 'task', taskPayload(identity), exportTaskSchema, signal), identity);
    },
    async cancelExport(identity, signal) {
      return acceptExport(null, await taskRequest(identity, 'cancel', taskPayload(identity), exportTaskSchema, signal), identity);
    },
    async exportResult(identity, signal) {
      const result = await taskRequest(identity, 'exportResult', taskPayload(identity), exportResultSchema, signal);
      acceptExport(null, result.task, identity);
      await checkExportBytes(result);
      signal?.throwIfAborted();
      return result;
    },
  };
}
