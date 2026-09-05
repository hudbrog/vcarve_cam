import { z } from 'zod';
import type { Job } from '../contracts/job';
import type { CamService, Capabilities } from '../contracts/service';
import { apiVersion, capabilitiesSchema, displaySchema, envelopeSchema, errorSchema, openedSchema, sessionSchema, validationSchema } from '../contracts/wire';
import { fixtureService } from './fixture';

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
  return {
    capabilities,
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
  };
}
