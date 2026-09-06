import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { spawn, type ChildProcess } from 'node:child_process';
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { createHttpService } from '../src/service/http';
import type { CamService } from '../src/contracts/service';
import { terminal, type TaskIdentity } from '../src/contracts/planning';
import { exportIdentity, profileSchema } from '../src/contracts/export';

const workspace = fileURLToPath(new URL('../../', import.meta.url));
const output = process.env.CAM_FLOWER_OUTPUT ?? `${workspace}artifacts/postprocessing-flower/service`;
let child: ChildProcess | undefined;
let service: CamService;

describe.runIf(process.env.CAM_FLOWER_EXPORT === '1')('flower_box G-code generation', () => {
  beforeAll(async () => {
    mkdirSync(output, { recursive: true });
    const portable = process.env.CAM_TEST_EXE;
    child = spawn(portable ?? process.env.CAM_FLOWER_SERVER ?? `${workspace}target/release/cam-web.exe`,
      [...(portable ? ['serve'] : ['--ui-dir', `${workspace}web/dist`]), '--port', '0', '--library-dir', `${output}/library`],
      { cwd: workspace, windowsHide: true });
    const base = await new Promise<string>((resolve, reject) => {
      let stdout = ''; let stderr = '';
      const timer = setTimeout(() => reject(new Error(`Server did not start: ${stderr}`)), 15_000);
      child!.on('error', reject);
      child!.stderr?.on('data', chunk => { stderr += chunk; writeFileSync(`${output}/server.timings.txt`, stderr); });
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

  it('exports every saved flower motion with the three-decimal machine profile within the performance budget', async () => {
    const jobPath = process.env.CAM_FLOWER_JOB ?? `${workspace}../real_data/flower_box-svg.job-real.json`;
    const jobText = readFileSync(jobPath, 'utf8');
    const opened = await service.openJob!(jobText, 81);
    expect(opened.job.source.svg.trim()).toBe(readFileSync(`${workspace}../real_data/flower_box.svg`, 'utf8').trim());
    const caps = await service.capabilities();
    const id: TaskIdentity = { taskId: crypto.randomUUID(), instanceId: caps.planning!.instanceId,
      engineVersion: caps.engineVersion, revision: 81, documentFingerprint: opened.documentFingerprint, stage: 'combined' };
    let plan = await service.startPlan!(opened.job, id);
    while (!terminal(plan)) {
      await new Promise(resolve => setTimeout(resolve, 100));
      plan = await service.planTask!(id);
    }
    expect(plan.state, JSON.stringify(plan.diagnostic)).toBe('succeeded');
    expect(plan.summary?.status).toBe('complete');
    const profilePath = process.env.CAM_FLOWER_PROFILE ?? `${workspace}../real_data/machine-profile.json`;
    const profile = profileSchema.parse(JSON.parse(readFileSync(profilePath, 'utf8')));
    expect(profile.decimal_places).toBe(3);
    const measurements = [];
    for (const layout of ['combined', 'combined', 'per_tool'] as const) {
      const identity = exportIdentity(plan, profile, layout, caps.verification!.defaultOptions, crypto.randomUUID());
      const started = performance.now();
      let task = await service.startExport!(identity);
      while (!['succeeded', 'failed', 'cancelled'].includes(task.state)) {
        await new Promise(resolve => setTimeout(resolve, 200));
        task = await service.exportTask!(identity);
      }
      expect(task.state, JSON.stringify(task.diagnostic)).toBe('succeeded');
      const result = await service.exportResult!(identity);
      const seconds = (performance.now() - started) / 1000;
      const run: number = measurements.length + 1;
      const measurement = { run, layout, seconds, status: result.report.status,
        outputDecimalPlaces: result.report.output_decimal_places, motionCount: plan.summary!.motionCount,
        checkedMotions: result.report.emitted_verification?.checked_motion_count,
        originalCells: result.report.plan_verification.original.evaluated_cells,
        emittedCells: result.report.emitted_verification?.evaluated_cells,
        diagnostics: result.report.diagnostics };
      measurements.push(measurement);
      writeFileSync(`${output}/measurements.json`, JSON.stringify({ jobPath, profilePath,
        jobSha256: createHash('sha256').update(jobText).digest('hex'), engineVersion: caps.engineVersion, measurements }, null, 2));
      writeFileSync(`${output}/report-${run}.json`, result.reportJson);
      for (const program of result.programs) writeFileSync(`${output}/${run}-${program.filename}`, program.gcode);
      console.info('Flower export:', measurement);
      expect(result.report.status).toBe('passed');
      expect(result.report.emitted_verification?.unresolved_cells).toBe(0);
      expect(measurement.checkedMotions).toBe(plan.summary!.motionCount);
      expect(result.report.programs.reduce((sum, p) => sum + p.motion_count, 0)).toBe(plan.summary!.motionCount);
      expect(result.programs.length).toBe(layout === 'combined' ? 1 : 2);
      expect(seconds).toBeLessThan(Number(process.env.CAM_FLOWER_EXPORT_SECONDS ?? 5));
    }
  }, 240_000);
});
