import { describe, expect, it } from 'vitest';
import { createHttpService } from '../src/service/http';
import { fixtureService } from '../src/service/fixture';
import { editableDownloadAllowed, outputBlockedReasons, type Validation } from '../src/contracts/service';
import { apiVersion } from '../src/contracts/wire';

const caps = { apiVersion, mode: 'live', engineVersion: '0.6.0', importArtwork: true, openJob: true, validateDraft: true,
  planningStages: [], verificationScopes: [], exportFormats: [], limits: { svgBytes: 2_000_000, jobBytes: 8_000_000, requestBytes: 16_100_000, concurrentInspections: 2 } };
const good = { valid: true, authoritative: true, scope: 'editable-job-and-svg', documentFingerprint: 'b'.repeat(64), diagnostics: [], missingMachiningFields: ['stock.thickness_mm'] };
function harness(change: (reply: Record<string, unknown>) => void = () => {}, status = 200) {
  const calls: { url: string; init: RequestInit }[] = [];
  const service = createHttpService(async (input, init) => {
    const url = String(input); calls.push({ url, init: init! });
    if (url.endsWith('session')) return Response.json({ apiVersion, engineVersion: '0.6.0', sessionToken: 'a'.repeat(64) });
    if (url.endsWith('capabilities')) return Response.json(caps);
    const request = JSON.parse(String(init?.body));
    const reply = { apiVersion, engineVersion: '0.6.0', requestId: request.requestId, revision: request.revision, data: structuredClone(good) };
    change(reply);
    return Response.json(reply, { status });
  });
  return { service, calls };
}
describe('checked same-origin Rust adapter', () => {
  it('downloads only a current authoritative editable-job receipt', () => {
    const result: Validation = { ...good, scope: 'editable-job-and-svg', revision: 5 };
    expect(editableDownloadAllowed(result, 5)).toBe(true);
    expect(editableDownloadAllowed(result, 6)).toBe(false);
    expect(editableDownloadAllowed(undefined, 5)).toBe(false);
    expect(editableDownloadAllowed({ ...result, authoritative: false }, 5)).toBe(false);
    expect(editableDownloadAllowed({ ...result, valid: false }, 5)).toBe(false);
    expect(editableDownloadAllowed({ ...result, documentFingerprint: null }, 5)).toBe(false);
  });
  it('uses an in-memory session and returns only scoped validation for the accepted revision', async () => {
    const { service, calls } = harness();
    expect(outputBlockedReasons(await service.capabilities())).toHaveLength(3);
    const { job } = await fixtureService.openExample();
    const result = await service.validateDraft(job, 19);
    expect(result).toEqual({ ...good, revision: 19 });
    expect(calls.map(call => call.url)).toEqual(['/api/v1/session', '/api/v1/capabilities', '/api/v1/document']);
    const init = calls.at(-1)!.init;
    expect(init.credentials).toBe('omit');
    expect(init.cache).toBe('no-store');
    expect(new Headers(init.headers).get('x-cam-session')).toBe('a'.repeat(64));
    expect(JSON.parse(String(init.body)).command.job).toEqual(job);
    expect(String(init.body)).not.toContain('sessionToken');
  });
  it.each(['requestId', 'revision', 'engineVersion', 'apiVersion'] as const)('rejects mismatched %s', async field => {
    const { service } = harness(reply => { reply[field] = field === 'revision' ? 77 : 'different'; });
    await service.capabilities();
    await expect(service.validateDraft((await fixtureService.openExample()).job, 1)).rejects.toThrow(/identity|incompatible/);
  });
  it('rejects inconsistent validation success and malformed display coordinates', async () => {
    const { service } = harness(reply => { reply.data = { ...good, documentFingerprint: null }; });
    await service.capabilities();
    const { job } = await fixtureService.openExample();
    await expect(service.validateDraft(job, 0)).rejects.toThrow(/incompatible/);
    await expect(service.displayFor(job)).rejects.toThrow(/incompatible/);
  });
  it('retains engine diagnostic code and source association on rejected import', async () => {
    const { service } = harness(reply => { delete reply.data; reply.diagnostic = { code: 'SVG_TEXT', severity: 'error', stage: 'svg', sourceId: 'letter', message: 'Convert text to paths.' }; }, 422);
    await service.capabilities();
    const { job } = await fixtureService.openExample();
    await expect(service.importArtwork!('text.svg', '<svg/>', job.import, 0)).rejects.toThrow('SVG_TEXT: Convert text to paths. (source: letter)');
  });
  it('requires explicit reconnect after a restarted session without replaying operations', async () => {
    const { service, calls } = harness(() => {}, 401);
    await service.capabilities();
    const { job } = await fixtureService.openExample();
    await expect(service.validateDraft(job, 0)).rejects.toThrow(/restarted/);
    await expect(service.validateDraft(job, 1)).rejects.toThrow(/Reconnect/);
    expect(calls.filter(call => call.url.endsWith('document'))).toHaveLength(1);
  });
  it('propagates aborts even when transport ignores cancellation', async () => {
    const { service } = harness();
    await service.capabilities();
    const controller = new AbortController(); controller.abort();
    await expect(service.validateDraft((await fixtureService.openExample()).job, 0, controller.signal)).rejects.toThrow();
  });
  it('keeps invalid user settings as an authoritative rejection, not a transport failure', async () => {
    const diagnostic = { code: 'JOB_PARAMETER', severity: 'error', stage: 'job', message: 'stock.thickness_mm must be positive' };
    const { service } = harness(reply => { reply.data = { ...good, valid: false, documentFingerprint: null, diagnostics: [diagnostic], missingMachiningFields: [] }; });
    await service.capabilities();
    const { job } = await fixtureService.openExample(); job.stock.thickness_mm = -2;
    expect((await service.validateDraft(job, 7)).diagnostics).toEqual([diagnostic]);
    expect(job.stock.thickness_mm).toBe(-2);
  });
});
