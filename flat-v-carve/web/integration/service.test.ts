import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { createHttpService } from '../src/service/http';
import type { CamService } from '../src/contracts/service';
import { terminal, type PlanTask, type TaskIdentity } from '../src/contracts/planning';

// Runs the actual Rust server and the same-engine CLI; no fixture HTTP responses.
const workspace = fileURLToPath(new URL('../../', import.meta.url));
const web = fileURLToPath(new URL('../', import.meta.url));
const suffix = process.platform === 'win32' ? '.exe' : '';
const executable = `${workspace}target/release/cam-web${suffix}`;
const cli = `${workspace}target/release/cam${suffix}`;
const output = `${web}test-results/live`;
let child: ChildProcess;
let base: string;
let service: CamService;
function runCli(args: string[], expectedStatus = 0) {
  const result = spawnSync(cli, args, { cwd: workspace, encoding: 'utf8', maxBuffer: 16_000_000, windowsHide: true });
  if (result.error) throw result.error;
  expect(result.status, result.stderr).toBe(expectedStatus);
  return result.stdout;
}
beforeAll(async () => {
  mkdirSync(output, { recursive: true });
  child = spawn(executable, ['--port', '0', '--ui-dir', `${web}dist`], { cwd: workspace, windowsHide: true });
  base = await new Promise<string>((resolve, reject) => {
    let stdout = ''; let stderr = '';
    const timer = setTimeout(() => reject(new Error(`Server did not start: ${stderr}`)), 15_000);
    child.on('error', error => { clearTimeout(timer); reject(error); });
    child.stderr?.on('data', chunk => { stderr += chunk; });
    child.once('exit', code => { clearTimeout(timer); reject(new Error(`Server exited ${code}: ${stderr}`)); });
    child.stdout?.on('data', chunk => {
      stdout += chunk;
      const url = stdout.match(/CAM_WEB_URL=(http:\/\/127\.0\.0\.1:\d+)/)?.[1];
      if (url) { clearTimeout(timer); resolve(url); }
    });
  });
  service = createHttpService((url, init) => fetch(new URL(String(url), base), init));
  await service.capabilities();
}, 20_000);
afterAll(() => { child?.kill(); });

describe('live Rust/UI contract and CLI parity', () => {
  it('serves the offline production build with the API on the same origin', async () => {
    const response = await fetch(base);
    expect(response.status).toBe(200);
    expect(response.headers.get('content-security-policy')).toContain("connect-src 'self'");
    const html = await response.text();
    const script = html.match(/src="([^"]+\.js)"/)?.[1];
    expect(script).toBeTruthy();
    const asset = await fetch(new URL(script!, base));
    expect(asset.headers.get('content-type')).toContain('javascript');
    expect(asset.status).toBe(200);
    expect((await fetch(new URL('/Cargo.toml', base))).status).toBe(404);
  });
  it('imports Inkscape with exactly the CLI job, normalized rings, holes, and missing settings', async () => {
    const source = `${workspace}fixtures/m2/inkscape-export.svg`;
    const jsonPath = `${output}/cli-import.job.json`;
    runCli(['import', source, '--output', jsonPath]);
    const cliJob = JSON.parse(readFileSync(jsonPath, 'utf8'));
    const imported = await service.importArtwork!('inkscape-export.svg', readFileSync(source, 'utf8'), cliJob.import, 23);
    expect(imported.job).toEqual(cliJob);
    const inspection = JSON.parse(runCli(['validate-job', jsonPath])).inspection;
    expect(imported.display.engineVersion).toBe(inspection.engine_version);
    expect(imported.display.components).toEqual(inspection.geometry.sources.map((source: { id: string; source_id: string; label: string | null; geometry: { grid: { ticks_per_mm: number }; rings: { is_hole: boolean; points: { x: number; y: number }[] }[] } }) => ({
      id: source.id, label: source.label || source.source_id,
      rings: source.geometry.rings.map(ring => ({ hole: ring.is_hole, points: ring.points.map(point => ({ x: point.x / source.geometry.grid.ticks_per_mm, y: point.y / source.geometry.grid.ticks_per_mm })) })),
    })));
    expect(imported.missingMachiningFields).toEqual(inspection.missing_machining_fields);
    expect((await service.validateDraft(imported.job, 23)).documentFingerprint).toBe(imported.documentFingerprint);
  }, 15_000);
  it('opens configured jobs, migrates old schemas, and rejects future/invalid jobs', async () => {
    const original = readFileSync(`${workspace}fixtures/m4/finite-tip.json`, 'utf8');
    const opened = await service.openJob!(original, 4);
    expect(opened.job).toEqual(JSON.parse(original));
    expect(opened.missingMachiningFields).toEqual([]);
    const old = { ...opened.job, schema_version: 1 };
    expect((await service.openJob!(JSON.stringify(old), 5)).job).toEqual(opened.job);
    await expect(service.openJob!(JSON.stringify({ ...old, schema_version: 99 }), 6)).rejects.toThrow(/JOB_SCHEMA_VERSION/);
    opened.job.stock.thickness_mm = -1;
    const invalid = await service.validateDraft(opened.job, 7);
    expect(invalid.valid).toBe(false);
    expect(invalid.diagnostics[0].code).toBe('JOB_PARAMETER');
    await expect(service.openJob!(JSON.stringify(opened.job), 7)).rejects.toThrow(/JOB_PARAMETER/);
  });
  it('normalizes changed artwork placement and roundtrips the browser adapter snapshot through Rust', async () => {
    const options = { geometry_tolerance_mm: 0.001, ticks_per_mm: null, placement: { origin_mm: { x: 2, y: 3 }, scale: 1.4, rotation_deg: 27 } };
    const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="30mm" height="20mm" viewBox="0 0 30 20"><rect id="plate" x="5" y="5" width="15" height="10"/></svg>';
    const imported = await service.importArtwork!('integration-plate.svg', svg, options, 8);
    expect(await service.displayFor(imported.job)).toEqual(imported.display);
    writeFileSync(`${output}/adapter.job.json`, JSON.stringify(imported.job));
    const inspection = JSON.parse(runCli(['validate-job', `${output}/adapter.job.json`])).inspection;
    expect(inspection.geometry.sources.map((source: { id: string }) => source.id)).toEqual(imported.display.components.map(component => component.id));
    const checked = await service.validateDraft(imported.job, 8);
    expect(checked.valid).toBe(true);
    expect(checked.missingMachiningFields).toEqual(inspection.missing_machining_fields);
    imported.job.name = 'changed';
    expect((await service.validateDraft(imported.job, 9)).documentFingerprint).not.toBe(checked.documentFingerprint);
  });
});

async function identity(fixture: string, stage: TaskIdentity['stage'] = 'endmill') {
  const opened = await service.openJob!(readFileSync(`${workspace}fixtures/${fixture}.json`, 'utf8'), 14);
  const caps = await service.capabilities();
  const id: TaskIdentity = { taskId: crypto.randomUUID(), instanceId: caps.planning!.instanceId,
    engineVersion: caps.engineVersion, revision: 14, documentFingerprint: opened.documentFingerprint, stage };
  return { job: opened.job, id };
}
async function finish(id: TaskIdentity) {
  const deadline = Date.now() + 30_000;
  let previous: PlanTask | null = null;
  while (Date.now() < deadline) {
    const task = await service.planTask!(id);
    if (previous) expect(task.sequence).toBeGreaterThanOrEqual(previous.sequence);
    if (terminal(task)) return task;
    previous = task;
    await new Promise(resolve => setTimeout(resolve, 20));
  }
  throw new Error('Planning did not finish within the test deadline');
}
describe('real background planning', () => {
  it.each([
    ['m3/rectangle', 'endmill', 'complete'], ['m3/no-access', 'endmill', 'empty'],
    ['m3/unsupported-entry', 'endmill', 'incomplete'], ['m3/resource-limit', 'endmill', 'inconclusive'],
    ['m4/finite-tip', 'combined', 'complete'],
  ] as const)('matches CLI artifact and recorded motions for %s', async (fixture, stage, status) => {
    const { job, id } = await identity(fixture, stage);
    const accepted = await service.startPlan!(job, id);
    expect(accepted.state).toBe('queued');
    const task = await finish(id);
    expect(task.state, JSON.stringify(task.diagnostic)).toBe('succeeded');
    expect(task.summary?.status).toBe(status);
    const result = await service.planResult!(id);
    const path = `${output}/${fixture.replace('/', '-')}-${stage}.plan.json`;
    runCli(['plan', `${workspace}fixtures/${fixture}.json`, '--stage', stage, '--output', path], ['complete', 'empty'].includes(status) ? 0 : 1);
    const cliPlan = JSON.parse(readFileSync(path, 'utf8'));
    expect(task.summary?.inputFingerprint).toBe(cliPlan.input_fingerprint);
    expect(task.summary?.motionFingerprint).toBe(cliPlan.motion_fingerprint);
    const motions = stage === 'combined' ? [...cliPlan.endmill.motions, ...cliPlan.vbit_motions] : cliPlan.motions;
    expect(result.motions).toEqual(motions.slice(0, 20_000));
    expect(task.summary?.motionCount).toBe(motions.length);
    const session = await (await fetch(`${base}/api/v1/session`)).json();
    const artifact = await fetch(`${base}/api/v1/tasks/${id.taskId}/artifact`, { headers: { 'X-Cam-Session': session.sessionToken } });
    expect(await artifact.json()).toEqual(cliPlan);
    // The CLI rebuilds analysis independently from the saved artifact. Display
    // rings must be exactly that engine geometry, including holes and placement.
    const inspectPlan = (artifactPath: string, suffix: string, expectedStatus: number) => {
      const report = `${path}.${suffix}.report.json`;
      runCli(['inspect', artifactPath, '--output', `${path}.${suffix}.svg`, '--report', report], expectedStatus);
      return JSON.parse(readFileSync(report, 'utf8')).analysis;
    };
    const analysis = inspectPlan(path, 'analysis', ['complete', 'empty'].includes(status) ? 0 : 1);
    let endmillAnalysis = analysis;
    if (stage === 'combined') {
      const endmillPath = `${path}.endmill.json`;
      writeFileSync(endmillPath, JSON.stringify(cliPlan.endmill));
      endmillAnalysis = inspectPlan(endmillPath, 'endmill', 0);
    }
    const regionKeys = { nominalTarget: 'nominal_section', remainingTarget: 'remaining_target', possibleOvercut: 'possible_overcut',
      accessibleFloor: 'accessible_floor', missingFloor: 'missing_floor_beyond_tolerance', requestedCenters: 'requested_centers' };
    expect(result.stockSlices.length).toBe(endmillAnalysis.layers.length + (stage === 'combined' ? analysis.slices.length : 0));
    for (const info of result.stockSlices) {
      const index = Number(info.id.split('-')[1]);
      const core = info.stage === 'endmill' ? endmillAnalysis.layers[index] : analysis.slices[index];
      const display = await service.stockSlice!(id, info);
      expect(display.slice.info).toEqual(info);
      expect(info.depthMm).toBe(core.depth_mm);
      expect(info.capsuleRadialErrorMm).toBe(core.removal.capsule_radial_error_mm);
      expect(info.contributingMotionCount).toBe(core.removal.contributing_motion_ids.length);
      expect(display.slice.geometry).not.toBeNull();
      for (const region of display.slice.geometry!) {
        const actual = region.key === 'removedLower' ? core.removal.lower : region.key === 'removedUpper' ? core.removal.upper : core[regionKeys[region.key]];
        expect(region.rings).toEqual(actual.rings.map((ring: { is_hole: boolean; points: { x: number; y: number }[] }) => ({
          hole: ring.is_hole, points: ring.points.map(p => ({ x: p.x / actual.grid.ticks_per_mm, y: p.y / actual.grid.ticks_per_mm })),
        })));
      }
    }
    await expect(service.stockSlice!(id, { ...result.stockSlices[0], id: 'endmill-999' })).rejects.toThrow(/SLICE_NOT_FOUND/);
    expect((await service.cancelPlan!(id)).state).toBe('succeeded');
    expect((await service.startPlan!(job, id)).taskId).toBe(id.taskId); // Retry does not calculate again.
    await expect(service.startPlan!({ ...job, name: 'edited' }, id)).rejects.toThrow(/STALE_DOCUMENT/);
    await expect(service.startPlan!(job, { ...id, revision: id.revision + 1 })).rejects.toThrow(/TASK_KEY_REUSED/);
  }, 45_000);
  it('checks the chosen stage in Rust and preserves its setup diagnostic', async () => {
    const { job, id } = await identity('m3/rectangle', 'combined');
    await service.startPlan!(job, id);
    const task = await finish(id);
    expect(task.state).toBe('failed');
    expect(task.diagnostic?.code).toBeTruthy();
    expect(task.summary).toBeNull();
    await expect(service.planResult!(id)).rejects.toThrow(/PLAN_RESULT_UNAVAILABLE/);
  });
  it('keeps validation responsive, reconnects without replay, and cancels running work', async () => {
    const { job, id } = await identity('m4/finite-tip', 'combined');
    // The 0.7.2 spatial index finishes small fixtures before a browser roundtrip.
    // Use a real, bounded sampling workload to exercise active cancellation.
    job.vbit_planning!.quality_sample_spacing_mm = 0.12;
    job.vbit_planning!.max_quality_samples = 50_000;
    id.documentFingerprint = (await service.validateDraft(job, id.revision)).documentFingerprint!;
    await service.startPlan!(job, id);
    let task = await service.planTask!(id);
    while (task.state === 'queued') task = await service.planTask!(id);
    expect(task.state).toBe('running');
    expect((await service.validateDraft(job, 15)).valid).toBe(true);
    await service.capabilities();
    const cancelled = await service.cancelPlan!(id);
    expect(cancelled.state).toBe('cancelling');
    expect((await finish(id)).state).toBe('cancelled');
    expect((await service.cancelPlan!(id)).state).toBe('cancelled');
    await expect(service.planResult!(id)).rejects.toThrow(/PLAN_RESULT_UNAVAILABLE/);
    await expect(service.planTask!({ ...id, instanceId: '0'.repeat(32) })).rejects.toThrow(/previous service instance/);
  }, 15_000);
});
