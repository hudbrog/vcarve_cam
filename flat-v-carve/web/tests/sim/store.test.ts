import { describe, expect, it } from 'vitest';
import {
  buildStore,
  buildTiming,
  indexForTime,
  kindCuts,
  poseAt,
  timeOfIndex,
} from '../../src/sim/store';
import type { Motion } from '../../src/contracts/planning';

const motion = (overrides: Partial<Motion> = {}): Motion => ({
  id: 0,
  tool_id: 'endmill',
  operation_id: 'carve',
  layer: 0,
  kind: 'cut',
  start: { x: 0, y: 0, z: -1 },
  end: { x: 10, y: 0, z: -1 },
  feed_mm_min: 600,
  ...overrides,
});
const toolIndex = (toolId: string) => (toolId === 'endmill' ? 0 : toolId === 'vbit' ? 1 : undefined);

describe('compact motion store', () => {
  it('preserves every motion field in the arrays', () => {
    const motions = [
      motion(),
      motion({ id: 7, tool_id: 'vbit', kind: 'plunge', layer: 3, start: { x: 1, y: 2, z: -3 }, end: { x: 4, y: 5, z: -6 }, feed_mm_min: 300 }),
      motion({ kind: 'rapid_x_y', start: { x: 1, y: 1, z: 5 }, end: { x: 1, y: 4, z: 5 }, feed_mm_min: null }),
    ];
    const store = buildStore(motions, { toolIndex, assumedFeedMmMin: 1000 });
    expect(store.count).toBe(3);
    expect(store.x0).toEqual(new Float64Array([0, 1, 1]));
    expect(store.y1).toEqual(new Float64Array([0, 5, 4]));
    expect(store.z0).toEqual(new Float64Array([-1, -3, 5]));
    expect(store.kind).toEqual(new Uint8Array([0, 1, 3]));
    expect(store.tool).toEqual(new Uint8Array([0, 1, 0]));
    expect(store.layer).toEqual(new Int32Array([0, 3, 0]));
    expect(store.lengthMm[0]).toBeCloseTo(10, 6);
    expect(store.lengthMm[1]).toBeCloseTo(Math.hypot(3, 3, 3), 5);
    expect(store.assumedFeedCount).toBe(0); // the null-feed motion is a rapid
  });

  it('counts assumed feeds only for cutting motions', () => {
    const store = buildStore(
      [motion({ feed_mm_min: null }), motion({ kind: 'rapid_retract', feed_mm_min: null })],
      { toolIndex, assumedFeedMmMin: 1000 },
    );
    expect(store.assumedFeedCount).toBe(1);
    expect(store.feedMmMin).toEqual(new Float32Array([1000, 1000]));
  });

  it('rejects unknown tools and non-finite coordinates', () => {
    expect(() => buildStore([motion({ tool_id: 'mystery' })], { toolIndex, assumedFeedMmMin: 1000 })).toThrow(RangeError);
    expect(() => buildStore([motion({ start: { x: Number.NaN, y: 0, z: -1 } })], { toolIndex, assumedFeedMmMin: 1000 })).toThrow(RangeError);
  });

  it('treats only cut, plunge, and ramp as cutting kinds', () => {
    const store = buildStore(
      [motion(), motion({ kind: 'plunge' }), motion({ kind: 'ramp' }), motion({ kind: 'approach' }), motion({ kind: 'rapid_x_y' })],
      { toolIndex, assumedFeedMmMin: 1000 },
    );
    expect(Array.from(store.kind, kindCuts)).toEqual([true, true, true, false, false]);
  });
});

describe('feed-based timing', () => {
  const store = buildStore(
    [
      motion(), // 10 mm at 600 mm/min = 1 s
      motion({ kind: 'rapid_x_y', start: { x: 10, y: 0, z: 5 }, end: { x: 20, y: 0, z: 5 }, feed_mm_min: 9999 }), // 10 mm rapid
      motion({ start: { x: 20, y: 0, z: -1 }, end: { x: 50, y: 0, z: -1 }, feed_mm_min: 1800 }), // 30 mm = 1 s
    ],
    { toolIndex, assumedFeedMmMin: 1000 },
  );

  it('uses recorded feeds for cutting moves and the rapid rate otherwise', () => {
    const timing = buildTiming(store, 3000, false);
    expect(timeOfIndex(timing, 0)).toBe(0);
    expect(timeOfIndex(timing, 1)).toBeCloseTo(1, 9);
    expect(timeOfIndex(timing, 2)).toBeCloseTo(1.2, 9); // 10 mm at 3000 mm/min
    expect(timing.totalSeconds).toBeCloseTo(2.2, 9);
  });

  it('collapses rapid durations in cutting-time-only mode', () => {
    const cuttingOnly = buildTiming(store, 3000, true);
    expect(timeOfIndex(cuttingOnly, 2)).toBeCloseTo(1, 9);
    expect(cuttingOnly.totalSeconds).toBeCloseTo(2, 9);
  });

  it('maps times to indices and fractions exactly at boundaries', () => {
    const timing = buildTiming(store, 3000, false);
    expect(indexForTime(timing, 0)).toEqual({ index: 0, fraction: 0 });
    expect(indexForTime(timing, 1)).toEqual({ index: 1, fraction: 0 });
    const half = indexForTime(timing, 1.1);
    expect(half.index).toBe(1);
    expect(half.fraction).toBeCloseTo(0.5, 12);
    expect(indexForTime(timing, 99)).toEqual({ index: 2, fraction: 1 });
    const empty = buildTiming(buildStore([], { toolIndex, assumedFeedMmMin: 1000 }), 3000, false);
    expect(indexForTime(empty, 5)).toEqual({ index: 0, fraction: 0 });
    const zeroSpan = buildTiming(buildStore([motion({ end: { x: 0, y: 0, z: -1 } })], { toolIndex, assumedFeedMmMin: 1000 }), 3000, false);
    expect(indexForTime(zeroSpan, 0)).toEqual({ index: 0, fraction: 1 });
  });

  it('interpolates poses with clamping', () => {
    const store2 = buildStore([motion({ start: { x: 0, y: 2, z: -1 }, end: { x: 10, y: 4, z: -3 }, tool_id: 'vbit' })], { toolIndex, assumedFeedMmMin: 1000 });
    expect(poseAt(store2, 0, 0)).toEqual({ x: 0, y: 2, z: -1, tool: 1 });
    expect(poseAt(store2, 0, 0.5)).toEqual({ x: 5, y: 3, z: -2, tool: 1 });
    expect(poseAt(store2, 0, 1)).toEqual({ x: 10, y: 4, z: -3, tool: 1 });
    expect(poseAt(store2, 0, 2).x).toBe(10);
    expect(poseAt(store2, 0, -1).x).toBe(0);
  });
});
