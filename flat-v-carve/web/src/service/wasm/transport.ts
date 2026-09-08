// Main-thread transport over the engine parent worker.
import type { WasmTransport } from '../wasm';
import type { WasmResponse } from './protocol';

export function spawnParentWorker(): WasmTransport {
  const worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' });
  const pending = new Map<string, { resolve: (reply: WasmResponse) => void; reject: (error: Error) => void }>();
  worker.addEventListener('message', event => {
    const message = event.data as WasmResponse;
    const entry = pending.get(message.id);
    if (!entry) return;
    pending.delete(message.id);
    if ('error' in message) entry.reject(new Error(`${message.error.code}: ${message.error.message}`));
    else entry.resolve(message);
  });
  worker.addEventListener('error', () => {
    const failure = new Error('The in-browser engine worker failed. Reload the page.');
    for (const entry of pending.values()) entry.reject(failure);
    pending.clear();
  });
  return {
    invoke(op, payload) {
      return new Promise((resolve, reject) => {
        const id = crypto.randomUUID();
        pending.set(id, {
          resolve: reply => resolve('ok' in reply ? { ok: reply.ok } : { error: reply.error }),
          reject,
        });
        worker.postMessage({ id, op, payload });
      });
    },
    dispose: () => worker.terminate(),
  };
}
