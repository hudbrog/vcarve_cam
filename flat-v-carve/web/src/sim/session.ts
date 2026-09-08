// Worker-side seek state machine (U8 §5.4–5.5): owns the heightfield, the
// applied prefix, and the undo log used for rewinds. Pure module on top of
// the engine so vitest can drive it directly without a real Worker.
import {
  applyMotion,
  beginEpoch,
  checksum,
  createField,
  type Resolution,
  type SimMotion,
  type StockField,
  type StockRect,
  type Tool,
} from './engine';
import type { MotionStore } from './store';

export interface TileDelta {
  /** Tile origin in cells. */
  x: number;
  y: number;
  heights: Uint16Array;
  owners: Uint8Array;
}

export interface SessionStats {
  applied: number;
  dirtyCells: number;
  removedVolumeMm3: number;
  stageRemovedMm3: [number, number];
}

export interface SeekResult {
  /** Fully applied prefix; the partial motion is `applied` itself, if any. */
  applied: number;
  /** Fraction of motion `applied` already cut (0 when none). */
  fraction: number;
  tiles: TileDelta[];
  stats: SessionStats;
}

/**
 * Saved tile state at the start of an interval. `null` marks a tile that was
 * still pristine (unallocated) and must be released again on restore.
 */
type SavedTile = { h: Uint16Array; o: Uint8Array } | null;

interface UndoEntry {
  /** Motion index at the start of the interval this entry rewinds to. */
  index: number;
  tiles: Map<number, SavedTile>;
  bytes: number;
  /** Incremental statistics at interval start (see restoreStats). */
  stats: { dirtyCells: number; removedVolumeMm3: number; stage: [number, number] };
}

export interface SessionOptions {
  intervalMotions: number;
  logByteCap: number;
}

export const DEFAULT_INTERVAL_MOTIONS = 2048;
export const DEFAULT_LOG_BYTE_CAP = 64 * 1024 * 1024;

const KIND_NAME: readonly SimMotion['kind'][] = ['cut', 'plunge', 'ramp', 'rapid_x_y', 'rapid_retract', 'approach'];

/**
 * Seeks are idempotent: `seek(i, f)` leaves the field at exactly
 * "motions [0, i) fully applied plus fraction f of motion i", regardless of
 * the previous position. Forward seeks apply the delta; backward seeks undo
 * log intervals and re-apply — re-application is a monotone minimum, so the
 * extra replay of one interval is harmless and keeps the log simple.
 */
export class SimulationSession {
  readonly field: StockField;
  private readonly store: MotionStore;
  private readonly interval: number;
  private readonly logCap: number;
  private readonly logs: UndoEntry[] = [];
  private logBytes = 0;
  private applied = 0;
  private fraction = 0;
  private readonly emitted: Uint32Array;
  private readonly scratch: SimMotion;

  constructor(stock: StockRect, tools: Tool[], resolution: Resolution, store: MotionStore, options?: Partial<SessionOptions>) {
    this.field = createField(stock, tools, resolution);
    this.store = store;
    this.interval = Math.max(1, options?.intervalMotions ?? DEFAULT_INTERVAL_MOTIONS);
    this.logCap = options?.logByteCap ?? DEFAULT_LOG_BYTE_CAP;
    this.emitted = new Uint32Array(this.field.heights.length);
    this.scratch = { kind: 'cut', tool: 0, x0: 0, y0: 0, z0: 0, x1: 0, y1: 0, z1: 0 };
    this.field.onTileChange = tile => this.capture(tile);
    this.pushLog(0);
  }

  get position(): { applied: number; fraction: number } {
    return { applied: this.applied, fraction: this.fraction };
  }

  private pushLog(index: number): void {
    const stats = this.field.stats;
    this.logs.push({
      index,
      tiles: new Map(),
      bytes: 0,
      stats: {
        dirtyCells: stats.dirtyCells,
        removedVolumeMm3: stats.removedVolumeMm3,
        stage: [stats.stageRemovedMm3[0], stats.stageRemovedMm3[1]],
      },
    });
    beginEpoch(this.field);
  }

  private trimLog(): void {
    // Keep at least the newest entry; rewinding below the oldest retained
    // entry falls back to a pristine replay, which is the same bound.
    while (this.logBytes > this.logCap && this.logs.length > 1) {
      const dropped = this.logs.shift()!;
      this.logBytes -= dropped.bytes;
    }
  }

  private capture(tile: number): void {
    const entry = this.logs[this.logs.length - 1];
    if (entry === undefined || entry.tiles.has(tile)) return;
    const heights = this.field.heights[tile];
    const owners = this.field.cellOwner[tile];
    if (heights === undefined || owners === undefined) {
      entry.tiles.set(tile, null); // pristine: restore must release it again
      return;
    }
    const saved = { h: heights.slice(), o: owners.slice() };
    entry.tiles.set(tile, saved);
    entry.bytes += saved.h.byteLength + saved.o.byteLength;
    this.logBytes += saved.h.byteLength + saved.o.byteLength;
    this.trimLog();
  }

  private applyRange(from: number, to: number): void {
    const scratch = this.scratch;
    const store = this.store;
    for (let index = from; index < to; index++) {
      if (index > 0 && index === this.applied && index % this.interval === 0) this.pushLog(index);
      scratch.kind = KIND_NAME[store.kind[index]];
      scratch.tool = store.tool[index];
      scratch.x0 = store.x0[index];
      scratch.y0 = store.y0[index];
      scratch.z0 = store.z0[index];
      scratch.x1 = store.x1[index];
      scratch.y1 = store.y1[index];
      scratch.z1 = store.z1[index];
      applyMotion(this.field, scratch);
      this.applied = index + 1;
    }
  }

  private applyPartial(index: number, from: number, to: number): void {
    if (to <= from) return;
    const scratch = this.scratch;
    const store = this.store;
    scratch.kind = KIND_NAME[store.kind[index]];
    scratch.tool = store.tool[index];
    scratch.x0 = store.x0[index];
    scratch.y0 = store.y0[index];
    scratch.z0 = store.z0[index];
    scratch.x1 = store.x1[index];
    scratch.y1 = store.y1[index];
    scratch.z1 = store.z1[index];
    applyMotion(this.field, scratch, from, to);
  }

  /**
   * Restore the field to the interval boundary at or before `index` and
   * return that boundary. The retained base entry keeps its own copies so it
   * stays valid for later rewinds; the restore writes fresh slices.
   */
  private rewindToBoundary(index: number): number {
    while (this.logs.length > 1 && this.logs[this.logs.length - 1].index > index) {
      const entry = this.logs.pop()!;
      for (const [tile, saved] of entry.tiles) this.restoreTile(tile, saved);
      this.logBytes -= entry.bytes;
    }
    const base = this.logs[this.logs.length - 1];
    if (base.index > index) {
      // Every retained entry starts after the target: rebuild from pristine.
      // Cleared tiles must count as changed so collect() emits them and the
      // display drops the old cuts.
      for (let tile = 0; tile < this.field.heights.length; tile++) {
        if (this.field.heights[tile] === undefined) continue;
        this.field.heights[tile] = undefined;
        this.field.cellOwner[tile] = undefined;
        this.field.tileVersions[tile]++;
      }
      this.field.stats.dirtyCells = 0;
      this.field.stats.removedVolumeMm3 = 0;
      this.field.stats.stageRemovedMm3 = [0, 0];
      this.logs.length = 0;
      this.logBytes = 0;
      this.pushLog(0);
      return 0;
    }
    for (const [tile, saved] of base.tiles) this.restoreTile(tile, saved);
    this.restoreStats(base);
    return base.index;
  }

  /** Undo entries snapshot the incremental statistics at interval start, so a
   * rewind restores the exact attribution a forward pass would report — the
   * per-stage split cannot be reconstructed from final cell owners. */
  private restoreStats(entry: UndoEntry): void {
    this.field.stats.dirtyCells = entry.stats.dirtyCells;
    this.field.stats.removedVolumeMm3 = entry.stats.removedVolumeMm3;
    this.field.stats.stageRemovedMm3 = [entry.stats.stage[0], entry.stats.stage[1]];
  }

  private restoreTile(tile: number, saved: SavedTile): void {
    if (saved === null) {
      this.field.heights[tile] = undefined;
      this.field.cellOwner[tile] = undefined;
    } else {
      this.field.heights[tile] = saved.h.slice();
      this.field.cellOwner[tile] = saved.o.slice();
    }
    this.field.tileVersions[tile]++;
  }

  seek(index: number, fraction: number): SeekResult {
    const count = this.store.count;
    if (count === 0) return this.collect();
    let targetIndex = Math.min(Math.max(index, 0), count - 1);
    let targetFraction = Math.min(Math.max(fraction, 0), 1);
    if (index >= count) {
      targetIndex = count - 1;
      targetFraction = 1;
    }
    const target = targetIndex + targetFraction;
    const current = this.applied + this.fraction;
    if (target !== current) {
      if (target < current) {
        if (this.fraction > 0) {
          // Snap to a clean prefix so the undo log records the full motion.
          this.applyPartial(this.applied, this.fraction, 1);
          this.applied++;
          this.fraction = 0;
        }
        const base = this.rewindToBoundary(targetIndex);
        this.applied = base;
        this.applyRange(base, targetIndex);
      } else {
        if (this.fraction > 0) {
          if (this.applied === targetIndex) {
            const next = Math.max(targetFraction, this.fraction);
            this.applyPartial(this.applied, this.fraction, next);
            this.fraction = next;
            return this.collect();
          }
          this.applyPartial(this.applied, this.fraction, 1);
          this.applied++;
          this.fraction = 0;
        }
        this.applyRange(this.applied, targetIndex);
      }
      this.fraction = targetFraction;
      if (targetFraction > 0) this.applyPartial(targetIndex, 0, targetFraction);
    }
    return this.collect();
  }

  /** Seek straight to the finished stock; used for the static result view. */
  seekToEnd(): SeekResult {
    return this.seek(this.store.count, 1);
  }

  checksum(): string {
    return checksum(this.field);
  }

  private collect(): SeekResult {
    const tiles: TileDelta[] = [];
    const field = this.field;
    for (let tile = 0; tile < this.emitted.length; tile++) {
      if (field.tileVersions[tile] === this.emitted[tile]) continue;
      this.emitted[tile] = field.tileVersions[tile];
      const heights = field.heights[tile];
      const owners = field.cellOwner[tile];
      const x = (tile % field.tilesX) * 256;
      const y = Math.floor(tile / field.tilesX) * 256;
      if (heights === undefined || owners === undefined) {
        // Released by a rewind: emit an empty tile to clear the display.
        tiles.push({ x, y, heights: new Uint16Array(256 * 256), owners: new Uint8Array(256 * 256) });
      } else {
        tiles.push({ x, y, heights: heights.slice(), owners: owners.slice() });
      }
    }
    const stats = field.stats;
    return {
      applied: this.applied,
      fraction: this.fraction,
      tiles,
      stats: {
        applied: stats.appliedMotions,
        dirtyCells: stats.dirtyCells,
        removedVolumeMm3: stats.removedVolumeMm3,
        stageRemovedMm3: [stats.stageRemovedMm3[0], stats.stageRemovedMm3[1]],
      },
    };
  }
}
