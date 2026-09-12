#!/usr/bin/env node
// Turn a measure-validation.ps1 results.json into a summary plus SVG charts.
//
//   node scripts/validation-timing-report.mjs artifacts/validation-timing/ci-run/results.json
//   node scripts/validation-timing-report.mjs results.json --merge other-results.json
//
// Writes summary.md, timing-bars.svg, and timing-timeline.svg next to the input.
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';

// Canonical CI order, used for the waterfall and for ordering merged runs.
const CI_ORDER = [
  'rust-fmt', 'gui-native-build', 'gui-web-build', 'portable-cargo-build',
  'portable-script', 'rust-clippy', 'rust-test-compile', 'rust-test-run',
];

const argv = process.argv.slice(2);
const mergePaths = [];
for (let index = 0; index < argv.length; index++) {
  if (argv[index] === '--merge') mergePaths.push(argv[++index]);
}
const positional = argv.filter((value, index) => value !== '--merge' && argv[index - 1] !== '--merge');
const input = positional[0] ?? 'artifacts/validation-timing/results.json';
const resultsPath = resolve(input);
const outDir = dirname(resultsPath);
const results = JSON.parse(readFileSync(resultsPath, 'utf8'));
const primary = [...results.steps];
// Fill steps that the primary run did not measure, or that failed for harness
// reasons and were re-measured on their own. Later files lose to earlier ones.
const candidates = {};
for (const path of mergePaths) {
  const other = JSON.parse(readFileSync(resolve(path), 'utf8'));
  for (const step of other.steps) {
    const existing = candidates[step.id];
    if (!existing || existing.status !== 'ok') candidates[step.id] = { ...step, source: path };
  }
}
const merged = {};
const replaced = [];
for (const step of primary) {
  if (step.status === 'ok' || !candidates[step.id]) continue;
  merged[step.id] = candidates[step.id];
  replaced.push(step.id);
}
const effective = primary.filter(step => !merged[step.id]).concat(Object.values(merged));
const orderOf = id => {
  const index = CI_ORDER.indexOf(id);
  return index === -1 ? CI_ORDER.length : index;
};
const steps = [...effective].sort((a, b) => orderOf(a.id) - orderOf(b.id));
const summedSeconds = steps.reduce((sum, step) => sum + step.seconds, 0);
// A merged dataset keeps the primary run's wall clock only if nothing was merged in.
const total = Object.keys(merged).length > 0 ? summedSeconds : (results.totalSeconds ?? summedSeconds);
const mergedIds = Object.keys(merged);

const PALETTE = [
  '#2563eb', '#0d9488', '#7c3aed', '#b45309', '#c2410c',
  '#0369a1', '#15803d', '#be123c', '#4d7c0f', '#6d28d9',
  '#a16207', '#1e40af', '#b91c1c', '#0f766e', '#9333ea',
];
const categories = [...new Set(steps.map(step => step.category))];
const colorFor = category => PALETTE[categories.indexOf(category) % PALETTE.length];

// CI sets CARGO_TERM_COLOR=always, so every captured log carries ANSI codes.
const stripAnsi = text => text.replace(/\u001b\[[0-9;]*m/g, '');

function logText(step) {
  try {
    return stripAnsi(readFileSync(step.log, 'utf8'));
  } catch {
    return '';
  }
}

// Cargo reports its own elapsed build time; compare it with the wall clock we
// measured to expose harness and process-startup overhead.
function cargoDuration(value) {
  const minutes = value.match(/(\d+)m\s*([\d.]+)s/);
  if (minutes) return Number(minutes[1]) * 60 + Number(minutes[2]);
  const seconds = value.match(/([\d.]+)s/);
  return seconds ? Number(seconds[1]) : null;
}

function cargoReportedSeconds(text) {
  const matches = [...text.matchAll(/Finished `[^`]+` profile \[[^\]]+\] target\(s\) in ([^\r\n]+)/g)];
  return matches.length ? cargoDuration(matches.at(-1)[1]) : null;
}

function cargoTestExecSeconds(text) {
  const matches = [...text.matchAll(/test result: \w+\. \d+ passed; \d+ failed; \d+ ignored; [^\n]*finished in ([\d.]+)s/g)];
  return matches.length ? matches.reduce((sum, match) => sum + Number(match[1]), 0) : null;
}

function cargoTestCounts(text) {
  const matches = [...text.matchAll(/test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored/g)];
  return {
    passed: matches.reduce((sum, match) => sum + Number(match[1]), 0),
    failed: matches.reduce((sum, match) => sum + Number(match[2]), 0),
    ignored: matches.reduce((sum, match) => sum + Number(match[3]), 0),
    binaries: matches.length,
  };
}

function vitestSummary(text) {
  const duration = text.match(/Duration\s+([\d.]+)s/);
  const files = text.match(/Test Files\s+(?:(\d+) failed[^\n]*?)?(\d+) passed/);
  const tests = text.match(/Tests\s+(?:(\d+) failed[^\n]*?)?(\d+) passed/);
  return {
    reportedSeconds: duration ? Number(duration[1]) : null,
    testFiles: files ? Number(files[2]) : null,
    tests: tests ? Number(tests[2]) : null,
  };
}

const enriched = steps.map(step => {
  const text = logText(step);
  const testCounts = step.id.startsWith('rust-test') ? cargoTestCounts(text) : null;
  return {
    ...step,
    percent: total > 0 ? (step.seconds / total) * 100 : 0,
    cargoReportedSeconds: cargoReportedSeconds(text),
    testExecSeconds: step.id.startsWith('rust-test') ? cargoTestExecSeconds(text) : null,
    testCounts,
    vitest: step.id === 'frontend-test' || step.id === 'live-integration' ? vitestSummary(text) : null,
    compileLines: (text.match(/^\s+(?:Compiling|Checking)\s+\S+\s+v/gm) ?? []).length,
  };
});

const categoryTotals = categories
  .map(category => ({
    category,
    seconds: enriched.filter(step => step.category === category).reduce((sum, step) => sum + step.seconds, 0),
    steps: enriched.filter(step => step.category === category).length,
  }))
  .sort((a, b) => b.seconds - a.seconds);

const failed = enriched.filter(step => step.status !== 'ok');
const escaped = value => String(value).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
const seconds = value => `${value.toFixed(1)}s`;

function barChart() {
  const sorted = [...enriched].sort((a, b) => b.seconds - a.seconds);
  const rowHeight = 26;
  const top = 74;
  const left = 330;
  const width = 980;
  const plotWidth = width - left - 110;
  const height = top + sorted.length * rowHeight + 46;
  const max = Math.max(...sorted.map(step => step.seconds), 0.001);
  const rows = sorted.map((step, index) => {
    const y = top + index * rowHeight;
    const barWidth = Math.max((step.seconds / max) * plotWidth, 1);
    const color = step.status === 'ok' ? colorFor(step.category) : '#9ca3af';
    return `
    <text x="${left - 12}" y="${y + 15}" text-anchor="end" class="label">${escaped(step.id)}</text>
    <rect x="${left}" y="${y + 3}" width="${barWidth.toFixed(1)}" height="17" rx="3" fill="${color}"/>
    <text x="${(left + barWidth + 8).toFixed(1)}" y="${y + 16}" class="value">${seconds(step.seconds)} · ${step.percent.toFixed(1)}%</text>`;
  }).join('');
  const legend = categoryTotals.map((entry, index) => {
    const x = 24 + index * 190;
    return `<g transform="translate(${x}, 640)"><rect width="12" height="12" rx="2" fill="${colorFor(entry.category)}"/>
      <text x="18" y="11" class="legend">${escaped(entry.category)} — ${seconds(entry.seconds)}</text></g>`;
  }).join('');
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}" font-family="Segoe UI, system-ui, sans-serif">
  <style>
    .title { font-size: 19px; font-weight: 600; fill: #111827; }
    .subtitle { font-size: 12.5px; fill: #4b5563; }
    .label { font-size: 12.5px; fill: #1f2937; }
    .value { font-size: 12px; fill: #374151; }
    .legend { font-size: 11.5px; fill: #374151; }
    .grid { stroke: #e5e7eb; stroke-width: 1; }
  </style>
  <rect width="${width}" height="${height}" fill="#ffffff"/>
  <text x="24" y="30" class="title">Validation step wall time (single warm-cache run)</text>
  <text x="24" y="50" class="subtitle">${escaped(results.label || 'unlabelled')} · total ${seconds(total)} · ${sorted.length} steps · longest first</text>
  ${rows}
  ${legend}
</svg>`;
}

function timelineChart() {
  const rowHeight = 24;
  const top = 76;
  const left = 250;
  const width = 980;
  const plotWidth = width - left - 130;
  const height = top + steps.length * rowHeight + 40;
  const ticks = 6;
  let cursor = 0;
  const rows = steps.map((step, index) => {
    const y = top + index * rowHeight;
    const x = left + (cursor / total) * plotWidth;
    const barWidth = Math.max((step.seconds / total) * plotWidth, 1);
    cursor += step.seconds;
    const color = step.status === 'ok' ? colorFor(step.category) : '#9ca3af';
    return `
    <text x="${left - 12}" y="${y + 15}" text-anchor="end" class="label">${index + 1}. ${escaped(step.id)}</text>
    <rect x="${x.toFixed(1)}" y="${y + 3}" width="${barWidth.toFixed(1)}" height="16" rx="3" fill="${color}"/>
    <text x="${(x + barWidth + 8).toFixed(1)}" y="${y + 16}" class="value">${seconds(step.seconds)} → ${seconds(cursor)}</text>`;
  }).join('');
  const gridlines = Array.from({ length: ticks + 1 }, (_, index) => {
    const value = (total / ticks) * index;
    const x = left + (value / total) * plotWidth;
    return `<line x1="${x.toFixed(1)}" y1="${top - 8}" x2="${x.toFixed(1)}" y2="${top + steps.length * rowHeight}" class="grid"/>
    <text x="${x.toFixed(1)}" y="${top - 14}" text-anchor="middle" class="value">${Math.round(value)}s</text>`;
  }).join('');
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}" font-family="Segoe UI, system-ui, sans-serif">
  <style>
    .title { font-size: 19px; font-weight: 600; fill: #111827; }
    .subtitle { font-size: 12.5px; fill: #4b5563; }
    .label { font-size: 12.5px; fill: #1f2937; }
    .value { font-size: 11.5px; fill: #374151; }
    .grid { stroke: #e5e7eb; stroke-width: 1; }
  </style>
  <rect width="${width}" height="${height}" fill="#ffffff"/>
  <text x="24" y="30" class="title">Validation pipeline waterfall (CI order)</text>
  <text x="24" y="50" class="subtitle">Bar offset is cumulative wall time; label shows step duration and running total. Total ${seconds(total)}.</text>
  ${gridlines}
  ${rows}
</svg>`;
}

function percent(value) {
  return `${value.toFixed(1)}%`;
}

const environment = results.environment ?? {};
const lines = [];
lines.push('# Project validation timing');
lines.push('');
lines.push(`- Label: \`${results.label || 'unlabelled'}\``);
lines.push(`- Measured: ${results.startedUtc} → ${results.finishedUtc}`);
lines.push(`- Total wall time: **${seconds(total)}** across ${steps.length} steps${mergedIds.length ? ' (sum of both runs; see †)' : ''}`);
if (mergedIds.length) {
  lines.push(`- Steps taken from another run: ${mergedIds.map(id => `\`${id}\` ← ${merged[id].source}`).join(', ')}`);
}
lines.push(`- Steps failing: ${failed.length}${failed.length ? ` (${failed.map(step => step.id).join(', ')})` : ''}`);
lines.push(`- Host: ${environment.cpu ?? 'unknown CPU'} · ${environment.logicalProcessors ?? '?'} logical processors · ${environment.memoryGB ?? '?'} GB RAM`);
lines.push(`- Commit: \`${(environment.gitCommit ?? '').slice(0, 12)}\` (${environment.gitDirtyFiles ?? '?'} dirty files) · rustc ${environment.rustc ?? '?'} · node ${environment.node ?? '?'} · pnpm ${environment.pnpm ?? '?'} · wasm-pack ${environment.wasmPack ?? '?'}`);
lines.push(`- Cargo profile: \`${environment.rustFlagsProfile}\` · offline: ${environment.offline}`);
lines.push('');
lines.push('## Per-step timings');
lines.push('');
lines.push('| # | Step | Category | Wall | Share | Exit | Rebuilt crates | CI step |');
lines.push('| --- | --- | --- | ---: | ---: | --- | ---: | --- |');
enriched.forEach((step, index) => {
  lines.push(`| ${index + 1} | \`${step.id}\`${step.source ? ' †' : ''} | ${step.category} | ${step.seconds.toFixed(2)}s | ${percent(step.percent)} | ${step.exitCode} | ${step.compileLines} | ${step.ciStep} |`);
});
lines.push('');
lines.push('## Category rollup');
lines.push('');
lines.push('| Category | Wall | Share | Steps |');
lines.push('| --- | ---: | ---: | ---: |');
for (const entry of categoryTotals) {
  lines.push(`| ${entry.category} | ${entry.seconds.toFixed(2)}s | ${percent(total > 0 ? (entry.seconds / total) * 100 : 0)} | ${entry.steps} |`);
}
lines.push('');
lines.push('## Where the time goes');
lines.push('');
const top = [...enriched].sort((a, b) => b.seconds - a.seconds).slice(0, 5);
for (const step of top) {
  lines.push(`- \`${step.id}\`: ${step.seconds.toFixed(1)}s (${percent(step.percent)}) — \`${step.command}\``);
}
lines.push('');
lines.push('## Measured work inside the big steps');
lines.push('');
lines.push('| Step | Wall | Cargo-reported build | Test execution | Tests | Reported by tool |');
lines.push('| --- | ---: | ---: | ---: | ---: | --- |');
for (const step of enriched) {
  const reported = [];
  if (step.cargoReportedSeconds !== null) reported.push(`cargo says ${step.cargoReportedSeconds}s`);
  if (step.testExecSeconds !== null) reported.push(`${step.testExecSeconds.toFixed(2)}s in test binaries`);
  if (step.testCounts && step.testCounts.binaries) {
    reported.push(`${step.testCounts.passed} passed / ${step.testCounts.failed} failed / ${step.testCounts.ignored} ignored`);
  }
  if (step.vitest && step.vitest.reportedSeconds !== null) {
    reported.push(`vitest ${step.vitest.reportedSeconds}s, ${step.vitest.tests ?? '?'} tests in ${step.vitest.testFiles ?? '?'} files`);
  }
  lines.push(`| \`${step.id}\` | ${step.seconds.toFixed(2)}s | ${step.cargoReportedSeconds ?? '—'} | ${step.testExecSeconds !== null ? step.testExecSeconds.toFixed(2) + 's' : '—'} | ${step.testCounts && step.testCounts.binaries ? step.testCounts.passed : '—'} | ${reported.join('; ') || '—'} |`);
}
lines.push('');
lines.push('Generated by `scripts/validation-timing-report.mjs` from `results.json`.');
lines.push('');

writeFileSync(join(outDir, 'summary.md'), lines.join('\n'));
writeFileSync(join(outDir, 'timing-bars.svg'), barChart());
writeFileSync(join(outDir, 'timing-timeline.svg'), timelineChart());
console.log(`Wrote summary.md, timing-bars.svg, timing-timeline.svg in ${outDir}`);
console.log(`Total measured wall time: ${seconds(total)} (${steps.length} steps, ${failed.length} failing)`);
