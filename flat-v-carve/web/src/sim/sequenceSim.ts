// Sequence-plan stock/timeline model (plan section 15.2): consumes the
// complete ordered motion stream of a sequence plan — paged in by the
// caller — with dynamic per-operation ownership instead of the legacy
// endmill/V-bit roles. Pure module over the tiled heightfield engine so
// vitest can drive it without a Worker or canvas.
import {
  applyMotion,
  chooseResolution,
  createField,
  depthAt,
  TILE,
  type Resolution,
  type StockField,
  type StockRect,
  type ToolSpec,
  normalizeTool,
} from './engine';
import type { PlannedMotion } from '../contracts/sequence';
import { KIND_CUT, KIND_PLUNGE, KIND_RAMP, KIND_RAPID_RETRACT, KIND_RAPID_XY, KIND_APPROACH } from './store';

/** Explicit index capacities (plan section 15.2): 8-bit tools, 16-bit
 * operation ownership — never wrapped at two or 255. */
export const MAX_TOOLS = 256;
export const MAX_OPERATIONS = 65_535;
/** Beyond this many motions the display stays explicitly incomplete rather
 * than freezing the tab with a synchronous sweep. */
export const MAX_DISPLAY_MOTIONS = 200_000;

export interface SequenceTools {
  ids: string[];
  specs: ToolSpec[];
  /** Radius used for ownership scans (conservative for V-bits). */
  scanRadiusMm: number[];
  toolIndexOf: (toolId: string) => number | undefined;
}

interface RawJobTool {
  id: string;
  geometry?: { kind: string; dimensions?: Record<string, number> } | null;
}

/** Milling tools of a raw schema-4 job, in document order. Knife tools have
 * no milling effect and never enter the field (their motions map to
 * non-cutting kinds; a cutting motion referencing one is an error). */
export function sequenceTools(job: unknown): SequenceTools {
  const tools = (job as { tools?: RawJobTool[] } | null)?.tools ?? [];
  const ids: string[] = [];
  const specs: ToolSpec[] = [];
  const scanRadiusMm: number[] = [];
  for (const tool of tools) {
    const geometry = tool.geometry;
    if (!geometry) continue;
    const dimensions = geometry.dimensions ?? {};
    if (geometry.kind === 'endmill') {
      specs.push({ kind: 'endmill', diameterMm: dimensions.diameter_mm });
      scanRadiusMm.push((dimensions.diameter_mm ?? 0) / 2);
    } else if (geometry.kind === 'vbit') {
      specs.push({
        kind: 'vbit',
        includedAngleDeg: dimensions.included_angle_deg,
        tipDiameterMm: dimensions.tip_diameter_mm,
        maxCuttingDiameterMm: dimensions.max_cutting_diameter_mm,
        cuttingHeightMm: dimensions.cutting_height_mm,
      });
      scanRadiusMm.push((dimensions.max_cutting_diameter_mm ?? 0) / 2);
    } else {
      continue;
    }
    ids.push(tool.id);
  }
  if (specs.length === 0) throw new Error('The job has no milling tool geometry for the stock display.');
  if (specs.length > MAX_TOOLS) throw new Error(`The job exceeds the ${MAX_TOOLS}-tool display cap.`);
  const toolIndexOf = (toolId: string) => {
    const index = ids.indexOf(toolId);
    return index === -1 ? undefined : index;
  };
  return { ids, specs, scanRadiusMm, toolIndexOf };
}

/** Compact structure-of-arrays store of the ordered sequence stream. */
export interface SequenceStore {
  readonly count: number;
  readonly x0: Float64Array;
  readonly y0: Float64Array;
  readonly z0: Float64Array;
  readonly x1: Float64Array;
  readonly y1: Float64Array;
  readonly z1: Float64Array;
  readonly kind: Uint8Array;
  readonly tool: Uint8Array;
  /** Index into the plan's operation list; ownership never wraps. */
  readonly opIndex: Uint16Array;
  readonly layer: Int32Array;
}

function motionKind(motion: PlannedMotion): number {
  if (motion.interpolation === 'linear_feed' && motion.effect === 'milling_sweep') {
    if (motion.purpose === 'entry') {
      const xyMove = motion.start.x !== motion.end.x || motion.start.y !== motion.end.y;
      return xyMove ? KIND_RAMP : KIND_PLUNGE;
    }
    return KIND_CUT;
  }
  if (motion.purpose === 'clearance') {
    return motion.end.z > motion.start.z ? KIND_RAPID_RETRACT : KIND_RAPID_XY;
  }
  return KIND_APPROACH;
}

export function buildSequenceStore(
  motions: readonly PlannedMotion[],
  tools: SequenceTools,
  operationIds: readonly string[],
): SequenceStore {
  if (operationIds.length > MAX_OPERATIONS) {
    throw new Error(`The plan exceeds the ${MAX_OPERATIONS}-operation display cap.`);
  }
  const count = motions.length;
  const store: SequenceStore = {
    count,
    x0: new Float64Array(count),
    y0: new Float64Array(count),
    z0: new Float64Array(count),
    x1: new Float64Array(count),
    y1: new Float64Array(count),
    z1: new Float64Array(count),
    kind: new Uint8Array(count),
    tool: new Uint8Array(count),
    opIndex: new Uint16Array(count),
    layer: new Int32Array(count),
  };
  const opPositions = new Map(operationIds.map((id, index) => [id, index]));
  for (let index = 0; index < count; index++) {
    const motion = motions[index];
    const kind = motionKind(motion);
    const tool = tools.toolIndexOf(motion.toolId);
    if (tool === undefined) {
      // Only non-milling tools (e.g. knife traces) may bypass the milling
      // tool array, and only on non-cutting motions.
      const cutting = kind === KIND_CUT || kind === KIND_PLUNGE || kind === KIND_RAMP;
      if (cutting || tools.ids.length >= MAX_TOOLS) {
        throw new RangeError(`motion ${index} references unknown tool '${motion.toolId}'`);
      }
      store.tool[index] = 255;
    } else {
      store.tool[index] = tool;
    }
    const op = opPositions.get(motion.operationId);
    if (op === undefined) {
      throw new RangeError(`motion ${index} references operation '${motion.operationId}' outside the plan`);
    }
    const coordinates = [motion.start.x, motion.start.y, motion.start.z, motion.end.x, motion.end.y, motion.end.z];
    if (coordinates.some(value => !Number.isFinite(value))) {
      throw new RangeError(`motion ${index} has non-finite coordinates`);
    }
    store.x0[index] = motion.start.x;
    store.y0[index] = motion.start.y;
    store.z0[index] = motion.start.z;
    store.x1[index] = motion.end.x;
    store.y1[index] = motion.end.y;
    store.z1[index] = motion.end.z;
    store.kind[index] = kind;
    store.opIndex[index] = op;
    store.layer[index] = motion.layer;
  }
  return store;
}

/** Stock after a prefix of motions, with per-cell operation ownership
 * (`opOwner`: 0 = untouched, otherwise operation index + 1). */
export interface StockCheckpoint {
  motionIndex: number;
  /** Quantized levels in field order, one per cell. */
  heights: Uint16Array;
  opOwner: Uint16Array;
  removedVolumeMm3: number;
  removedByOperationMm3: number[];
}

export interface SequenceField {
  field: StockField;
  store: SequenceStore;
  tools: SequenceTools;
  /** Full-grid ownership mirrors maintained alongside the engine tiles. */
  opOwner: Uint16Array;
  removedByOperationMm3: number[];
  resolution: Resolution;
}

/** Build the sequence field against the physical stock rectangle. */
export function createSequenceField(
  stock: StockRect,
  store: SequenceStore,
  tools: SequenceTools,
): SequenceField {
  const detail = Math.min(...tools.specs.map(spec =>
    spec.kind === 'endmill' ? spec.diameterMm : spec.tipDiameterMm));
  const resolution = chooseResolution(stock, detail);
  const field = createField(stock, tools.specs.map(normalizeTool), resolution);
  const opOwner = new Uint16Array(field.cols * field.rows);
  return { field, store, tools, opOwner, removedByOperationMm3: [], resolution };
}

function cellIndex(field: StockField, col: number, row: number): number {
  return row * field.cols + col;
}

function quantize(field: StockField, depthMm: number): number {
  return Math.max(0, Math.min(65535, Math.round(depthMm * field.invQuantum)));
}

/**
 * Apply motions `[from, to)` to the field, tracking per-cell operation
 * ownership and per-operation removed volume by diffing cell depths across
 * each cutting motion's conservative scan box (depth below top only ever
 * increases, so an increase is exactly this motion's removal).
 */
export function applyRange(state: SequenceField, from: number, to: number): void {
  const { field, store, tools, opOwner } = state;
  const perOp = state.removedByOperationMm3;
  while (perOp.length < store.count + 1) perOp.push(0);
  for (let index = from; index < to; index++) {
    const kind = store.kind[index];
    const op = store.opIndex[index];
    if (kind !== KIND_CUT && kind !== KIND_PLUNGE && kind !== KIND_RAMP) {
      applyMotion(field, {
        kind: 'approach',
        tool: store.tool[index],
        x0: store.x0[index], y0: store.y0[index], z0: store.z0[index],
        x1: store.x1[index], y1: store.y1[index], z1: store.z1[index],
      });
      continue;
    }
    const radius = tools.scanRadiusMm[store.tool[index]] ?? 0;
    const minX = Math.min(store.x0[index], store.x1[index]) - radius;
    const maxX = Math.max(store.x0[index], store.x1[index]) + radius;
    const minY = Math.min(store.y0[index], store.y1[index]) - radius;
    const maxY = Math.max(store.y0[index], store.y1[index]) + radius;
    const col0 = Math.max(0, Math.floor((minX - field.x0Mm) / field.cellMm));
    const col1 = Math.min(field.cols - 1, Math.floor((maxX - field.x0Mm) / field.cellMm));
    const row0 = Math.max(0, Math.floor((minY - field.y0Mm) / field.cellMm));
    const row1 = Math.min(field.rows - 1, Math.floor((maxY - field.y0Mm) / field.cellMm));
    const before: number[] = [];
    for (let row = row0; row <= row1; row++) {
      for (let col = col0; col <= col1; col++) {
        before.push(depthAt(field, col, row));
      }
    }
    applyMotion(field, {
      kind: kind === KIND_PLUNGE ? 'plunge' : kind === KIND_RAMP ? 'ramp' : 'cut',
      tool: store.tool[index],
      x0: store.x0[index], y0: store.y0[index], z0: store.z0[index],
      x1: store.x1[index], y1: store.y1[index], z1: store.z1[index],
    });
    let cell = 0;
    for (let row = row0; row <= row1; row++) {
      for (let col = col0; col <= col1; col++) {
        const depth = depthAt(field, col, row);
        if (depth > before[cell]) {
          const grid = cellIndex(field, col, row);
          opOwner[grid] = op + 1;
          perOp[op] += (depth - before[cell]) * field.cellAreaMm2;
        }
        cell++;
      }
    }
  }
}

/** Capture the current stock as a checkpoint prefix of `motionIndex`. */
export function captureCheckpoint(state: SequenceField, motionIndex: number): StockCheckpoint {
  const { field, opOwner } = state;
  const heights = new Uint16Array(field.cols * field.rows);
  for (let row = 0; row < field.rows; row++) {
    for (let col = 0; col < field.cols; col++) {
      heights[cellIndex(field, col, row)] = quantize(field, depthAt(field, col, row));
    }
  }
  return {
    motionIndex,
    heights,
    opOwner: opOwner.slice(),
    removedVolumeMm3: field.stats.removedVolumeMm3,
    removedByOperationMm3: state.removedByOperationMm3.slice(),
  };
}

function writeCell(field: StockField, col: number, row: number, level: number): void {
  const tile = Math.floor(col / TILE) + Math.floor(row / TILE) * field.tilesX;
  let heights = field.heights[tile];
  if (heights === undefined) {
    if (level === 0) return;
    heights = new Uint16Array(TILE * TILE);
    field.heights[tile] = heights;
    field.cellOwner[tile] ??= new Uint8Array(TILE * TILE);
  }
  heights[(col % TILE) + (row % TILE) * TILE] = level;
}

/** Restore a checkpoint exactly: tiles above the captured state are reset,
 * then captured levels are written back cell by cell. */
export function restoreCheckpoint(state: SequenceField, checkpoint: StockCheckpoint): void {
  const { field, opOwner } = state;
  field.heights.fill(undefined);
  field.cellOwner.fill(undefined);
  field.stats = {
    appliedMotions: 0,
    cuttingMotions: 0,
    dirtyCells: 0,
    removedVolumeMm3: checkpoint.removedVolumeMm3,
    stageRemovedMm3: [0, 0],
  };
  for (let row = 0; row < field.rows; row++) {
    for (let col = 0; col < field.cols; col++) {
      const level = checkpoint.heights[cellIndex(field, col, row)];
      if (level !== 0) writeCell(field, col, row, level);
    }
  }
  opOwner.set(checkpoint.opOwner);
  state.removedByOperationMm3 = checkpoint.removedByOperationMm3.slice();
}

/** Remaining-material depth probe at a setup-space point (plan section
 * 15.3): adequate to inspect tab heights without trusting the display grid,
 * so the resolution is reported alongside the value. Points outside the
 * physical stock return null. */
export function probeDepth(
  state: SequenceField,
  xMm: number,
  yMm: number,
): { depthMm: number; cellMm: number } | null {
  const { field } = state;
  const col = Math.floor((xMm - field.x0Mm) / field.cellMm);
  const row = Math.floor((yMm - field.y0Mm) / field.cellMm);
  if (col < 0 || row < 0 || col >= field.cols || row >= field.rows) return null;
  return { depthMm: depthAt(field, col, row), cellMm: field.cellMm };
}

/** Display color mapping for the top-down stock view. Returns RGBA bytes. */
export interface StockViewColors {
  /** operation index + 1 → [r, g, b] for operation coloring mode */
  operation: [number, number, number][];
}

export const SEQUENCE_STOCK_TOP: readonly [number, number, number] = [154, 157, 163];
const SEQUENCE_CUT_LOW: readonly [number, number, number] = [42, 98, 148];
const SEQUENCE_THROUGH: readonly [number, number, number] = [12, 16, 22];

export function stockViewColor(
  depthFraction: number,
  through: boolean,
  operation: number,
  colors: StockViewColors,
  operationMode: boolean,
): [number, number, number] {
  if (operationMode && operation > 0) {
    return colors.operation[(operation - 1) % colors.operation.length];
  }
  if (through) return [...SEQUENCE_THROUGH];
  if (depthFraction <= 0) return [...SEQUENCE_STOCK_TOP];
  const t = Math.min(depthFraction, 1);
  const top = SEQUENCE_STOCK_TOP;
  const low = SEQUENCE_CUT_LOW;
  return [
    Math.round(top[0] + (low[0] - top[0]) * t),
    Math.round(top[1] + (low[1] - top[1]) * t),
    Math.round(top[2] + (low[2] - top[2]) * t),
  ];
}

/** Deterministic per-operation palette (HSL gold-angle rotation). */
export function operationColors(count: number): [number, number, number][] {
  const colors: [number, number, number][] = [];
  for (let index = 0; index < count; index++) {
    const hue = (index * 137.508) % 360;
    const [s, l] = [0.55, 0.55];
    const c = (1 - Math.abs(2 * l - 1)) * s;
    const x = c * (1 - Math.abs(((hue / 60) % 2) - 1));
    const m = l - c / 2;
    const sector = Math.floor(hue / 60) % 6;
    const rgb = [[c, x, 0], [x, c, 0], [0, c, x], [0, x, c], [x, 0, c], [c, 0, x]][sector];
    colors.push(rgb.map(v => Math.round((v + m) * 255)) as [number, number, number]);
  }
  return colors;
}

/** Page a complete motion stream through the service until `total` is
 * reached; the caller supplies one page request per step. */
export async function pageAllMotions(
  request: (offset: number) => Promise<{ motions: { count: number; total: number; motions: PlannedMotion[] } }>,
): Promise<{ motions: PlannedMotion[]; total: number }> {
  const first = await request(0);
  const total = first.motions.total;
  const collected = [...first.motions.motions];
  while (collected.length < total) {
    const page = await request(collected.length);
    if (page.motions.motions.length === 0) break;
    collected.push(...page.motions.motions);
  }
  return { motions: collected, total };
}
