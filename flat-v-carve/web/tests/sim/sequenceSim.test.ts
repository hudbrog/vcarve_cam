// D3 sequence stock/timeline model: store building (kind mapping, tool and
// operation indices), sweep application with per-operation ownership,
// checkpoint capture/restore, and motion paging.
import { describe, expect, it } from 'vitest';
import { depthAt } from '../../src/sim/engine';
import {
  applyRange, buildSequenceStore, captureCheckpoint, createSequenceField,
  pageAllMotions, probeDepth, restoreCheckpoint, sequenceTools,
} from '../../src/sim/sequenceSim';
import { KIND_APPROACH, KIND_CUT, KIND_PLUNGE, KIND_RAMP, KIND_RAPID_RETRACT, KIND_RAPID_XY } from '../../src/sim/store';
import { plannedMotionSchema, type PlannedMotion } from '../../src/contracts/sequence';

const job = {
  tools: [
    { id: 't1', geometry: { kind: 'endmill', dimensions: { diameter_mm: 4, cutting_length_mm: 12 } } },
    { id: 't2', geometry: { kind: 'vbit', dimensions: { included_angle_deg: 90, tip_diameter_mm: 1, max_cutting_diameter_mm: 12, cutting_height_mm: 5 } } },
    { id: 'k1', geometry: { kind: 'drag_knife', dimensions: { blade_offset_mm: 1, max_cut_depth_mm: 1 } } },
  ],
};

function motion(
  index: number, operationId: string, toolId: string,
  purpose: PlannedMotion['purpose'], effect: PlannedMotion['effect'],
  interpolation: PlannedMotion['interpolation'],
  start: [number, number, number], end: [number, number, number],
): PlannedMotion {
  return {
    id: index, operationId, stageId: `${operationId}-s`, toolId, contourId: null, passId: 0, layer: 0,
    interpolation, purpose, effect,
    start: { x: start[0], y: start[1], z: start[2] },
    end: { x: end[0], y: end[1], z: end[2] },
    feedMmMin: interpolation === 'linear_feed' ? 300 : null,
  };
}

function fixtureMotions(): PlannedMotion[] {
  return [
    // Operation A: plunge at (5, 10), cut a slot to (30, 10) at depth 2.
    motion(0, 'op-a', 't1', 'clearance', 'none', 'rapid', [0, 0, 5], [5, 10, 5]),
    motion(1, 'op-a', 't1', 'entry', 'milling_sweep', 'linear_feed', [5, 10, 0], [5, 10, -2]),
    motion(2, 'op-a', 't1', 'rough', 'milling_sweep', 'linear_feed', [5, 10, -2], [30, 10, -2]),
    motion(3, 'op-a', 't1', 'clearance', 'none', 'rapid', [30, 10, -2], [30, 10, 5]),
    // Operation B: ramp entry, then a short shallower cut.
    motion(4, 'op-b', 't1', 'entry', 'milling_sweep', 'linear_feed', [15, 20, 0], [15, 21, -1]),
    motion(5, 'op-b', 't1', 'rough', 'milling_sweep', 'linear_feed', [15, 21, -1], [15, 25, -1]),
    // A knife trace never removes stock in the heightfield model.
    motion(6, 'op-b', 'k1', 'knife_cut', 'knife_trace', 'linear_feed', [15, 25, -1], [16, 25, -1]),
  ];
}

describe('sequence store', () => {
  it('maps motion purposes onto engine kinds and dense indices', () => {
    const tools = sequenceTools(job);
    expect(tools.ids).toEqual(['t1', 't2']);
    const store = buildSequenceStore(fixtureMotions(), tools, ['op-a', 'op-b']);
    expect([...store.kind]).toEqual([
      KIND_RAPID_XY, KIND_PLUNGE, KIND_CUT, KIND_RAPID_RETRACT, KIND_RAMP, KIND_CUT, KIND_APPROACH,
    ]);
    expect([...store.opIndex]).toEqual([0, 0, 0, 0, 1, 1, 1]);
    expect([...store.tool]).toEqual([0, 0, 0, 0, 0, 0, 255]);
    // store.tool 255 is the knife (outside the milling field) and only ever
    // appears on non-cutting motions.
    expect(store.kind[6]).toBe(KIND_APPROACH);
  });

  it('rejects cutting motions that reference non-milling tools', () => {
    const tools = sequenceTools(job);
    const bad = motion(0, 'op-a', 'k1', 'rough', 'milling_sweep', 'linear_feed', [0, 0, -1], [1, 0, -1]);
    expect(() => buildSequenceStore([bad], tools, ['op-a'])).toThrow(/unknown tool 'k1'/);
    expect(() => buildSequenceStore(fixtureMotions(), tools, ['op-a'])).toThrow(/outside the plan/);
  });
});

describe('sequence stock application', () => {
  const tools = sequenceTools(job);
  const operations = ['op-a', 'op-b'];

  function built() {
    const store = buildSequenceStore(fixtureMotions(), tools, operations);
    const state = createSequenceField(
      { x0: 0, y0: 0, x1: 40, y1: 30, thicknessMm: 8 }, store, tools,
    );
    return { store, state };
  }

  it('tracks per-operation ownership and depth', () => {
    const { store, state } = built();
    applyRange(state, 0, store.count);
    const { field, opOwner } = state;
    const depth = (x: number, y: number) => depthAt(field, Math.floor(x / field.cellMm), Math.floor(y / field.cellMm));
    const owner = (x: number, y: number) => opOwner[Math.floor(y / field.cellMm) * field.cols + Math.floor(x / field.cellMm)];
    expect(depth(15, 10)).toBeCloseTo(2, 1);
    expect(owner(15, 10)).toBe(1); // op-a index 0 → owner 1
    expect(depth(15, 23)).toBeCloseTo(1, 1);
    expect(owner(15, 23)).toBe(2); // op-b
    expect(depth(5, 5)).toBe(0);
    expect(owner(5, 5)).toBe(0);
    const [volumeA, volumeB] = state.removedByOperationMm3;
    // Slot ≈ (25 mm + πr² overlap) × 2 mm deep ≈ 260 mm³; short cut ≈ 20 mm³.
    expect(volumeA).toBeGreaterThan(200);
    expect(volumeA).toBeLessThan(320);
    expect(volumeB).toBeGreaterThan(10);
    expect(volumeB).toBeLessThan(60);
  });

  it('captures checkpoints per operation and restores them exactly', () => {
    const { store, state } = built();
    applyRange(state, 0, 4);
    const afterA = captureCheckpoint(state, 4);
    expect(afterA.removedByOperationMm3[0]).toBeGreaterThan(0);
    expect(afterA.removedByOperationMm3[1] ?? 0).toBe(0);
    applyRange(state, 4, store.count);
    expect(state.removedByOperationMm3[1]).toBeGreaterThan(0);

    // Rewind to after op A: op-b's material is back, op-a's is still gone.
    restoreCheckpoint(state, afterA);
    const { field } = state;
    expect(depthAt(field, Math.floor(15 / field.cellMm), Math.floor(23 / field.cellMm))).toBe(0);
    expect(depthAt(field, Math.floor(15 / field.cellMm), Math.floor(10 / field.cellMm))).toBeCloseTo(2, 1);
    expect(state.opOwner[Math.floor(10 / field.cellMm) * field.cols + Math.floor(15 / field.cellMm)]).toBe(1);
  });

  it('probes remaining material depth with its resolution (tab inspection)', () => {
    const { store, state } = built();
    applyRange(state, 0, store.count);
    // Inside op-a's slot: ~2 mm removed; the probe reports the cell size so
    // a sub-cell tab is never mistaken for absent material.
    const slot = probeDepth(state, 15, 10);
    expect(slot).not.toBeNull();
    expect(slot!.depthMm).toBeCloseTo(2, 1);
    expect(slot!.cellMm).toBeGreaterThan(0);
    expect(probeDepth(state, 5, 5)?.depthMm).toBe(0);
    // Outside the physical stock rectangle there is no material to probe.
    expect(probeDepth(state, 45, 10)).toBeNull();
    expect(probeDepth(state, -1, 10)).toBeNull();
  });
});

describe('motion paging', () => {
  it('pages until the total is reached and stops on empty pages', async () => {
    const all = fixtureMotions();
    const request = async (offset: number) => {
      const motions = all.slice(offset, offset + 3);
      return { motions: { count: motions.length, total: all.length, motions } };
    };
    const result = await pageAllMotions(request);
    expect(result.total).toBe(all.length);
    expect(result.motions).toEqual(all);
    const stalled = await pageAllMotions(async () => ({ motions: { count: 0, total: 100, motions: [] } }));
    expect(stalled.motions).toEqual([]);
    expect(stalled.total).toBe(100);
  });
});

// F1 knife display contract: knife motions program the holder pivot; the
// visible tip derives from the blade offset and the modeled heading carried
// on the motion. Knife tools never enter the milling field.
describe('knife display contract', () => {
  it('exposes knife tool geometry for pivot/tip traces without milling it', () => {
    const tools = sequenceTools(job);
    expect(tools.knifeTools).toEqual([{ id: 'k1', bladeOffsetMm: 1, maxCutDepthMm: 1 }]);
    expect(tools.ids).toEqual(['t1', 't2']);
    expect(tools.scanRadiusMm).toEqual([2, 6]);
  });

  it('parses knife motions that carry modeled blade headings', () => {
    const parsed = plannedMotionSchema.parse({
      ...motion(0, 'knife-op', 'k1', 'knife_swivel', 'knife_trace', 'linear_feed', [21, 0, -1], [20, 1, -1]),
      bladeHeadingDeg: [180, 270],
    });
    expect(parsed.bladeHeadingDeg).toEqual([180, 270]);
    // The heading is optional data on the wire; the store never cuts with it.
    const store = buildSequenceStore([parsed], sequenceTools(job), ['knife-op']);
    expect(store.kind[0]).toBe(KIND_APPROACH);
  });

  it('derives the blade tip from the pivot, heading and offset', () => {
    // tip = pivot + offset * u(heading); the section-19.1 corner numbers.
    const tip = (x: number, y: number, headingDeg: number, offset: number): [number, number] => [
      x + offset * Math.cos(headingDeg * Math.PI / 180),
      y + offset * Math.sin(headingDeg * Math.PI / 180),
    ];
    const cornerA = tip(11, 0, 180, 1);
    const cornerB = tip(10, 1, 270, 1);
    expect(cornerA[0]).toBeCloseTo(10, 9);
    expect(cornerA[1]).toBeCloseTo(0, 9);
    expect(cornerB[0]).toBeCloseTo(10, 9);
    expect(cornerB[1]).toBeCloseTo(0, 9);
  });
});
