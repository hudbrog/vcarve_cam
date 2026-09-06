import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { spawn, type ChildProcess } from 'node:child_process';
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { createHttpService } from '../src/service/http';
import type { CamService } from '../src/contracts/service';
import { terminal, type TaskIdentity } from '../src/contracts/planning';
import { verificationIdentity } from '../src/contracts/verification';

const workspace = fileURLToPath(new URL('../../', import.meta.url));
const suffix = process.platform === 'win32' ? '.exe' : '';
const output = process.env.CAM_FLOWER_OUTPUT ?? `${workspace}artifacts/verification-investigation/service`;
let child: ChildProcess | undefined;
let service: CamService;

describe.runIf(process.env.CAM_FLOWER_VERIFY === '1')('flower_box verification performance', () => {
  beforeAll(async () => {
    mkdirSync(output, { recursive: true });
    const portable = process.env.CAM_TEST_EXE;
    const args = portable ? ['serve'] : ['--ui-dir', `${workspace}web/dist`];
    child = spawn(portable ?? `${workspace}target/release/cam-web${suffix}`, [...args, '--port', '0'], {
      cwd: workspace, windowsHide: true,
    });
    const base = await new Promise<string>((resolve, reject) => {
      let stdout = ''; let stderr = '';
      const timer = setTimeout(() => reject(new Error(`Server did not start: ${stderr}`)), 15_000);
      child!.on('error', reject);
      child!.stderr?.on('data', chunk => { stderr += chunk; });
      child!.stdout?.on('data', chunk => {
        stdout += chunk;
        const url = stdout.match(/CAM_WEB_URL=(http:\/\/127\.0\.0\.1:\d+)/)?.[1];
        if (url) { clearTimeout(timer); resolve(url); }
      });
    });
    service = createHttpService((url, init) => fetch(new URL(String(url), base), init));
    await service.capabilities();
  }, 20_000);
  afterAll(() => { child?.kill(); });

  it('conclusively verifies the unchanged flower job three times through the browser adapter in under 20 seconds', async () => {
    const jobPath = process.env.CAM_FLOWER_JOB ?? `${workspace}../real_data/flower_box-svg.job (2).json`;
    const jobText = readFileSync(jobPath, 'utf8');
    const opened = await service.openJob!(jobText, 71);
    const checked = await service.validateDraft(opened.job, 71);
    expect(checked.valid).toBe(true);
    const caps = await service.capabilities();
    const id: TaskIdentity = { taskId: crypto.randomUUID(), instanceId: caps.planning!.instanceId,
      engineVersion: caps.engineVersion, revision: 71, documentFingerprint: checked.documentFingerprint!, stage: 'combined' };
    let plan = await service.startPlan!(opened.job, id);
    while (!terminal(plan)) {
      await new Promise(resolve => setTimeout(resolve, 700));
      plan = await service.planTask!(id);
    }
    expect(plan.state, JSON.stringify(plan.diagnostic)).toBe('succeeded');
    expect(plan.summary?.status).toBe('complete');
    writeFileSync(`${output}/plan-summary.json`, JSON.stringify({ jobPath,
      jobSha256: createHash('sha256').update(jobText).digest('hex'),
      engineVersion: caps.engineVersion, summary: plan.summary }, null, 2));
    const measurements = [];
    for (let run = 1; run <= 3; run++) {
      const identity = verificationIdentity(plan, caps.verification!.defaultOptions, crypto.randomUUID());
      const started = performance.now();
      let task = await service.startVerification!(identity);
      while (!['succeeded', 'failed', 'cancelled'].includes(task.state)) {
        await new Promise(resolve => setTimeout(resolve, 700));
        task = await service.verificationTask!(identity);
      }
      expect(task.state, JSON.stringify(task.diagnostic)).toBe('succeeded');
      const result = await service.verificationResult!(identity);
      const seconds = (performance.now() - started) / 1000;
      const measurement = { run, seconds, status: result.report.status, options: result.report.options,
        evaluatedCells: result.report.original.evaluated_cells, unresolvedCells: result.report.original.unresolved_cells,
        maximumErrorUncertaintyMm: result.report.original.maximum_error_uncertainty_mm,
        bounds: result.report.original.bounds,
        findingCodes: [...new Set(result.report.original.findings.map(f => f.code))],
        inputFingerprint: plan.summary!.inputFingerprint, motionFingerprint: plan.summary!.motionFingerprint };
      measurements.push(measurement);
      writeFileSync(`${output}/measurements.json`, JSON.stringify(measurements, null, 2));
      writeFileSync(`${output}/report-${run}.json`, JSON.stringify(result.report, null, 2));
      console.info('Flower verification:', measurement);
      expect(result.report.original.verification_tolerance_mm).toBe(opened.job.tolerances.verification_tolerance_mm);
      expect(measurement.findingCodes).not.toContain('M5_DEPTH_BAND_LIMIT');
      expect(result.report.options).toEqual(caps.verification!.defaultOptions);
      expect(result.report.status).toBe('passed');
      expect(measurement.unresolvedCells).toBe(0);
      expect(measurement.maximumErrorUncertaintyMm).toBeLessThanOrEqual(result.report.original.verification_tolerance_mm);
      expect(seconds).toBeLessThan(20);
    }
  }, 180_000);
});
