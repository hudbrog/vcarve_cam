import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { spawn, type ChildProcess } from 'node:child_process';
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname } from 'node:path';
import { createHttpService } from '../src/service/http';
import type { CamService } from '../src/contracts/service';
import { terminal, type TaskIdentity } from '../src/contracts/planning';
import { verificationIdentity } from '../src/contracts/verification';
import { buildSimSetup } from '../src/sim/setup';
import { normalizeTool } from '../src/sim/engine';
import { buildStore } from '../src/sim/store';
import { SimulationSession } from '../src/sim/session';

// U8 M5 cross-check: the visual simulator's removed area at every published
// depth band must fall inside the exact verification engine's bounds. This
// validates the heightfield against the Rust polygon analysis through a real
// plan and verification; it does not replace verification for product claims.
const workspace = fileURLToPath(new URL('../../', import.meta.url));
const web = fileURLToPath(new URL('../', import.meta.url));
const suffix = process.platform === 'win32' ? '.exe' : '';
const portable = process.env.CAM_TEST_EXE;
const executable = portable ?? `${workspace}target/release/cam-web${suffix}`;
const output = `${web}test-results/live/simulator-crosscheck`;

let child: ChildProcess;
let base: string;
let service: CamService;

beforeAll(async () => {
  mkdirSync(output, { recursive: true });
  const args = portable ? ['serve'] : ['--ui-dir', `${web}dist`];
  child = spawn(executable, [...args, '--port', '0'], {
    cwd: portable ? dirname(portable) : workspace,
    windowsHide: true,
  });
  base = await new Promise<string>((resolve, reject) => {
    let stdout = '';
    let stderr = '';
    const timer = setTimeout(() => reject(new Error(`Server did not start: ${stderr}`)), 15_000);
    child.on('error', error => { clearTimeout(timer); reject(error); });
    child.once('exit', code => { clearTimeout(timer); reject(new Error(`Server exited ${code}: ${stderr}`)); });
    child.stderr?.on('data', chunk => { stderr += chunk; });
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

async function poll<T>(read: () => Promise<T>, done: (value: T) => boolean, timeoutMs: number): Promise<T> {
  const started = Date.now();
  for (;;) {
    const value = await read();
    if (done(value)) return value;
    if (Date.now() - started > timeoutMs) throw new Error('Task did not finish in time');
    await new Promise(resolve => setTimeout(resolve, 700));
  }
}

describe('simulator against exact verification bounds', () => {
  it('keeps every depth band inside the verification engine intervals', { timeout: 300_000 }, async () => {
    const jobText = readFileSync(`${workspace}fixtures/m4/curved-medial.json`, 'utf8');
    const opened = await service.openJob!(jobText, 71);
    const checked = await service.validateDraft(opened.job, 71);
    expect(checked.valid, JSON.stringify(checked.diagnostics)).toBe(true);
    const caps = await service.capabilities();
    const identity: TaskIdentity = {
      taskId: crypto.randomUUID(),
      instanceId: caps.planning!.instanceId,
      engineVersion: caps.engineVersion,
      revision: 71,
      documentFingerprint: checked.documentFingerprint!,
      stage: 'combined',
    };
    let plan = await service.startPlan!(opened.job, identity);
    plan = await poll(() => service.planTask!(identity), terminal, 120_000);
    expect(plan.state, JSON.stringify(plan.diagnostic)).toBe('succeeded');
    expect(plan.summary?.status).toBe('complete');

    const planResult = await service.planResult!(identity);
    const verification = verificationIdentity(plan, caps.verification!.defaultOptions, crypto.randomUUID());
    let verificationTask = await service.startVerification!(verification);
    verificationTask = await poll(
      () => service.verificationTask!(verification),
      task => ['succeeded', 'failed', 'cancelled'].includes(task.state),
      180_000,
    );
    expect(verificationTask.state, JSON.stringify(verificationTask.diagnostic)).toBe('succeeded');
    const report = (await service.verificationResult!(verification)).report.original;

    const setup = buildSimSetup(opened.job, planResult.motions, planResult.stockSlices);
    const store = buildStore(planResult.motions, { toolIndex: setup.toolIndex, assumedFeedMmMin: 1000 });
    const session = new SimulationSession(setup.stock, setup.tools.map(normalizeTool), setup.resolution, store);
    const finished = session.seekToEnd();
    expect(finished.stats.dirtyCells).toBeGreaterThan(0);

    const field = session.field;
    const sliceArea = (depth: number): number => {
      const threshold = Math.ceil(depth / field.quantumMm);
      let cells = 0;
      for (const heights of field.heights) {
        if (heights === undefined) continue;
        for (let index = 0; index < heights.length; index++) {
          if (heights[index] >= threshold) cells++;
        }
      }
      return cells * field.cellAreaMm2;
    };

    const results = report.depth_bands.map(band => {
      const depth = (band.from_depth_mm + band.to_depth_mm) / 2;
      const sim = sliceArea(depth);
      const lower = band.removed_area_mm2.lower;
      const upper = band.removed_area_mm2.upper;
      // Grid quantization error concentrates on region boundaries; allow a
      // generous perimeter estimate of 4*sqrt(area) cell widths, doubled.
      const slack = 8 * Math.sqrt(Math.max(upper, 1)) * setup.resolution.cellMm + 0.5;
      return { depth, sim, lower, upper, slack, ok: sim >= lower - slack && sim <= upper + slack };
    });
    writeFileSync(`${output}/depth-bands.json`, JSON.stringify({
      engine: caps.engineVersion,
      verificationStatus: report.status,
      cellMm: setup.resolution.cellMm,
      motionCount: planResult.motions.length,
      simRemovedVolumeMm3: finished.stats.removedVolumeMm3,
      residualVolumeMm3: report.bounds.residual_volume_mm3,
      overcutVolumeMm3: report.bounds.overcut_volume_mm3,
      results,
    }, null, 2));

    expect(report.depth_bands.length).toBeGreaterThan(2);
    for (const entry of results) {
      expect(entry.ok,
        `depth ${entry.depth.toFixed(2)} mm: simulator ${entry.sim.toFixed(1)} mm² outside [${entry.lower.toFixed(1)}, ${entry.upper.toFixed(1)}] ± ${entry.slack.toFixed(1)}`).toBe(true);
    }
  });
});
