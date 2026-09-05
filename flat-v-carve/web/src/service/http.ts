import { acceptLibraryCandidate, librarySnapshotSchema, libraryCandidateSchema, type LibraryConnection, type LibraryJobInput } from '../contracts/library';
import { acceptExport, exportTaskSchema, exportResultSchema, checkExportBytes } from '../contracts/export';
import { z } from 'zod';
import type { Job } from '../contracts/job';
import type { CamService, Capabilities } from '../contracts/service';
import { apiVersion, capabilitiesSchema, displaySchema, envelopeSchema, errorSchema, openedSchema, sessionSchema, validationSchema } from '../contracts/wire';
import { fixtureService } from './fixture';
import { acceptTask, planResultSchema, sliceResponseSchema, taskSchema, type TaskIdentity } from '../contracts/planning';
import { sliceInfoSchema } from '../contracts/stock';
import { acceptVerification, verificationTaskSchema, verificationResultSchema } from '../contracts/verification';

// Only same-origin URLs. Session tokens stay in memory and are never persisted or logged.
export function createHttpService(fetcher: typeof fetch = (...args) => fetch(...args)): CamService {
  let connection: { token: string; capabilities: Capabilities } | undefined;
  function parse<T>(schema: z.ZodType<T>, value: unknown): T {
    const result = schema.safeParse(value);
    if (!result.success) throw new Error('The local service returned an incompatible response. Rebuild the UI and restart the service.');
    return result.data;
  }
  async function read(path: string, init: RequestInit, signal?: AbortSignal) {
    const timeout = AbortSignal.timeout(30_000);
    const response = await fetcher(`/api/v1/${path}`, { ...init, credentials: 'omit', cache: 'no-store',
      signal: signal ? AbortSignal.any([signal, timeout]) : timeout });
    if (response.status === 401) {
      connection = undefined;
      throw new Error('The local service restarted or the session expired. Reconnect to validate again; your edits are kept.');
    }
    if (!response.headers.get('content-type')?.includes('application/json')) throw new Error('The local service did not return the UI API. Start cam-web with the current production build.');
    const value: unknown = await response.json();
    const failure = errorSchema.safeParse(value);
    if (failure.success) throw new Error(`${failure.data.error.code}: ${failure.data.error.message}`);
    return { response, value };
  }
  async function capabilities(signal?: AbortSignal): Promise<Capabilities> {
    connection = undefined;
    const session = parse(sessionSchema, (await read('session', {}, signal)).value);
    const next = parse(capabilitiesSchema, (await read('capabilities', { headers: { 'X-Cam-Session': session.sessionToken } }, signal)).value);
    if (next.engineVersion !== session.engineVersion) throw new Error('The engine changed while connecting. Reconnect to retry.');
    signal?.throwIfAborted();
    connection = { token: session.sessionToken, capabilities: next };
    return next;
  }
  async function request<T>(command: object, revision: number, schema: z.ZodType<T>, signal?: AbortSignal): Promise<T> {
    if (!connection) throw new Error('Reconnect to the local service before continuing. Your draft is kept.');
    const accepted = connection;
    const requestId = crypto.randomUUID();
    const body = JSON.stringify({ apiVersion, requestId, revision, command });
    if (new TextEncoder().encode(body).length > accepted.capabilities.limits!.requestBytes) throw new Error('Request exceeds the local service input limit.');
    const { response, value } = await read('document', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Cam-Session': accepted.token }, body }, signal);
    const envelope = parse(envelopeSchema, value);
    if (envelope.requestId !== requestId || envelope.revision !== revision || envelope.engineVersion !== accepted.capabilities.engineVersion)
      throw new Error('The service returned a different request, revision, or engine identity. This response was discarded.');
    if (envelope.diagnostic) throw new Error(`${envelope.diagnostic.code}: ${envelope.diagnostic.message}${envelope.diagnostic.sourceId ? ` (source: ${envelope.diagnostic.sourceId})` : ''}`);
    if (!response.ok) throw new Error(`Local service request failed (${response.status}).`);
    signal?.throwIfAborted();
    const result = parse(schema, envelope.data);
    const display = displaySchema.safeParse(result);
    const opened = openedSchema.safeParse(result);
    const engine = display.success ? display.data.engineVersion : opened.success ? opened.data.display.engineVersion : null;
    if (engine !== null && engine !== envelope.engineVersion) throw new Error('Display engine identity differs from the accepted response.');
    return result;
  }
  async function taskRequest<T>(identity: TaskIdentity, path: string, schema: z.ZodType<T>, body?: object, signal?: AbortSignal): Promise<T> {
    if (!connection) throw new Error('Reconnect to check this task. Disconnecting does not cancel planning.');
    const accepted = connection;
    if (identity.instanceId !== accepted.capabilities.planning?.instanceId || identity.engineVersion !== accepted.capabilities.engineVersion)
      throw new Error('This task belongs to a previous service instance. It has not been restarted. Start a new plan explicitly.');
    const payload = body === undefined ? undefined : JSON.stringify(body);
    if (payload && new TextEncoder().encode(payload).length > accepted.capabilities.limits!.requestBytes) throw new Error('Request exceeds the service input limit.');
    const { response, value } = await read(path, { method: payload === undefined ? 'GET' : 'POST',
      headers: { 'X-Cam-Session': accepted.token, 'Content-Type': 'application/json' }, body: payload }, signal);
    if (!response.ok) throw new Error(`Task request failed (${response.status}).`);
    signal?.throwIfAborted();
    return parse(schema, value);
  }
  async function libraryRequest<T>(identity:LibraryConnection,command:object,schema:z.ZodType<T>,hasJob=false,signal?:AbortSignal):Promise<T> {
    if (!connection || !connection.capabilities.toolLibrary) throw new Error('Reconnect to a service with a local tool library.');
    const accepted=connection;
    if (identity.instanceId!==accepted.capabilities.planning?.instanceId || identity.engineVersion!==accepted.capabilities.engineVersion)
      throw new Error('The library service changed. Reload the library and review the latest record before saving.');
    const requestId=crypto.randomUUID(),body=JSON.stringify({apiVersion,instanceId:identity.instanceId,requestId,command});
    if (new TextEncoder().encode(body).length > (hasJob ? accepted.capabilities.limits!.requestBytes : 16_100_000)) throw new Error('Library request exceeds the service input limit.');
    const {response,value}=await read(hasJob ? 'library/job' : 'library',{method:'POST',headers:{'Content-Type':'application/json','X-Cam-Session':accepted.token},body},signal);
    if (!response.ok) throw new Error(`Library request failed (${response.status}). Reload before retrying a write.`);
    const envelope=parse(z.object({apiVersion:z.literal(apiVersion),engineVersion:z.string(),instanceId:z.string(),requestId:z.string()}),value);
    if (envelope.engineVersion!==identity.engineVersion || envelope.instanceId!==identity.instanceId || envelope.requestId!==requestId)
      throw new Error('The library response belongs to another request or service. Reload before continuing.');
    if (connection !== accepted) throw new Error('The library connection changed while the request was running. Reload before continuing; a write may have completed.');
    signal?.throwIfAborted();return parse(schema,value);
  }
  const libraryJob=(input:LibraryJobInput) => ({json:JSON.stringify(input.job),revision:input.revision,document_fingerprint:input.documentFingerprint});
  return {
    capabilities,
    library:(c,signal)=>libraryRequest(c,{operation:'load'},librarySnapshotSchema,false,signal),
    initializeLibrary:(c,signal)=>libraryRequest(c,{operation:'initialize'},librarySnapshotSchema,false,signal),
    changeLibrary:(c,revision,change,signal)=>libraryRequest(c,{operation:'change',expected_revision:revision,change_json:JSON.stringify(change)},librarySnapshotSchema,false,signal),
    importLibrary:(c,revision,json,signal)=>libraryRequest(c,{operation:'import',expected_revision:revision,json},librarySnapshotSchema,false,signal),
    captureLibraryTool:(c,input,signal)=>libraryRequest(c,{operation:'capture',expected_revision:input.expectedRevision,job:libraryJob(input),slot:input.slot,tool_id:input.toolId,name:input.name,preset:input.preset},librarySnapshotSchema,true,signal),
    async applyLibraryTool(c,input,signal) {
      return acceptLibraryCandidate(await libraryRequest(c,{operation:'apply',expected_revision:input.expectedRevision,job:libraryJob(input),slot:input.slot,tool_id:input.toolId,preset_id:input.presetId},libraryCandidateSchema,true,signal),input);
    },
    async startExport(identity, signal) {
      return acceptExport(null, await taskRequest(identity, 'exports', exportTaskSchema, {
        apiVersion, instanceId:identity.instanceId, requestId:identity.taskId, revision:identity.revision,
        documentFingerprint:identity.documentFingerprint, export:identity.export,
      }, signal), identity);
    },
    async exportTask(identity, signal) {
      return acceptExport(null, await taskRequest(identity, `tasks/${encodeURIComponent(identity.taskId)}`, exportTaskSchema, undefined, signal), identity);
    },
    async cancelExport(identity, signal) {
      return acceptExport(null, await taskRequest(identity, `tasks/${encodeURIComponent(identity.taskId)}/cancel`, exportTaskSchema, {}, signal), identity);
    },
    async exportResult(identity, signal) {
      const result = await taskRequest(identity, `tasks/${encodeURIComponent(identity.taskId)}/export`, exportResultSchema, undefined, signal);
      acceptExport(null,result.task,identity);
      await checkExportBytes(result);
      signal?.throwIfAborted();
      return result;
    },
    async startVerification(identity, signal) {
      const task = await taskRequest(identity, 'verifications', verificationTaskSchema, {
        apiVersion, instanceId: identity.instanceId, requestId: identity.taskId, revision: identity.revision,
        documentFingerprint: identity.documentFingerprint, verification: identity.verification,
      }, signal);
      return acceptVerification(null, task, identity);
    },
    async verificationTask(identity, signal) {
      return acceptVerification(null, await taskRequest(identity, `tasks/${encodeURIComponent(identity.taskId)}`, verificationTaskSchema, undefined, signal), identity);
    },
    async cancelVerification(identity, signal) {
      return acceptVerification(null, await taskRequest(identity, `tasks/${encodeURIComponent(identity.taskId)}/cancel`, verificationTaskSchema, {}, signal), identity);
    },
    async verificationResult(identity, signal) {
      const result = await taskRequest(identity, `tasks/${encodeURIComponent(identity.taskId)}/verification`, verificationResultSchema, undefined, signal);
      acceptVerification(null, result.task, identity);
      return result;
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
    importArtwork: (filename, svg, options: Job['import'], revision, signal) => request({ operation: 'import', filename, svg, options }, revision, openedSchema, signal),
    async startPlan(job, identity, signal) {
      const task = await taskRequest(identity, 'tasks', taskSchema, { apiVersion, instanceId: identity.instanceId,
        requestId: identity.taskId, revision: identity.revision, documentFingerprint: identity.documentFingerprint, stage: identity.stage, job }, signal);
      return acceptTask(null, task, identity);
    },
    async planTask(identity, signal) {
      return acceptTask(null, await taskRequest(identity, `tasks/${encodeURIComponent(identity.taskId)}`, taskSchema, undefined, signal), identity);
    },
    async cancelPlan(identity, signal) {
      return acceptTask(null, await taskRequest(identity, `tasks/${encodeURIComponent(identity.taskId)}/cancel`, taskSchema, {}, signal), identity);
    },
    async planResult(identity, signal) {
      const result = await taskRequest(identity, `tasks/${encodeURIComponent(identity.taskId)}/result`, planResultSchema, undefined, signal);
      acceptTask(null, result.task, identity);
      return result;
    },
    async stockSlice(identity, slice, signal) {
      const result = await taskRequest(identity, `tasks/${encodeURIComponent(identity.taskId)}/slices/${encodeURIComponent(slice.id)}`, sliceResponseSchema, undefined, signal);
      acceptTask(null, result.task, identity);
      if (JSON.stringify(result.slice.info) !== JSON.stringify(parse(sliceInfoSchema, slice))) throw new Error('The service returned different slice metadata. This response was discarded.');
      return result;
    },
  };
}
