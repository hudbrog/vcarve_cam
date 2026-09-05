import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { acceptTask, currentPlan, planResultSchema, taskSchema, type Motion, type PlanTask, type TaskIdentity } from '../src/contracts/planning';
import type { Capabilities, Validation } from '../src/contracts/service';
import { apiVersion } from '../src/contracts/wire';
import { createHttpService } from '../src/service/http';
import { fixtureService } from '../src/service/fixture';
import { Viewport, motionPath } from '../src/components/Viewport';

const identity: TaskIdentity = { taskId: 'test-plan', instanceId: 'a'.repeat(32), engineVersion: '0.6.0', revision: 7, documentFingerprint: 'b'.repeat(64), stage: 'combined' };
const summary = { engineVersion: '0.6.0', status: 'incomplete' as const, inputFingerprint: 'c'.repeat(64), motionFingerprint: 'd'.repeat(64),
  meaning: 'Continuous clearance and sampled checks only', limitations: ['No global stock bounds'], motionCount: 1, cuttingMotionCount: 1,
  previewMotionCount: 1, omittedMotionCount: 0, diagnostics: [], omittedDiagnostics: 0, generationIssues: [], omittedGenerationIssues: 0 };
const finished: PlanTask = { ...identity, apiVersion, sequence: 5, state: 'succeeded', summary, diagnostic: null, resultAvailable: true };
const caps: Capabilities = { apiVersion, mode: 'live', engineVersion: '0.6.0', importArtwork: true, openJob: true, validateDraft: true,
  planningStages: ['endmill', 'combined'], verificationScopes: [], exportFormats: [],
  limits: { svgBytes: 2_000_000, jobBytes: 8_000_000, requestBytes: 16_100_000, concurrentInspections: 2 },
  planning: { instanceId: identity.instanceId, concurrentPlans: 1, maxPending: 4, maxTasks: 128, retainedResults: 4, timeoutSeconds: 300, previewMotions: 20_000, artifactBytes: 16_000_000 } };
const validation: Validation = { authoritative: true, valid: true, revision: 7, diagnostics: [], scope: 'editable-job-and-svg', documentFingerprint: identity.documentFingerprint };
const motion: Motion = { id: 0, tool_id: 'endmill', operation_id: 'carve', layer: 0, kind: 'cut', start: { x: 2, y: 3, z: -1 }, end: { x: 4, y: 5, z: -2 }, feed_mm_min: 100 };

describe('task identity and evidence', () => {
  it('keeps task success separate from the engine outcome', () => {
    for (const status of ['complete', 'empty', 'incomplete', 'inconclusive']) expect(taskSchema.parse({ ...finished, summary: { ...summary, status } }).state).toBe('succeeded');
    expect(taskSchema.safeParse({ ...finished, state: 'cancelled' }).success).toBe(false);
    expect(taskSchema.safeParse({ ...finished, summary: { ...summary, engineVersion: 'another' } }).success).toBe(false);
    expect(planResultSchema.safeParse({ task: finished, coordinateSpace: 'workpiece-mm-z-up', motions: [] }).success).toBe(false);
    expect(planResultSchema.parse({ task: finished, coordinateSpace: 'workpiece-mm-z-up', motions: [motion] }).motions).toEqual([motion]);
  });
  it('invalidates motions immediately for edits, invalid text, pending checks, stage changes, and restarts', () => {
    expect(currentPlan(finished, validation, 7, 'combined', caps)).toBe(true);
    expect(currentPlan(finished, validation, 8, 'combined', caps)).toBe(false);
    expect(currentPlan(finished, undefined, 7, 'combined', caps)).toBe(false);
    expect(currentPlan(finished, { ...validation, valid: false }, 7, 'combined', caps)).toBe(false);
    expect(currentPlan(finished, { ...validation, authoritative: false }, 7, 'combined', caps)).toBe(false);
    expect(currentPlan(finished, { ...validation, documentFingerprint: 'e'.repeat(64) }, 7, 'combined', caps)).toBe(false);
    expect(currentPlan(finished, validation, 7, 'endmill', caps)).toBe(false);
    expect(currentPlan(finished, validation, 7, 'combined', { ...caps, engineVersion: 'new' })).toBe(false);
    expect(currentPlan(finished, validation, 7, 'combined', { ...caps, planning: { ...caps.planning!, instanceId: 'f'.repeat(32) } })).toBe(false);
  });
  it('discards reordered updates and rejects mixed identities or terminal regressions', () => {
    const running: PlanTask = { ...finished, state: 'running', sequence: 2, summary: null, resultAvailable: false };
    expect(acceptTask(finished, running, identity)).toBe(finished);
    expect(acceptTask(finished, { ...finished }, identity)).toBe(finished);
    expect(() => acceptTask(finished, { ...running, sequence: 6 }, identity)).toThrow(/finished task/);
    for (const field of ['taskId', 'instanceId', 'engineVersion', 'documentFingerprint', 'stage'] as const)
      expect(() => acceptTask(null, finished, { ...identity, [field]: 'different' })).toThrow(/different task identity/);
    expect(() => acceptTask(null, finished, { ...identity, revision: 8 })).toThrow(/different task identity/);
  });
  it('projects recorded coordinates without applying artwork placement a second time', async () => {
    const { job, display } = await fixtureService.openExample();
    job.import.placement = { origin_mm: { x: 50, y: 90 }, scale: 3, rotation_deg: 45 };
    const path = motionPath([motion]);
    expect(path).toBe('M2,-3L4,-5');
    const markup = renderToStaticMarkup(<Viewport display={display} job={job} inspected={null} onInspect={() => {}} hidden={new Set()} motions={[motion]} />);
    expect(markup).toContain(`d="${path}"`);
    expect(markup).toContain('RECORDED MOTIONS');
    expect(markup).toContain('data-motion-group="endmill:cutting"');
  });
});

describe('planning HTTP adapter', () => {
  it('retries only an explicit immutable key and refuses a different service instance before sending', async () => {
    const calls: { url: string; init?: RequestInit }[] = [];
    let wrongIdentity = false;
    const service = createHttpService(async (input, init) => {
      const url = String(input); calls.push({ url, init });
      if (url.endsWith('session')) return Response.json({ apiVersion, engineVersion: '0.6.0', sessionToken: 'e'.repeat(64) });
      if (url.endsWith('capabilities')) return Response.json(caps);
      return Response.json({ ...finished, taskId: wrongIdentity ? 'wrong' : identity.taskId });
    });
    await service.capabilities();
    const { job } = await fixtureService.openExample();
    await service.startPlan!(job, identity);
    await service.startPlan!(job, identity);
    const requests = calls.filter(call => call.url.endsWith('/tasks'));
    expect(requests).toHaveLength(2);
    expect(requests[0].init?.body).toBe(requests[1].init?.body);
    expect(JSON.parse(String(requests[0].init?.body)).requestId).toBe(identity.taskId);
    const count = calls.length;
    await expect(service.startPlan!(job, { ...identity, instanceId: 'f'.repeat(32) })).rejects.toThrow(/previous service instance/);
    expect(calls.length).toBe(count);
    wrongIdentity = true;
    await expect(service.planTask!(identity)).rejects.toThrow(/different task identity/);
  });
});
