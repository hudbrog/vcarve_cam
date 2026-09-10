// ui-8 sequence service client. Speaks the same envelope over the local HTTP
// route (/api/v1/sequence) and the wasm worker ('sequence' op), validating
// every reply with the shared zod contracts.
import { z } from 'zod';
import {
  sequenceApiVersion, sequenceCapabilitiesSchema, sequenceDocumentSchema, sequenceEnvelopeSchema,
  exportResultSchema, planResultSchema, type SequenceService,
} from '../contracts/sequence';
import type { WasmConnect, WasmTransport } from './wasm';
import type { WasmInit, WasmReply } from './wasm/protocol';

function parse<T>(schema: z.ZodType<T>, value: unknown): T {
  const result = schema.safeParse(value);
  if (!result.success) throw new Error('The sequence service returned an incompatible response. Rebuild the UI and reconnect.');
  return result.data;
}

interface Dispatcher {
  request(command: object, signal?: AbortSignal): Promise<unknown>;
}

function client(dispatch: Dispatcher): SequenceService {
  let revision = 0;
  async function call<T>(schema: z.ZodType<T>, command: object, signal?: AbortSignal): Promise<T> {
    const requestId = crypto.randomUUID();
    const envelope = parse(sequenceEnvelopeSchema, await dispatch.request(
      { apiVersion: sequenceApiVersion, requestId, revision, command }, signal));
    revision += 1;
    if (envelope.requestId !== requestId) throw new Error('The sequence service returned a different request. This response was discarded.');
    if (envelope.diagnostic) throw new Error(`${envelope.diagnostic.code}: ${envelope.diagnostic.message}`);
    return parse(schema, envelope.data);
  }
  return {
    capabilities: signal => call(sequenceCapabilitiesSchema, { operation: 'capabilities' }, signal),
    open: (json, signal) => call(sequenceDocumentSchema, { operation: 'open', json }, signal),
    edit: (job, edits, signal) => call(sequenceDocumentSchema, { operation: 'edit', job, edits }, signal),
    applyProfile: (job, profile, signal) => call(sequenceDocumentSchema, { operation: "applyProfile", job, profile }, signal),
    updateSettings: (job, operationId, settings, signal) => call(sequenceDocumentSchema, { operation: "updateSettings", job, operationId, settings }, signal),
    plan: (job, scope, signal) => call(planResultSchema, { operation: 'plan', job, scope }, signal),
    export: (job, profile, signal) => call(exportResultSchema, { operation: 'export', job, profile }, signal),
  };
}

// Local HTTP transport: one session token per connection, same admission as ui-7.
export function createHttpSequenceService(fetcher: typeof fetch = (...args) => fetch(...args)): SequenceService {
  let token: string | undefined;
  const dispatcher: Dispatcher = {
    async request(command, signal) {
      token ??= await sessionToken(fetcher, signal);
      const body = JSON.stringify(command);
      const response = await fetcher('/api/v1/sequence', {
        method: 'POST', credentials: 'omit', cache: 'no-store', signal,
        headers: { 'Content-Type': 'application/json', 'X-Cam-Session': token }, body,
      });
      const value = await response.json();
      if (!response.ok && value?.error) throw new Error(`${value.error.code}: ${value.error.message}`);
      return value;
    },
  };
  return client(dispatcher);
}
async function sessionToken(fetcher: typeof fetch, signal?: AbortSignal): Promise<string> {
  const response = await fetcher('/api/v1/session', { credentials: 'omit', cache: 'no-store', signal });
  if (!response.ok) throw new Error('The local service did not answer. Start it and reconnect.');
  return (await response.json()).sessionToken as string;
}

// Wasm worker transport: identical envelope, instance-bound admission.
export function createWasmSequenceService(connect: WasmConnect = defaultConnect): SequenceService {
  let connection: { transport: WasmTransport; init: WasmInit } | undefined;
  const dispatcher: Dispatcher = {
    async request(command, signal) {
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
      const { transport, init } = connection;
      const requestJson = JSON.stringify(command);
      if (new TextEncoder().encode(requestJson).length > init.limits.requestBytes) throw new Error('Request exceeds the engine input limit.');
      const reply: WasmReply = await transport.invoke('sequence', { requestJson });
      if ('error' in reply) throw new Error(`${reply.error.code}: ${reply.error.message}`);
      signal?.throwIfAborted();
      return JSON.parse(String(reply.ok));
    },
  };
  return client(dispatcher);
}
export const defaultConnect: WasmConnect = async () => {
  const { spawnParentWorker } = await import('./wasm/transport');
  return spawnParentWorker();
};

// The standalone mode picker mirrors the legacy service selection.
export function createSequenceService(mode: 'http' | 'wasm' = 'http'): SequenceService {
  return mode === 'wasm' ? createWasmSequenceService() : createHttpSequenceService();
}
