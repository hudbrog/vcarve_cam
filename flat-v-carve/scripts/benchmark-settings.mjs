// Sequential, isolated CLI runs. Creates variants without changing the source job.
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { statistics } from './analyze-motions.mjs';

const [jobArg, camArg, outArg, ...requested] = process.argv.slice(2);
if (!outArg) throw new Error('usage: node scripts/benchmark-settings.mjs <job.json> <cam.exe> <new-output-dir> [variant-id ...]');
const source = JSON.parse(fs.readFileSync(jobArg, 'utf8'));
const vbitIndex = source.tools.findIndex(tool => tool.id === source.operation.vbit_id);
if (vbitIndex < 0) throw new Error('The job must contain its selected V-bit');
const out = path.resolve(outArg);
fs.mkdirSync(out); // Refuse to replace an earlier experiment.
fs.mkdirSync(path.join(out, 'jobs'));
const variants = [
  ['baseline-1', {}],
  ['motion-005', { 'tolerances.motion_tolerance_mm': 0.05 }],
  ['motion-010', { 'tolerances.motion_tolerance_mm': 0.1 }],
  ['geometry-00025', { 'import.geometry_tolerance_mm': 0.0025 }],
  ['geometry-0005', { 'import.geometry_tolerance_mm': 0.005 }],
  ['verification-010', { 'tolerances.verification_tolerance_mm': 0.1 }],
  ['geometry-001', { 'import.geometry_tolerance_mm': 0.01, 'tolerances.motion_tolerance_mm': 0.05, 'tolerances.verification_tolerance_mm': 0.1 }],
  ['ridge-005', { 'operation.max_floor_ridge_mm': 0.05 }],
  ['ridge-020', { 'operation.max_floor_ridge_mm': 0.2 }],
  ['wall-010', { 'operation.wall_allowance_mm': 0.1 }],
  ['wall-030', { 'operation.wall_allowance_mm': 0.3 }],
  ['wall-030-geometry-0005', { 'operation.wall_allowance_mm': 0.3, 'import.geometry_tolerance_mm': 0.005, 'tolerances.motion_tolerance_mm': 0.05 }],
  ['detail-001', { 'operation.max_detail_residual_mm': 0.01 }],
  ['detail-020', { 'operation.max_detail_residual_mm': 0.2 }],
  ['vbit-step-005', { [`tools.${vbitIndex}.stepover_mm`]: 0.05 }],
  ['vbit-step-020', { [`tools.${vbitIndex}.stepover_mm`]: 0.2 }],
  ['clearance-5', { 'endmill_planning.clearance_z_mm': 5 }],
  ['wood-balanced', { 'import.geometry_tolerance_mm': 0.005, 'tolerances.motion_tolerance_mm': 0.05 }],
  ['wood-finish', { 'import.geometry_tolerance_mm': 0.005, 'tolerances.motion_tolerance_mm': 0.05, 'operation.max_floor_ridge_mm': 0.05 }],
  ['baseline-2', {}],
];
const rows = [];
for (const id of requested) {
  if (!variants.some(([name]) => name === id)) throw new Error(`Unknown variant: ${id}`);
}
for (const [id, changes] of variants) {
  if (requested.length && !requested.includes(id)) continue;
  const job = structuredClone(source);
  for (const [key, value] of Object.entries(changes)) {
    const keys = key.split('.');
    let parent = job;
    for (const key of keys.slice(0, -1)) parent = parent[key];
    parent[keys.at(-1)] = value;
  }
  const jobFile = path.join(out, 'jobs', `${id}.job.json`);
  const runDir = path.join(out, id);
  fs.writeFileSync(jobFile, JSON.stringify(job, null, 2) + '\n');
  process.stdout.write(`Starting ${id}\n`);
  const run = spawnSync('pwsh', ['-NoProfile', '-File', 'scripts/benchmark-flower.ps1', '-Cam', path.resolve(camArg), '-Job', jobFile, '-OutputDirectory', runDir, '-Stages', 'combined'], { encoding: 'utf8', windowsHide: true });
  fs.writeFileSync(path.join(out, `${id}.harness.log`), run.stdout + run.stderr);
  const summaryFile = path.join(runDir, 'summary.json');
  if (!fs.existsSync(summaryFile)) throw new Error(`Benchmark harness failed for ${id}; see ${id}.harness.log`);
  const row = { id, changes, harness_exit_code: run.status, ...(fs.existsSync(summaryFile) ? JSON.parse(fs.readFileSync(summaryFile, 'utf8')) : {}) };
  row.status = fs.readFileSync(path.join(runDir, 'combined.timings.txt'), 'utf8').match(/Combined M4 stage: (\w+)/)?.[1] ?? null;
  const planFile = path.join(runDir, 'combined.plan.json');
  if (fs.existsSync(planFile)) {
    const plan = JSON.parse(fs.readFileSync(planFile, 'utf8'));
    row.endmill = statistics(plan.endmill.motions);
    row.vbit = statistics(plan.vbit_motions);
    row.motion_fingerprint = plan.motion_fingerprint;
    row.generation_issues = plan.generation_issues;
    row.endmill_generation_issues = plan.endmill.generation_issues;
  }
  rows.push(row);
  fs.writeFileSync(path.join(out, 'results.json'), JSON.stringify(rows, null, 2) + '\n');
  process.stdout.write(JSON.stringify({ id, seconds: row.runs?.[0]?.seconds, exit: row.runs?.[0]?.exit_code, vbit_mm: row.vbit?.total_distance_mm, vbit_motions: row.vbit?.motions, error: run.error?.message }) + '\n');
}
