// Service selection for a page that may be served by the local portable app
// or statically hosted. An explicit ?mode= override bypasses detection.
import type { CamService } from '../contracts/service';
import { apiVersion } from '../contracts/wire';
import { createHttpService } from './http';
import { createWasmService } from './wasm';

async function localServiceAvailable(): Promise<boolean> {
  try {
    const response = await fetch('/api/v1/session', { cache: 'no-store', credentials: 'omit', signal: AbortSignal.timeout(2000) });
    if (!response.ok) return false;
    const value: unknown = await response.json();
    return !!value && typeof value === 'object' && 'apiVersion' in value && 'sessionToken' in value
      && (value as { apiVersion?: string }).apiVersion === apiVersion
      && typeof (value as { sessionToken?: unknown }).sessionToken === 'string';
  } catch {
    return false;
  }
}

// Detection runs once, lazily; every method routes through the winner so the
// app never observes a mixed service.
export function createAutoService(): CamService {
  let delegate: CamService | undefined;
  let detecting: Promise<void> | undefined;
  const service = async () => {
    detecting ??= (async () => {
      delegate = (await localServiceAvailable()) ? createHttpService() : createWasmService();
    })();
    await detecting;
    return delegate!;
  };
  const forward = (name: string) => async (...args: unknown[]) => {
    const delegate = await service();
    const method = (delegate as unknown as Record<string, ((...args: unknown[]) => unknown) | undefined>)[name];
    if (!method) throw new Error(`This service does not provide ${name}.`);
    return method(...args);
  };
  const methods = [
    'capabilities', 'openExample', 'displayFor', 'validateDraft', 'openJob', 'importArtwork',
    'startPlan', 'planTask', 'cancelPlan', 'planResult', 'stockSlice',
    'startVerification', 'verificationTask', 'cancelVerification', 'verificationResult',
    'startExport', 'exportTask', 'cancelExport', 'exportResult',
  ];
  return Object.fromEntries(methods.map(name => [name, forward(name)])) as unknown as CamService;
}
