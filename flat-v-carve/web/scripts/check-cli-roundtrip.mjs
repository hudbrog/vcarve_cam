import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, writeFileSync, unlinkSync, rmdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { parseJob } from '../src/contracts/job.ts';

const binary = process.argv[2];
if (!binary) throw new Error('Pass an existing cam executable path. This check never builds Rust.');
const executable = resolve(binary);
const directory = mkdtempSync(join(tmpdir(), 'flat-v-carve-u1-'));
const file = join(directory, 'roundtrip.job.json');
const cases = [
  new URL('../src/fixtures/inkscape.job.json', import.meta.url),
  new URL('../../fixtures/m4/finite-tip.json', import.meta.url),
];
try {
  for (const source of cases) {
    const original = JSON.parse(readFileSync(source, 'utf8'));
    const parsed = parseJob(original);
    assert.deepEqual(parsed, original, 'Frontend parsing changed portable settings');
    writeFileSync(file, JSON.stringify(parsed) + '\n');
    const result = spawnSync(executable, ['validate-job', file], { encoding: 'utf8', maxBuffer: 16 * 1024 * 1024, windowsHide: true });
    if (result.error) throw result.error;
    assert.equal(result.status, 0, `Rust rejected round-tripped ${original.name}: ${result.stderr}`);
    const report = JSON.parse(result.stdout);
    assert.equal(report.valid, true);
    const inspection = report.inspection;
    assert.equal(inspection.name, original.name);
    console.log(`Rust ${inspection.engine_version} accepted ${original.name}; ${inspection.missing_machining_fields.length} fields remain unset.`);
  }
} finally {
  // Delete only the single file we wrote, then the empty directory; never recurse.
  try { unlinkSync(file); } catch (error) { if (error.code !== 'ENOENT') throw error; }
  rmdirSync(directory);
}
