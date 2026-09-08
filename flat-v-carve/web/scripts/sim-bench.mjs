// U8 phase-1 benchmark: sweep-engine throughput and a flower-scale one-shot.
// Run with: node scripts/sim-bench.mjs
// The engine module is import-free TypeScript so Node's type stripping can
// load it directly; numbers land in docs/flat-v-carve/web-ui/u8-stock-simulator.md.
import {
  applyMotions,
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

const seconds = ms => ms / 1000;
const report = (name, lines) => {
  console.log(`\n== ${name} ==`);
  for (const line of lines) console.log(`   ${line}`);
};

// --- Bench A: worst-case endmill serpentine (6 mm tool, 0.1 mm cells) -----
{
  const cell = 0.1;
  const tool = normalizeTool({ kind: 'endmill', diameterMm: 6 });
  const field = createField({ x0: 0, y0: 0, x1: 400, y1: 300, thicknessMm: 12 }, [tool], { cellMm: cell, cappedByTexels: false, cappedByBudget: false });
  const passes = 100; // 300 mm span / 3 mm stepover
  const length = 294;
  const laps = 5;
  function* motions() {
    for (let lap = 0; lap < laps; lap++) {
      const z = -1.5 - lap;
      for (let pass = 0; pass < passes; pass++) {
        const y = 3 + pass * 3;
        const forward = (pass + lap) % 2 === 0;
        yield { kind: 'cut', tool: 0, x0: forward ? 3 : 297, y0: y, z0: z, x1: forward ? 297 : 3, y1: y, z1: z };
      }
    }
  }
  const started = performance.now();
  applyMotions(field, motions());
  const elapsed = seconds(performance.now() - started);
  const visits = passes * laps * ((6 + length) * 6) / (cell * cell);
  report('A: endmill serpentine, d6, 0.1 mm cells', [
    `motions applied: ${field.stats.cuttingMotions} (re-sweeping the same ${passes}-pass region ${laps} times)`,
    `wall time: ${elapsed.toFixed(3)} s`,
    `AABB cell visits: ${(visits / 1e6).toFixed(1)} M -> ${(visits / elapsed / 1e6).toFixed(1)} M cells/s`,
    `gate: >= 4.5 M cells/s -> ${visits / elapsed >= 4.5e6 ? 'PASS' : 'FAIL'}`,
  ]);
}

// --- Bench A2: V-bit detail traffic (short segments, fine cone) -----------
{
  const cell = 0.1;
  const tool = normalizeTool({ kind: 'vbit', includedAngleDeg: 60, tipDiameterMm: 0.2, maxCuttingDiameterMm: 3.175, cuttingHeightMm: 5 });
  const field = createField({ x0: 0, y0: 0, x1: 400, y1: 300, thicknessMm: 12 }, [tool], { cellMm: cell, cappedByTexels: false, cappedByBudget: false });
  const random = mulberry32(20260907);
  const count = 100_000;
  let totalLength = 0;
  function* motions() {
    let x = 20 + random() * 360;
    let y = 20 + random() * 260;
    let z = -(0.3 + random() * 1.9);
    for (let index = 0; index < count; index++) {
      const angle = random() * Math.PI * 2;
      const length = 0.4 + random() * 2.6;
      const nx = Math.min(395, Math.max(5, x + Math.cos(angle) * length));
      const ny = Math.min(285, Math.max(5, y + Math.sin(angle) * length));
      const nz = Math.min(-0.3, Math.max(-2.2, z + (random() - 0.5) * 0.4));
      totalLength += Math.hypot(nx - x, ny - y);
      yield { kind: 'cut', tool: 0, x0: x, y0: y, z0: z, x1: nx, y1: ny, z1: nz };
      x = nx; y = ny; z = nz;
    }
  }
  const started = performance.now();
  applyMotions(field, motions());
  const elapsed = seconds(performance.now() - started);
  const diameter = 2 * tool.maxRadiusMm;
  const visits = count * ((diameter + totalLength / count) * diameter) / (cell * cell);
  report('A2: V-bit detail traffic, 60 deg / 0.2 mm tip, 0.1 mm cells', [
    `motions applied: ${count}, mean length ${(totalLength / count).toFixed(2)} mm`,
    `wall time: ${elapsed.toFixed(3)} s`,
    `AABB cell visits: ${(visits / 1e6).toFixed(1)} M -> ${(visits / elapsed / 1e6).toFixed(1)} M cells/s`,
    `gate: >= 4.5 M cells/s -> ${visits / elapsed >= 4.5e6 ? 'PASS' : 'FAIL'}`,
  ]);
}

// --- Bench B: flower-scale one-shot on a 900x600 sheet --------------------
// Two scenarios: a realistic flower-like detail load and a deliberate
// 2-4x-heavier worst case. Roughing clears the region to -1.5 mm and the
// V-bit dips to -2.4 mm, so rest machining removes real volume.
const runFlower = (detailCount, label) => {
  const sheet = { x0: 0, y0: 0, x1: 900, y1: 600, thicknessMm: 12 };
  const detail = 0.2; // smallest cutting detail: V-bit tip diameter
  const resolution = chooseResolution(sheet, detail);
  const endmill = normalizeTool({ kind: 'endmill', diameterMm: 3 });
  const vbit = normalizeTool({ kind: 'vbit', includedAngleDeg: 60, tipDiameterMm: 0.2, maxCuttingDiameterMm: 3.175, cuttingHeightMm: 5 });
  const field = createField(sheet, [endmill, vbit], resolution);

  // Roughing: d3 serpentine over 500x350, 1.5 mm stepover, two stepdowns.
  const passes = Math.ceil(350 / 1.5);
  const ENDMILL_FEED = 1800; // mm/min, synthetic but representative
  const VBIT_FEED = 1000;
  let roughingPath = 0;
  function* roughing() {
    for (let layer = 0; layer < 2; layer++) {
      const z = -0.75 - layer * 0.75;
      for (let pass = 0; pass < passes; pass++) {
        const y = 125 + pass * 1.5;
        const forward = (pass + layer) % 2 === 0;
        const x0 = forward ? 200 : 700;
        const x1 = forward ? 700 : 200;
        roughingPath += Math.abs(x1 - x0);
        yield { kind: 'cut', tool: 0, x0, y0: y, z0: z, x1, y1: y, z1: z };
      }
    }
  }

  // Detail: short V-bit segments inside the artwork region, dipping below
  // the roughing floor near their deepest thirds.
  const random = mulberry32(42);
  let detailPath = 0;
  function* detailing() {
    let x = 220 + random() * 460;
    let y = 145 + random() * 310;
    let z = -1.2;
    for (let index = 0; index < detailCount; index++) {
      const angle = random() * Math.PI * 2;
      const length = 0.4 + random() * 2.6;
      const nx = Math.min(695, Math.max(205, x + Math.cos(angle) * length));
      const ny = Math.min(460, Math.max(140, y + Math.sin(angle) * length));
      const nz = Math.min(-0.3, Math.max(-2.4, z + (random() - 0.5) * 0.8));
      detailPath += Math.hypot(nx - x, ny - y);
      yield { kind: 'cut', tool: 1, x0: x, y0: y, z0: z, x1: nx, y1: ny, z1: nz };
      x = nx; y = ny; z = nz;
    }
  }

  const before = process.memoryUsage();
  const started = performance.now();
  applyMotions(field, roughing());
  const roughingMs = performance.now() - started;
  applyMotions(field, detailing());
  const detailMs = performance.now() - started - roughingMs;
  const elapsed = seconds(roughingMs + detailMs);
  const after = process.memoryUsage();

  // Cutting-time-only model (the plan's cutting-time-only toggle): rapid
  // travel is excluded from the playback clock.
  const modelSeconds = (roughingPath / ENDMILL_FEED + detailPath / VBIT_FEED) * 60;
  report(`B: ${label}, 900x600 sheet`, [
    `cell: ${resolution.cellMm.toFixed(4)} mm (${resolution.cappedByTexels ? 'texel-capped' : 'detail rule'}), grid ${field.cols}x${field.rows}`,
    `motions: ${field.stats.cuttingMotions} (${passes * 2} roughing + ${detailCount} detail, detail path ${(detailPath / 1000).toFixed(0)} m)`,
    `wall time: ${elapsed.toFixed(2)} s total (roughing ${seconds(roughingMs).toFixed(2)} s, detail ${seconds(detailMs).toFixed(2)} s)`,
    `gate: one-shot <= 10 s -> ${elapsed <= 10 ? 'PASS' : 'FAIL'}`,
    `dirty cells: ${(field.stats.dirtyCells / 1e6).toFixed(2)} M, removed volume ${(field.stats.removedVolumeMm3 / 1000).toFixed(1)} cm3 (endmill ${(field.stats.stageRemovedMm3[0] / 1000).toFixed(1)}, vbit ${(field.stats.stageRemovedMm3[1] / 1000).toFixed(1)})`,
    `model cutting time: ${(modelSeconds / 60).toFixed(1)} min -> sustained playback <= ${(modelSeconds / elapsed).toFixed(0)}x real time`,
    `gate: >= 15x -> ${modelSeconds / elapsed >= 15 ? 'PASS' : 'FAIL'}`,
    `heap ${(before.heapUsed / 1e6).toFixed(0)} -> ${(after.heapUsed / 1e6).toFixed(0)} MB, rss ${(after.rss / 1e6).toFixed(0)} MB`,
  ]);
};

runFlower(300_000, 'flower-like detail load');
runFlower(700_000, 'deliberate worst case (2-4x flower detail)');
