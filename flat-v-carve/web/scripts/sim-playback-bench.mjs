// U8 phase-1 experiment 4: playback pipeline costs on flower-like streams.
// Run with: node scripts/sim-playback-bench.mjs
// Measures what the phase-2 worker must sustain: per-frame seek batches with
// tile-delta emission, undo-log snapshots under a byte cap, and rewind
// latency. The rAF clock and GPU upload belong to the browser spike page.
//
// The original plan's full-dirty-tile snapshots degenerate at flower scale
// (every late snapshot copies the whole ~46 MB field, so a 64 MB cap retains
// about one snapshot and rewind breaks). This bench measures the undo-log
// replacement: the engine's onTileChange hook captures each tile once per
// interval before its first change; rewind undoes intervals and re-applies.
import {
  applyMotion,
  beginEpoch,
  chooseResolution,
  createField,
  normalizeTool,
} from '../src/sim/engine.ts';

const mulberry32 = seed => {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) | 0;
    let t = Math.imul(state ^ (state >>> 15), 1 | state);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
};

const sheet = { x0: 0, y0: 0, x1: 900, y1: 600, thicknessMm: 12 };
const resolution = chooseResolution(sheet, 0.2);
const endmill = normalizeTool({ kind: 'endmill', diameterMm: 3 });
const vbit = normalizeTool({ kind: 'vbit', includedAngleDeg: 60, tipDiameterMm: 0.2, maxCuttingDiameterMm: 3.175, cuttingHeightMm: 5 });
const ENDMILL_FEED = 1800;
const VBIT_FEED = 1000;

const roughing = () => {
  const motions = [];
  const passes = Math.ceil(350 / 1.5);
  for (let layer = 0; layer < 2; layer++) {
    const z = -0.75 - layer * 0.75;
    for (let pass = 0; pass < passes; pass++) {
      const y = 125 + pass * 1.5;
      const forward = (pass + layer) % 2 === 0;
      motions.push({ kind: 'cut', tool: 0, x0: forward ? 200 : 700, y0: y, z0: z, x1: forward ? 700 : 200, y1: y, z1: z });
    }
  }
  return motions;
};

// Adversarial detail: spatially random walk (poor locality, worst case).
const detailRandom = count => {
  const random = mulberry32(42);
  const motions = [];
  let x = 220 + random() * 460;
  let y = 145 + random() * 310;
  let z = -1.2;
  for (let index = 0; index < count; index++) {
    const angle = random() * Math.PI * 2;
    const length = 0.4 + random() * 2.6;
    const nx = Math.min(695, Math.max(205, x + Math.cos(angle) * length));
    const ny = Math.min(460, Math.max(140, y + Math.sin(angle) * length));
    const nz = Math.min(-0.3, Math.max(-2.4, z + (random() - 0.5) * 0.8));
    motions.push({ kind: 'cut', tool: 1, x0: x, y0: y, z0: z, x1: nx, y1: ny, z1: nz });
    x = nx; y = ny; z = nz;
  }
  return motions;
};

// Realistic detail: serpentine finishing passes (contour-like locality).
const detailSerpentine = count => {
  const motions = [];
  let index = 0;
  let pass = 0;
  while (index < count) {
    const y = 140 + pass * 0.32;
    const depth = -(0.6 + 1.4 * (0.5 + 0.5 * Math.sin(pass * 0.37)));
    const forward = pass % 2 === 0;
    let x = forward ? 205 : 695;
    motions.push({ kind: 'plunge', tool: 1, x0: x, y0: y, z0: -0.3, x1: x, y1: y, z1: depth });
    index++;
    while (index < count && (forward ? x < 690 : x > 210)) {
      const nx = x + (forward ? 1.5 : -1.5);
      motions.push({ kind: 'cut', tool: 1, x0: x, y0: y, z0: depth, x1: nx, y1: y, z1: depth });
      index++;
      x = nx;
    }
    pass++;
  }
  return motions;
};

const modelSeconds = motions => motions.reduce((sum, m) =>
  sum + Math.hypot(m.x1 - m.x0, m.y1 - m.y0) / (m.tool === 0 ? ENDMILL_FEED : VBIT_FEED) * 60, 0);

const CAP = 64 * 1024 * 1024;

const runVariant = (label, detail) => {
  console.log(`\n=== ${label} ===`);
  const motions = [...roughing(), ...detail];
  const totalSeconds = modelSeconds(motions);
  console.log(`stream: ${motions.length} motions, ${(totalSeconds / 60).toFixed(0)} min model cutting time`);

  // -- Per-frame playback batches with delta emission ----------------------
  const field = createField(sheet, [endmill, vbit], resolution);
  const emitted = new Uint32Array(field.tileVersions.length);
  let applied = 0;
  for (const speed of [15, 60, 200]) {
    const batch = Math.max(1, Math.round(motions.length / totalSeconds * speed / 60));
    emitted.fill(0);
    const frames = 3600;
    const started = performance.now();
    let deltaTiles = 0;
    for (let frame = 0; frame < frames; frame++) {
      const target = Math.min(motions.length, applied + batch);
      for (; applied < target; applied++) applyMotion(field, motions[applied]);
      for (let tile = 0; tile < emitted.length; tile++) {
        if (field.tileVersions[tile] !== emitted[tile]) { emitted[tile] = field.tileVersions[tile]; deltaTiles++; }
      }
    }
    const wall = (performance.now() - started) / 1000;
    const played = frames * batch / (motions.length / totalSeconds);
    console.log(`playback ${String(speed).padStart(3)}x: batch ${batch}/frame, 3600 frames in ${wall.toFixed(2)} s, delta tiles ${deltaTiles}, sustained ${(played / wall).toFixed(0)}x -> gate ${played / wall >= 15 ? 'PASS' : 'FAIL'}`);
  }

  // -- Undo-log snapshots with byte cap ------------------------------------
  const interval = Math.max(1024, Math.floor(motions.length / 256));
  const logs = [];            // logs[k] = Map(tile -> {h, o}) at interval k start
  let logBytes = 0;
  let firstIndex = 0;         // original interval index of logs[0] after evictions
  field.onTileChange = tile => {
    const current = logs[logs.length - 1];
    if (current === undefined || current.has(tile)) return;
    const h = field.heights[tile];
    const o = field.cellOwner[tile];
    if (h === undefined || o === undefined) return; // pristine: replay re-derives allocation
    current.set(tile, { h: h.slice(), o: o.slice() });
    logBytes += h.byteLength + o.byteLength;
  };
  const trim = () => {
    while (logBytes > CAP && logs.length > 1) {
      for (const s of logs[0].values()) logBytes -= s.h.byteLength + s.o.byteLength;
      logs.shift();
      firstIndex++;
    }
  };
  // Restart from a pristine field, snapshotting every interval.
  for (let tile = 0; tile < field.heights.length; tile++) { field.heights[tile] = undefined; field.cellOwner[tile] = undefined; }
  field.stats.dirtyCells = 0;
  field.stats.removedVolumeMm3 = 0;
  field.stats.stageRemovedMm3 = [0, 0];
  field.tileVersions.fill(0);
  applied = 0;
  const snapStart = performance.now();
  let peakBytes = 0;
  logs.push(new Map());
  beginEpoch(field);
  for (let index = 0; index < motions.length; index++) {
    if (index > 0 && index % interval === 0) {
      trim();
      peakBytes = Math.max(peakBytes, logBytes);
      logs.push(new Map());
      beginEpoch(field);
    }
    applyMotion(field, motions[index]);
  }
  trim();
  const snapMs = performance.now() - snapStart;
  console.log(`undo log: interval ${interval}, retained ${logs.length}/${Math.ceil(motions.length / interval)} intervals (first at ${firstIndex * interval}), ${(logBytes / 1e6).toFixed(0)} MB under ${(CAP / 1e6).toFixed(0)} MB cap, peak ${(peakBytes / 1e6).toFixed(0)} MB, logging overhead ${(snapMs / 1000).toFixed(1)} s cumulative`);

  // -- Rewind battery -------------------------------------------------------
  const rewind = target => {
    const started = performance.now();
    const j = Math.floor(target / interval);
    let undone = 0;
    if (j < firstIndex) return { ms: performance.now() - started, mode: 'fallback-needed', undone: 0 };
    while (logs.length - 1 > j - firstIndex) {
      const log = logs.pop();
      for (const [tile, s] of log) {
        field.heights[tile] = s.h.slice();
        field.cellOwner[tile] = s.o.slice();
        field.tileVersions[tile]++;
        logBytes -= s.h.byteLength + s.o.byteLength;
      }
      undone++;
    }
    const replayFrom = j * interval;
    for (let index = replayFrom; index < target; index++) applyMotion(field, motions[index]);
    return { ms: performance.now() - started, mode: 'undo+replay', undone, replayed: target - replayFrom };
  };
  const targets = [0.9, 0.5, 0.1];
  for (const fraction of targets) {
    const result = rewind(Math.floor(motions.length * fraction));
    const gate = result.mode === 'undo+replay' && result.ms <= 1000 ? 'PASS' : result.mode === 'fallback-needed' ? 'FALLBACK' : 'FAIL';
    console.log(`rewind to ${Math.round(fraction * 100)}%: ${result.mode}, ${result.undone} intervals undone + ${result.replayed ?? 0} replayed in ${result.ms.toFixed(0)} ms -> ${gate}`);
  }
  const step = rewind(Math.floor(motions.length * 0.5) - 1024);
  console.log(`rewind step -1024 motions: ${step.mode} in ${step.ms.toFixed(1)} ms`);

  // Fallback replay for targets older than the retained window.
  const fallbackStart = performance.now();
  for (let tile = 0; tile < field.heights.length; tile++) { field.heights[tile] = undefined; field.cellOwner[tile] = undefined; }
  field.stats.dirtyCells = 0;
  const fallbackTarget = Math.floor(motions.length * 0.1);
  for (let index = 0; index < fallbackTarget; index++) applyMotion(field, motions[index]);
  console.log(`fallback full replay to 10%: ${((performance.now() - fallbackStart) / 1000).toFixed(1)} s (bounded by the one-shot apply time)`);
  const memory = process.memoryUsage();
  console.log(`memory: heap ${(memory.heapUsed / 1e6).toFixed(0)} MB, rss ${(memory.rss / 1e6).toFixed(0)} MB`);
};

runVariant('adversarial detail (random walk, poor locality)', detailRandom(300_000));
runVariant('realistic detail (serpentine finishing, contour locality)', detailSerpentine(300_000));
