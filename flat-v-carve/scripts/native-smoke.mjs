// Native end-to-end smoke for a review build: drive the shipped executable's
// own worker mailbox exactly as the native UI does, over the GUI9 batch
// fixture, and record what the compute process actually did.
//
//   node scripts/native-smoke.mjs \
//     --exe artifacts/gui9/review/large-job-native/cam-gui.exe \
//     --out artifacts/gui/gui9-native-smoke.json
//
// The worker protocol is the application's, not a test-only path: a request
// file next to the mailbox, a response file back, metadata JSON framed by a
// u32 length followed by the binary payload.
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {mkdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync} from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const flag = (name, fallback) => {
  const index = args.indexOf(`--${name}`);
  return index >= 0 ? args[index + 1] : fallback;
};
const executable = path.resolve(root, flag('exe', 'artifacts/gui9/review/large-job-native/cam-gui.exe'));
const fixture = path.resolve(root, flag('job', 'fixtures/gui9/flower-box-batch.job.json'));
const machine = path.resolve(root, flag('machine', 'fixtures/gui2/machine.json'));
const out = path.resolve(root, flag('out', 'artifacts/gui/gui9-native-smoke.json'));
const mailbox = path.resolve(root, flag('mailbox', 'artifacts/gui/native-smoke-mailbox'));
const expectedMotions = Number(flag('motions', '137717'));

rmSync(mailbox, {recursive: true, force: true});
mkdirSync(mailbox, {recursive: true});
mkdirSync(path.dirname(out), {recursive: true});

const job = readFileSync(fixture, 'utf8');
const profile = readFileSync(machine, 'utf8');
const child = spawn(executable, ['--worker', mailbox], {
  stdio: args.includes('--verbose') ? ['ignore', 'inherit', 'inherit'] : 'ignore',
  windowsHide: !args.includes('--verbose'),
});

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

const statOrNull = file => {
  try {
    return statSync(file);
  } catch {
    return null;
  }
};

async function request(command, label) {
  const started = Date.now();
  writeFileSync(path.join(mailbox, 'pending'), JSON.stringify({Gui2: command}));
  renameSync(path.join(mailbox, 'pending'), path.join(mailbox, 'request'));
  const response = path.join(mailbox, 'response');
  const deadline = started + 300000;
  while (!statOrNull(response)) {
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${label}`);
    if (child.exitCode !== null) throw new Error(`the worker exited during ${label}`);
    await sleep(10);
  }
  const bytes = readFileSync(response);
  rmSync(response, {force: true});
  const metadataLength = bytes.readUInt32LE(0);
  const metadata = JSON.parse(bytes.subarray(4, 4 + metadataLength).toString('utf8'));
  const payloadLength = bytes.length - 4 - metadataLength;
  if (metadata.Err) throw new Error(`${label}: ${metadata.Err}`);
  const meta = metadata.Ok;
  return {meta, payloadLength, elapsedMs: Date.now() - started, label};
}

const steps = [];
const checks = [];
const record = (label, value) => {
  steps.push({label, ...value});
  console.log(`${label} :: ${JSON.stringify(value)}`);
};
const expect = (condition, message) => {
  if (!condition) throw new Error(message);
  checks.push(message);
};

try {
  const applied = await request(
    {ApplyProfile: {job, json: profile}},
    'apply machine profile',
  );
  expect(applied.meta.report.gui2.kind === 'profile', 'the machine profile applied');
  const prepared = applied.meta.job;

  const generated = await request(
    // GenerateScope is internally tagged: {"kind":"allEnabled"}.
    {Generate: {job: prepared, scope: {kind: 'allEnabled'}, preset: 'standard'}},
    'generate the batch at the standard resolution',
  );
  const report = generated.meta.report.gui2;
  expect(report.kind === 'generated', `generation reported ${report.kind}`);
  expect(report.checks?.exportReady === true, 'the generated result is export-ready');
  expect(
    generated.meta.motions === expectedMotions,
    `the batch fixture planned ${generated.meta.motions} motions (expected ${expectedMotions})`,
  );
  expect(
    generated.meta.transport.motionPages > 12,
    `the scene spans ${generated.meta.transport.motionPages} motion pages`,
  );
  const standard = generated.meta.stock;
  expect(standard.preset === 'standard', 'the scene used the requested preset');
  expect(standard.ladderFrames >= 3, `the raster retained ${standard.ladderFrames} checkpoints`);
  const handle = report.handle;
  record('generated', {
    motions: generated.meta.motions,
    payloadBytes: generated.meta.payloadBytes,
    motionPages: generated.meta.transport.motionPages,
    stockBytes: generated.meta.transport.stockBytes,
    cellMm: standard.cellMm,
    referenceCellMm: standard.referenceCellMm,
    checkpoints: standard.ladderFrames,
    simulationKey: standard.key,
    elapsedMs: generated.elapsedMs,
  });

  const half = Math.floor(generated.meta.motions / 2);
  const seeked = await request({Seek: {handle, prefix: half}}, 'seek to half way');
  expect(seeked.meta.report.gui2.kind === 'seek', 'the seek answered with a stock frame');
  expect(
    seeked.meta.stock.frames[0].prefix === half,
    `the seek returned prefix ${seeked.meta.stock.frames[0].prefix}`,
  );
  record('seek', {
    prefix: seeked.meta.stock.frames[0].prefix,
    replayedMotions: seeked.meta.report.gui2.replayed,
    payloadBytes: seeked.payloadLength,
    elapsedMs: seeked.elapsedMs,
  });

  const fine = await request(
    {DisplayPreset: {handle, preset: 'fine'}},
    'rebuild the display at the fine resolution',
  );
  const fineReport = fine.meta.report.gui2;
  const fineStock = fine.meta.stock;
  expect(fineReport.kind === 'preset' && fineReport.changed === true, 'the display rebuilt');
  expect(fineStock.cellMm < standard.cellMm, 'the fine preset resolved smaller cells');
  expect(fineStock.key !== standard.key, 'the rebuild produced a new simulation key');
  expect(
    fineStock.frames[0].prefix === half,
    'the rebuild kept the playhead it was asked to compare at',
  );
  record('display rebuilt at another resolution', {
    preset: fineStock.preset,
    cellMm: fineStock.cellMm,
    referenceCellMm: fineStock.referenceCellMm,
    checkpoints: fineStock.ladderFrames,
    retainedBytes: fineStock.retainedBytes,
    simulationKey: fineStock.key,
    playhead: fineStock.frames[0].prefix,
    payloadBytes: fine.payloadLength,
    elapsedMs: fine.elapsedMs,
  });

  const tenth = Math.floor(generated.meta.motions / 10);
  const afterPreset = await request({Seek: {handle, prefix: tenth}}, 'seek after the rebuild');
  expect(
    afterPreset.meta.stock.key === fineStock.key,
    'a seek after the rebuild stays on the new simulation key',
  );
  record('seek after the rebuild', {
    prefix: afterPreset.meta.stock.frames[0].prefix,
    simulationKey: afterPreset.meta.stock.key,
    elapsedMs: afterPreset.elapsedMs,
  });

  const preparedOutput = await request(
    {Prepare: {job: prepared, handle}},
    'prepare the checked program',
  );
  const output = preparedOutput.meta.report.gui2;
  expect(output.kind === 'prepared', 'the prepared program came back');
  expect(
    output.retained?.plansRun === 1,
    'preparation reused the retained plan instead of replanning',
  );
  expect(
    /^[0-9a-f]{64}$/.test(output.file.sha256 ?? ''),
    'the emitted program carries a SHA-256',
  );
  record('prepared output', {
    filename: output.file.filename,
    sha256: output.file.sha256,
    plansRun: output.retained.plansRun,
    payloadBytes: preparedOutput.payloadLength,
    elapsedMs: preparedOutput.elapsedMs,
  });

  const evidence = {
    createdAt: new Date().toISOString(),
    executable,
    executableSha256: createHash('sha256').update(readFileSync(executable)).digest('hex'),
    fixture: path.relative(root, fixture),
    fixtureSha256: createHash('sha256').update(job).digest('hex'),
    platform: process.platform,
    protocol: 'cam-gui-retained-5',
    steps,
    checks,
    note:
      'Native release executable driven through its own worker mailbox: the same ' +
      'request/response path the native UI uses. No window, GPU or input device was ' +
      'involved; the interactive tour is the manual review.',
  };
  writeFileSync(out, `${JSON.stringify(evidence, null, 2)}\n`);
  console.log(`native smoke passed · ${checks.length} checks · ${path.relative(root, out)}`);
} catch (error) {
  console.error(`FAILED: ${error.message}`);
  process.exitCode = 1;
} finally {
  child.kill();
  await sleep(150);
  rmSync(mailbox, {recursive: true, force: true});
}
