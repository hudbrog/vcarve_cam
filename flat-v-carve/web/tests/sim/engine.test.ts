import { describe, expect, it } from 'vitest';
import {
  allocatedTiles,
  applyMotion,
  applyMotions,
  beginEpoch,
  checksum,
  chooseResolution,
  createField,
  depthAt,
  normalizeTool,
  ownerAt,
} from '../../src/sim/engine';
import type { MotionKindName, SimMotion, StockField, StockRect, Tool } from '../../src/sim/engine';

const stock = (x1 = 100, y1 = 80, thicknessMm = 10): StockRect => ({ x0: 0, y0: 0, x1, y1, thicknessMm });
const field = (tools: Tool[], cellMm: number, rect: StockRect = stock()): StockField =>
  createField(rect, tools, { cellMm, cappedByTexels: false, cappedByBudget: false });
const endmill4 = (): Tool => normalizeTool({ kind: 'endmill', diameterMm: 4 });
const endmill2 = (): Tool => normalizeTool({ kind: 'endmill', diameterMm: 2 });
const vbit60 = (): Tool =>
  normalizeTool({ kind: 'vbit', includedAngleDeg: 60, tipDiameterMm: 0.5, maxCuttingDiameterMm: 6, cuttingHeightMm: 5 });

const motion = (tool: number, x0: number, y0: number, z0: number, x1: number, y1: number, z1: number,
  kind: MotionKindName = 'cut'): SimMotion => ({ kind, tool, x0, y0, z0, x1, y1, z1 });

describe('tool normalization', () => {
  it('accepts valid specs and derives consistent cone geometry', () => {
    const flat = normalizeTool({ kind: 'endmill', diameterMm: 4 });
    expect(flat).toEqual({ kind: 'endmill', radiusMm: 2 });
    const cone = vbit60();
    if (cone.kind !== 'vbit') throw new Error('expected vbit');
    expect(cone.tipRadiusMm).toBeCloseTo(0.25, 10);
    expect(cone.slope).toBeCloseTo(Math.tan(Math.PI / 6), 10);
    // The 6 mm max diameter is tighter than the 5 mm cutting height allows.
    expect(cone.maxRadiusMm).toBeCloseTo(3, 10);
    expect(cone.maxDepthBelowTipMm).toBeCloseTo(2.75 / Math.tan(Math.PI / 6), 9);
  });
  it('rejects inconsistent or non-positive geometry', () => {
    expect(() => normalizeTool({ kind: 'endmill', diameterMm: 0 })).toThrow(RangeError);
    expect(() => normalizeTool({ kind: 'vbit', includedAngleDeg: 0, tipDiameterMm: 1, maxCuttingDiameterMm: 2, cuttingHeightMm: 1 })).toThrow(RangeError);
    expect(() => normalizeTool({ kind: 'vbit', includedAngleDeg: 60, tipDiameterMm: 3, maxCuttingDiameterMm: 2, cuttingHeightMm: 1 })).toThrow(RangeError);
  });
});

describe('resolution selection', () => {
  it('uses the detail rule within caps and flags texel-cap coarsening', () => {
    const fine = chooseResolution({ x0: 0, y0: 0, x1: 100, y1: 80 }, 1);
    expect(fine).toEqual({ cellMm: 0.1, cappedByTexels: false, cappedByBudget: false });
    const detail = chooseResolution({ x0: 0, y0: 0, x1: 100, y1: 80 }, 0.2);
    expect(detail.cellMm).toBeCloseTo(0.05, 10);
    const sheet = chooseResolution({ x0: 0, y0: 0, x1: 900, y1: 600 }, 0.2);
    expect(sheet.cellMm).toBeCloseTo(900 / 8192, 10);
    expect(sheet.cappedByTexels).toBe(true);
    expect(sheet.cappedByBudget).toBe(false);
  });
  it('coarsens to the dirty-cell budget when the sheet is huge', () => {
    const squeezed = chooseResolution({ x0: 0, y0: 0, x1: 900, y1: 600 }, 0.2, 8192, 1_000_000);
    expect(squeezed.cellMm).toBeCloseTo(Math.sqrt((900 * 600) / 1_000_000), 10);
    expect(squeezed.cappedByBudget).toBe(true);
  });
  it('rejects degenerate stock rectangles and limits', () => {
    expect(() => chooseResolution({ x0: 0, y0: 0, x1: 0, y1: 80 }, 1)).toThrow(RangeError);
    expect(() => chooseResolution({ x0: 0, y0: 0, x1: 100, y1: 80 }, 1, 0)).toThrow(RangeError);
  });
});

describe('endmill sweeps', () => {
  it('plunges a disc whose cell count and volume match the circle', () => {
    const f = field([endmill4()], 0.5);
    applyMotion(f, motion(0, 50, 40, -1, 50, 40, -1, 'plunge'));
    const expectedCells = (Math.PI * 4) / 0.25;
    expect(f.stats.dirtyCells).toBeGreaterThan(expectedCells * 0.85);
    expect(f.stats.dirtyCells).toBeLessThan(expectedCells * 1.15);
    expect(f.stats.removedVolumeMm3).toBeGreaterThan(Math.PI * 4 * 0.85);
    expect(f.stats.removedVolumeMm3).toBeLessThan(Math.PI * 4 * 1.15);
    const col = Math.round(50 / 0.5 - 0.5);
    const row = Math.round(40 / 0.5 - 0.5);
    expect(depthAt(f, col, row)).toBeCloseTo(1, 3);
    expect(depthAt(f, col + 30, row)).toBe(0);
    expect(ownerAt(f, col, row)).toBe(1);
  });
  it('cuts a capsule with the expected swept volume', () => {
    const f = field([endmill4()], 0.5);
    applyMotion(f, motion(0, 10, 10, -1, 30, 10, -1));
    const area = Math.PI * 4 + 4 * 20;
    expect(f.stats.removedVolumeMm3).toBeGreaterThan(area * 0.85);
    expect(f.stats.removedVolumeMm3).toBeLessThan(area * 1.15);
  });
  it('never cuts above the stock top or below the stock bottom', () => {
    const f = field([endmill4()], 0.5);
    applyMotion(f, motion(0, 10, 10, 2, 30, 10, 2));
    expect(f.stats.dirtyCells).toBe(0);
    applyMotion(f, motion(0, 10, 10, -50, 30, 10, -50));
    const deep = depthAt(f, Math.round(20 / 0.5 - 0.5), Math.round(10 / 0.5 - 0.5));
    expect(deep).toBeCloseTo(10, 6);
  });
  it('bounds the exact ramp surface between midpoint and deep-end stamping', () => {
    const rect = stock(80, 60);
    const cell = 0.25;
    const run = motion(0, 20, 40, -0.2, 50, 40, -2.6, 'ramp');
    const exact = field([endmill2()], cell, rect);
    applyMotion(exact, run);
    const midpoint = field([endmill2()], cell, rect);
    const deepEnd = field([endmill2()], cell, rect);
    const steps = 600;
    for (let index = 0; index < steps; index++) {
      const t0 = index / steps;
      const t1 = (index + 1) / steps;
      const zMid = -0.2 + (t0 + t1) / 2 * -2.4;
      const zDeep = -0.2 + t1 * -2.4;
      const x0 = 20 + 30 * t0;
      const x1 = 20 + 30 * t1;
      applyMotion(midpoint, motion(0, x0, 40, zMid, x1, 40, zMid));
      applyMotion(deepEnd, motion(0, x0, 40, zDeep, x1, 40, zDeep));
    }
    for (let row = Math.round((40 - 2) / cell); row <= Math.round((40 + 2) / cell); row++) {
      for (let col = Math.round((18) / cell); col <= Math.round((52) / cell); col++) {
        const value = depthAt(exact, col, row);
        expect(value).toBeGreaterThanOrEqual(depthAt(midpoint, col, row) - 0.01);
        expect(value).toBeLessThanOrEqual(depthAt(deepEnd, col, row) + 0.01);
      }
    }
  });
});

describe('V-bit sweeps', () => {
  it('plunges the exact truncated-cone profile', () => {
    const cell = 0.05;
    const f = field([vbit60()], cell, stock(40, 30));
    applyMotion(f, motion(0, 20, 15, -3, 20, 15, -3, 'plunge'));
    const slope = Math.tan(Math.PI / 6);
    const samples = [[20.025, 15.025], [20.525, 15.025], [21.025, 15.025], [22.025, 15.025], [18.975, 15.025], [20.025, 17.975]];
    for (const [x, y] of samples) {
      const col = Math.floor(x / cell);
      const row = Math.floor(y / cell);
      const cx = (col + 0.5) * cell;
      const cy = (row + 0.5) * cell;
      const dist = Math.hypot(cx - 20, cy - 15);
      // The cone widens upward, so the cut is deepest at the tip flat and
      // shallower with distance; beyond tipR + depth*slope nothing is cut.
      const expected = Math.max(0, 3 - Math.max(0, dist - 0.25) / slope);
      expect(Math.abs(depthAt(f, col, row) - expected)).toBeLessThan(0.001);
    }
  });
  it('clamps through-thickness plunges to the stock bottom', () => {
    const cell = 0.1;
    const f = field([vbit60()], cell);
    applyMotion(f, motion(0, 50, 40, -10, 50, 40, -10, 'plunge'));
    expect(depthAt(f, Math.round(50 / cell - 0.5), Math.round(40 / cell - 0.5))).toBeCloseTo(10, 6);
  });
  it('attributes cells to the deepest passing role and keeps ties stable', () => {
    const cell = 0.25;
    const f = field([endmill4(), vbit60()], cell);
    applyMotion(f, motion(0, 50, 40, -1, 50, 40, -1, 'plunge'));
    expect(ownerAt(f, 199, 159)).toBe(1);
    // An equal-depth V-bit pass does not re-own the cell.
    applyMotion(f, motion(1, 50, 40, -1, 50, 40, -1, 'plunge'));
    expect(ownerAt(f, 199, 159)).toBe(1);
    expect(f.stats.stageRemovedMm3[1]).toBe(0);
    // A deeper V-bit plunge takes ownership of what it additionally removes.
    applyMotion(f, motion(1, 50, 40, -1.5, 50, 40, -1.5, 'plunge'));
    expect(ownerAt(f, 199, 159)).toBe(2);
    expect(f.stats.stageRemovedMm3[1]).toBeGreaterThan(0);
  });
});

describe('motion application semantics', () => {
  it('skips non-cutting kinds but counts them as applied', () => {
    const f = field([endmill4()], 0.5);
    applyMotion(f, motion(0, 10, 10, -1, 30, 10, -1, 'rapid_x_y'));
    applyMotion(f, motion(0, 10, 10, -1, 30, 10, -1, 'approach'));
    applyMotion(f, motion(0, 10, 10, 3, 10, 10, 3, 'rapid_retract'));
    expect(f.stats).toMatchObject({ appliedMotions: 3, cuttingMotions: 0, dirtyCells: 0 });
  });
  it('rejects unknown tools and non-finite coordinates', () => {
    const f = field([endmill4()], 0.5);
    expect(() => applyMotion(f, motion(5, 0, 0, -1, 1, 0, -1))).toThrow(RangeError);
    expect(() => applyMotion(f, motion(0, Number.NaN, 0, -1, 1, 0, -1))).toThrow(RangeError);
  });
  it('applies a partial segment identically to the clipped motion', () => {
    for (const run of [
      motion(0, 10, 10, -1, 30, 10, -1),
      motion(0, 10, 40, -0.2, 40, 40, -2.6, 'ramp'),
      motion(1, 10, 10, -0.4, 25, 18, -1.8, 'ramp'),
    ]) {
      const tools = [endmill4(), vbit60()];
      const partial = field(tools, 0.25);
      applyMotion(partial, run, 0.25, 0.75);
      const clipped = field(tools, 0.25);
      applyMotion(clipped, {
        ...run,
        x0: run.x0 + (run.x1 - run.x0) * 0.25,
        y0: run.y0 + (run.y1 - run.y0) * 0.25,
        z0: run.z0 + (run.z1 - run.z0) * 0.25,
        x1: run.x0 + (run.x1 - run.x0) * 0.75,
        y1: run.y0 + (run.y1 - run.y0) * 0.75,
        z1: run.z0 + (run.z1 - run.z0) * 0.75,
      });
      expect(checksum(partial)).toBe(checksum(clipped));
      expect(partial.stats).toEqual(clipped.stats);
    }
  });
  it('reports each tile once per epoch before its first change', () => {
    const f = field([endmill4()], 0.5);
    const events: number[] = [];
    f.onTileChange = tile => events.push(tile);
    applyMotion(f, motion(0, 50, 40, -1, 50, 40, -1, 'plunge'));
    const firstEpoch = events.length;
    expect(firstEpoch).toBeGreaterThan(0);
    expect(new Set(events).size).toBe(firstEpoch);
    applyMotion(f, motion(0, 50, 40, -2, 50, 40, -2, 'plunge'));
    expect(events.length).toBe(firstEpoch);
    beginEpoch(f);
    applyMotion(f, motion(0, 50, 40, -3, 50, 40, -3, 'plunge'));
    expect(events.length).toBe(firstEpoch * 2);
    expect(new Set(events.slice(firstEpoch))).toEqual(new Set(events.slice(0, firstEpoch)));
  });
  it('captures pre-change tile state when the hook fires', () => {
    const f = field([endmill4()], 0.5);
    applyMotion(f, motion(0, 50, 40, -1, 50, 40, -1, 'plunge'));
    const saved = new Map<number, Uint16Array>();
    beginEpoch(f);
    f.onTileChange = tile => saved.set(tile, f.heights[tile]!.slice());
    applyMotion(f, motion(0, 50, 40, -2, 50, 40, -2, 'plunge'));
    expect(saved.size).toBeGreaterThan(0);
    for (const [tile, snapshot] of saved) {
      const live = f.heights[tile]!;
      expect(live).not.toEqual(snapshot);
      expect(snapshot.some(level => level > 0)).toBe(true);
    }
  });
  it('is deterministic for a repeated mixed workload', () => {
    const tools = [endmill4(), vbit60()];
    let seed = 123456789;
    const random = () => (seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0) / 2 ** 32;
    const script: SimMotion[] = [];
    for (let index = 0; index < 400; index++) {
      const tool = random() < 0.5 ? 0 : 1;
      script.push(motion(tool, 5 + random() * 90, 5 + random() * 70, -(0.2 + random() * 3),
        5 + random() * 90, 5 + random() * 70, -(0.2 + random() * 3),
        random() < 0.3 ? 'ramp' : 'cut'));
    }
    const first = field(tools, 0.2);
    applyMotions(first, script);
    const second = field(tools, 0.2);
    applyMotions(second, script);
    expect(checksum(first)).toBe(checksum(second));
    expect(first.stats).toEqual(second.stats);
    expect(allocatedTiles(first)).toBe(allocatedTiles(second));
  });
});

describe('throughput floor', () => {
  it('sustains at least ten times the 15x worst-case cell rate', () => {
    const cell = 0.1;
    const f = field([endmill4()], cell, stock(60, 40));
    const passes = 20;
    const laps = 10;
    const length = 56;
    const run = (lap: number) => {
      for (let pass = 0; pass < passes; pass++) {
        const y = 2 + pass * 2;
        const forward = pass % 2 === 0;
        applyMotion(f, motion(0, forward ? 2 : 58, y, -1.5, forward ? 58 : 2, y, -1.5, lap === 0 ? 'cut' : 'cut'));
      }
    };
    const started = performance.now();
    for (let lap = 0; lap < laps; lap++) run(lap);
    const elapsedSeconds = (performance.now() - started) / 1000;
    // AABB cells visited per pass: (2r + length) * 2r / cell^2.
    const visits = passes * laps * ((4 + length) * 4) / (cell * cell);
    const rate = visits / elapsedSeconds;
    // The U8 plan requires 450k cell updates/s at 15x real time; hold a 10x
    // margin so slower CI runners cannot flake here.
    expect(rate).toBeGreaterThan(4_500_000);
  });
});
