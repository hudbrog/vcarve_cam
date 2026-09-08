// Compact structure-of-arrays motion store with feed-based timing (U8 §5.2).
// The worker converts contract motions once and drops the objects; the main
// thread gets a clone for pose interpolation and the playback clock.
import type { Motion } from '../contracts/planning';

export const KIND_CUT = 0;
export const KIND_PLUNGE = 1;
export const KIND_RAMP = 2;
export const KIND_RAPID_XY = 3;
export const KIND_RAPID_RETRACT = 4;
export const KIND_APPROACH = 5;

const KIND_BY_NAME: Record<Motion['kind'], number> = {
  cut: KIND_CUT,
  plunge: KIND_PLUNGE,
  ramp: KIND_RAMP,
  rapid_x_y: KIND_RAPID_XY,
  rapid_retract: KIND_RAPID_RETRACT,
  approach: KIND_APPROACH,
};

/** True when the engine applies the sweep (material can only be lowered). */
export function kindCuts(kind: number): boolean {
  return kind === KIND_CUT || kind === KIND_PLUNGE || kind === KIND_RAMP;
}

export interface MotionStore {
  readonly count: number;
  readonly x0: Float64Array;
  readonly y0: Float64Array;
  readonly z0: Float64Array;
  readonly x1: Float64Array;
  readonly y1: Float64Array;
  readonly z1: Float64Array;
  readonly kind: Uint8Array;
  /** Index into the tool array shared with the engine field. */
  readonly tool: Uint8Array;
  readonly layer: Int32Array;
  readonly lengthMm: Float64Array;
  /** Recorded feed; motions without one use the assumed feed (counted). */
  readonly feedMmMin: Float32Array;
  readonly assumedFeedCount: number;
}

export interface StoreOptions {
  /** Maps a contract tool_id to a tool array index; must cover every motion. */
  toolIndex: (toolId: string) => number | undefined;
  /** Feed used when a motion has none; the count is reported to the UI. */
  assumedFeedMmMin: number;
}

export function buildStore(motions: readonly Motion[], options: StoreOptions): MotionStore {
  const count = motions.length;
  const x0 = new Float64Array(count);
  const y0 = new Float64Array(count);
  const z0 = new Float64Array(count);
  const x1 = new Float64Array(count);
  const y1 = new Float64Array(count);
  const z1 = new Float64Array(count);
  const kind = new Uint8Array(count);
  const tool = new Uint8Array(count);
  const layer = new Int32Array(count);
  const lengthMm = new Float64Array(count);
  const feedMmMin = new Float32Array(count);
  let assumedFeedCount = 0;
  for (let index = 0; index < count; index++) {
    const motion = motions[index];
    const toolIndex = options.toolIndex(motion.tool_id);
    if (toolIndex === undefined || toolIndex < 0 || toolIndex > 255) {
      throw new RangeError(`motion ${index} references unknown tool '${motion.tool_id}'`);
    }
    if (![motion.start.x, motion.start.y, motion.start.z, motion.end.x, motion.end.y, motion.end.z].every(Number.isFinite)) {
      throw new RangeError(`motion ${index} has non-finite coordinates`);
    }
    x0[index] = motion.start.x;
    y0[index] = motion.start.y;
    z0[index] = motion.start.z;
    x1[index] = motion.end.x;
    y1[index] = motion.end.y;
    z1[index] = motion.end.z;
    kind[index] = KIND_BY_NAME[motion.kind];
    tool[index] = toolIndex;
    layer[index] = motion.layer;
    lengthMm[index] = Math.hypot(motion.end.x - motion.start.x, motion.end.y - motion.start.y, motion.end.z - motion.start.z);
    if (motion.feed_mm_min === null) {
      feedMmMin[index] = options.assumedFeedMmMin;
      if (kindCuts(KIND_BY_NAME[motion.kind])) assumedFeedCount++;
    } else {
      feedMmMin[index] = motion.feed_mm_min;
    }
  }
  return { count, x0, y0, z0, x1, y1, z1, kind, tool, layer, lengthMm, feedMmMin, assumedFeedCount };
}

/**
 * Feed-based playback timing (display model, U8 §5.3): cutting moves take
 * their recorded length/feed, rapid and approach moves take the configured
 * rapid rate, and the cutting-time-only mode collapses rapid durations.
 */
export interface Timing {
  readonly cuttingOnly: boolean;
  readonly rapidFeedMmMin: number;
  /** Cumulative model seconds before each motion; length count + 1. */
  readonly cumulative: Float64Array;
  readonly totalSeconds: number;
}

export function buildTiming(store: MotionStore, rapidFeedMmMin: number, cuttingOnly: boolean): Timing {
  const cumulative = new Float64Array(store.count + 1);
  for (let index = 0; index < store.count; index++) {
    const cutting = kindCuts(store.kind[index]);
    const feed = cutting ? store.feedMmMin[index] : rapidFeedMmMin;
    // Multiply before dividing so whole-number feeds keep exact boundaries.
    const seconds = cutting || !cuttingOnly ? (store.lengthMm[index] * 60) / feed : 0;
    cumulative[index + 1] = cumulative[index] + seconds;
  }
  return { cuttingOnly, rapidFeedMmMin, cumulative, totalSeconds: cumulative[store.count] };
}

/** Model seconds at the boundary before motion `index`. */
export function timeOfIndex(timing: Timing, index: number): number {
  return timing.cumulative[Math.min(Math.max(index, 0), timing.cumulative.length - 1)];
}

/** Motion index and in-motion fraction for a model time; exact at boundaries. */
export function indexForTime(timing: Timing, time: number): { index: number; fraction: number } {
  const cumulative = timing.cumulative;
  const count = cumulative.length - 1;
  if (count === 0) return { index: 0, fraction: 0 };
  if (time >= cumulative[count]) return { index: count - 1, fraction: 1 };
  let lo = 0;
  let hi = count;
  while (lo + 1 < hi) {
    const mid = (lo + hi) >> 1;
    if (cumulative[mid] <= time) lo = mid; else hi = mid;
  }
  const span = cumulative[lo + 1] - cumulative[lo];
  return { index: lo, fraction: span > 0 ? (time - cumulative[lo]) / span : 1 };
}

export interface ToolPose {
  x: number;
  y: number;
  z: number;
  tool: number;
}

/** Interpolated tip pose inside motion `index` (main-thread clock only). */
export function poseAt(store: MotionStore, index: number, fraction: number): ToolPose {
  const clamped = Math.min(Math.max(fraction, 0), 1);
  const safeIndex = Math.min(Math.max(index, 0), Math.max(store.count - 1, 0));
  return {
    x: store.x0[safeIndex] + (store.x1[safeIndex] - store.x0[safeIndex]) * clamped,
    y: store.y0[safeIndex] + (store.y1[safeIndex] - store.y0[safeIndex]) * clamped,
    z: store.z0[safeIndex] + (store.z1[safeIndex] - store.z0[safeIndex]) * clamped,
    tool: store.tool[safeIndex],
  };
}
