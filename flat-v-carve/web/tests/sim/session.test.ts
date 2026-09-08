import { describe, expect, it } from 'vitest';
import { applyMotion, createField, normalizeTool } from '../../src/sim/engine';
import type { StockField, StockRect, Tool } from '../../src/sim/engine';
import { SimulationSession } from '../../src/sim/session';
import { buildStore } from '../../src/sim/store';
import type { Motion } from '../../src/contracts/planning';

const stock: StockRect = { x0: 0, y0: 0, x1: 60, y1: 40, thicknessMm: 8 };
const tools: Tool[] = [
  normalizeTool({ kind: 'endmill', diameterMm: 4 }),
  normalizeTool({ kind: 'vbit', includedAngleDeg: 60, tipDiameterMm: 0.5, maxCuttingDiameterMm: 6, cuttingHeightMm: 5 }),
];
const resolution = { cellMm: 0.25, cappedByTexels: false, cappedByBudget: false };

let nextId = 0;
const motion = (kind: Motion['kind'], tool: 0 | 1, x0: number, y0: number, z0: number, x1: number, y1: number, z1: number): Motion => ({
  id: nextId++,
  tool_id: tool === 0 ? 'endmill' : 'vbit',
  operation_id: 'carve',
  layer: 0,
  kind,
  start: { x: x0, y: y0, z: z0 },
  end: { x: x1, y: y1, z: z1 },
  feed_mm_min: tool === 0 ? 1200 : 600,
});

// A deterministic script that crosses tile and interval boundaries: an
// endmill serpentine, a ramp, and V-bit detail passes.
function script(): Motion[] {
  const motions: Motion[] = [];
  for (let pass = 0; pass < 8; pass++) {
    const y = 5 + pass * 3;
    const forward = pass % 2 === 0;
    motions.push(motion(pass === 0 ? 'plunge' : 'cut', 0, forward ? 5 : 55, y, -1.5, forward ? 55 : 5, y, -1.5));
  }
  motions.push(motion('ramp', 0, 10, 30, -0.5, 50, 35, -2.5));
  let x = 8;
  let y = 20;
  let depth = -0.8;
  let seed = 42;
  const random = () => (seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0) / 2 ** 32;
  for (let index = 0; index < 80; index++) {
    const angle = random() * Math.PI * 2;
    const length = 1 + random() * 5;
    const nx = Math.min(55, Math.max(5, x + Math.cos(angle) * length));
    const ny = Math.min(35, Math.max(5, y + Math.sin(angle) * length));
    const nz = Math.min(-0.4, Math.max(-3, depth + (random() - 0.5) * 0.8));
    motions.push(motion('cut', 1, x, y, depth, nx, ny, nz));
    x = nx;
    y = ny;
    depth = nz;
  }
  return motions;
}

const buildSession = (motions: Motion[], interval = 4, cap = DEFAULT_CAP) => {
  const store = buildStore(motions, {
    toolIndex: toolId => (toolId === 'endmill' ? 0 : toolId === 'vbit' ? 1 : undefined),
    assumedFeedMmMin: 1000,
  });
  return new SimulationSession(stock, tools, resolution, store, { intervalMotions: interval, logByteCap: cap });
};

const DEFAULT_CAP = 64 * 1024 * 1024;

/** Version-independent digest of cut cells; tiles without cuts are pristine,
 * whether never allocated or released by an undo to all-zero contents. */
function stateDigest(field: StockField): string {
  let hash = 2166136261;
  const mix = (value: number) => {
    hash = Math.imul(hash ^ ((value >>> 0) & 0xffff), 16777619) >>> 0;
    hash = Math.imul(hash ^ ((value >>> 16) & 0xffff), 16777619) >>> 0;
  };
  for (let tile = 0; tile < field.heights.length; tile++) {
    const heights = field.heights[tile];
    const owners = field.cellOwner[tile];
    let cut = false;
    if (heights !== undefined && owners !== undefined) {
      for (let local = 0; local < heights.length; local++) {
        if (heights[local] === 0) continue;
        if (!cut) { mix(tile); cut = true; }
        mix(local);
        mix(heights[local]);
        mix(owners[local]);
      }
    }
    if (!cut) mix(-1);
  }
  return hash.toString(16);
}

describe('simulation session seeks', () => {
  it('reaches the same end state as one-shot application', () => {
    const motions = script();
    const session = buildSession(motions);
    const result = session.seekToEnd();
    const reference = createField(stock, tools, resolution);
    for (const m of motions) applyMotion(reference, { kind: m.kind, tool: m.tool_id === 'endmill' ? 0 : 1, x0: m.start.x, y0: m.start.y, z0: m.start.z, x1: m.end.x, y1: m.end.y, z1: m.end.z });
    expect(stateDigest(session.field)).toBe(stateDigest(reference));
    expect(result.applied).toBe(motions.length - 1);
    expect(result.fraction).toBe(1);
    expect(result.stats.dirtyCells).toBe(reference.stats.dirtyCells);
  });

  it('matches direct seeks when stepping incrementally', () => {
    const motions = script();
    const stepped = buildSession(motions);
    const direct = buildSession(motions);
    for (let index = 0; index < motions.length; index += 7) {
      stepped.seek(index, 0);
      direct.seek(index, 0);
      expect(stateDigest(stepped.field)).toBe(stateDigest(direct.field));
    }
  });

  it('applies partial fractions progressively', () => {
    const motions = script();
    const session = buildSession(motions);
    session.seek(3, 0.5);
    const half = buildSession(motions);
    half.seek(3, 0.5);
    expect(stateDigest(session.field)).toBe(stateDigest(half.field));
    session.seek(3, 1);
    const full = buildSession(motions);
    full.seek(3, 1);
    expect(stateDigest(session.field)).toBe(stateDigest(full.field));
    expect(session.field.stats.dirtyCells).toBeGreaterThan(0);
  });

  it('rewinds to any position and matches a fresh session', () => {
    const motions = script();
    const session = buildSession(motions);
    session.seekToEnd();
    for (const fraction of [0.999, 0.5, 0]) {
      for (const index of [motions.length - 1, 60, 24, 12, 4, 0]) {
        const result = session.seek(index, fraction);
        expect(result.applied).toBe(index);
        const fresh = buildSession(motions);
        const freshResult = fresh.seek(index, fraction);
        if (stateDigest(session.field) !== stateDigest(fresh.field)) {
          throw new Error(`rewind mismatch at index=${index} fraction=${fraction}`);
        }
        // Stage attribution must survive the rewind exactly: undo entries
        // snapshot the incremental statistics rather than re-deriving them
        // from final cell owners.
        expect(result.stats.dirtyCells).toBe(freshResult.stats.dirtyCells);
        expect(Math.abs(result.stats.removedVolumeMm3 - freshResult.stats.removedVolumeMm3)).toBeLessThan(1e-9);
        expect(Math.abs(result.stats.stageRemovedMm3[0] - freshResult.stats.stageRemovedMm3[0])).toBeLessThan(1e-9);
        expect(Math.abs(result.stats.stageRemovedMm3[1] - freshResult.stats.stageRemovedMm3[1])).toBeLessThan(1e-9);
      }
    }
  });

  it('falls back to a pristine replay when the undo log is evicted', () => {
    const motions = script();
    // Cap smaller than one saved tile pair, so every entry is evicted.
    const session = buildSession(motions, 4, 1024);
    session.seekToEnd();
    expect(session.field.stats.dirtyCells).toBeGreaterThan(0);
    const fresh = buildSession(motions);
    fresh.seek(10, 0.5);
    const rewind = session.seek(10, 0.5);
    expect(stateDigest(session.field)).toBe(stateDigest(fresh.field));
    // The pristine rebuild must emit tile deltas so the display clears the
    // cuts that are no longer present.
    expect(rewind.tiles.length).toBeGreaterThan(0);
    expect(rewind.stats.dirtyCells).toBe(fresh.field.stats.dirtyCells);
  });

  it('emits tile deltas only for changed tiles', () => {
    const session = buildSession(script());
    const first = session.seek(5, 0);
    expect(first.tiles.length).toBeGreaterThan(0);
    const again = session.seek(5, 0);
    expect(again.tiles).toHaveLength(0);
    const forward = session.seek(30, 0);
    expect(forward.tiles.length).toBeGreaterThan(0);
    const back = session.seek(0, 0);
    expect(back.tiles.length).toBeGreaterThan(0);
  });

  it('handles empty motion stores', () => {
    const session = buildSession([]);
    const result = session.seek(0, 0);
    expect(result.applied).toBe(0);
    expect(result.tiles).toHaveLength(0);
  });
});
