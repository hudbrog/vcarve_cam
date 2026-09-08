// Pure 2.5D stock-simulation engine (U8 experiment 1).
//
// The stock is a heightfield: one quantized depth per cell (Uint16, uncut = 0)
// held in lazily allocated 256x256-cell tiles. Cutting motions lower cells
// monotonically, which is exact for 3-axis 2.5D milling — a tool flank never
// cuts below the tip's own passage along the same trajectory, so only the tip
// profile participates in the surface update.
//
// Coordinates are workpiece millimetres with stock top Z = 0 and cutting Z
// negative, matching recorded motions. Cell row indices grow with +Y; any
// display flipping belongs to rendering.

export const TILE = 256;
export const DEFAULT_MAX_LONG_SIDE_TEXELS = 8192;
export const DEFAULT_BUDGET_CELLS = 64_000_000;

export interface StockRect {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
  thicknessMm: number;
}

export type CuttingKind = 'plunge' | 'ramp' | 'cut';
export type MotionKindName = CuttingKind | 'rapid_x_y' | 'rapid_retract' | 'approach';

/** One linear XYZ move; `tool` indexes the profile array given to the field. */
export interface SimMotion {
  kind: MotionKindName;
  tool: number;
  x0: number;
  y0: number;
  z0: number;
  x1: number;
  y1: number;
  z1: number;
}

export interface EndmillSpec {
  kind: 'endmill';
  diameterMm: number;
}
export interface VbitSpec {
  kind: 'vbit';
  includedAngleDeg: number;
  tipDiameterMm: number;
  maxCuttingDiameterMm: number;
  cuttingHeightMm: number;
}
export type ToolSpec = EndmillSpec | VbitSpec;

/** Normalized endmill: a flat disc of `radiusMm` at the tip Z. */
export interface EndmillTool {
  kind: 'endmill';
  radiusMm: number;
}
/**
 * Normalized V-bit: a truncated cone. `slope` is the radius growth per unit
 * height above the tip (tan of the half angle). The effective cone height is
 * clamped to the smaller of the declared cutting height and the height implied
 * by the maximum cutting diameter, so inconsistent specs cut defensively
 * rather than gouging.
 */
export interface VbitTool {
  kind: 'vbit';
  tipRadiusMm: number;
  slope: number;
  maxRadiusMm: number;
  maxDepthBelowTipMm: number;
}
export type Tool = EndmillTool | VbitTool;

export const ENDMILL_ROLE = 1;
export const VBIT_ROLE = 2;

function finitePositive(value: number, name: string): number {
  if (!Number.isFinite(value) || value <= 0) {
    throw new RangeError(`${name} must be finite and positive, got ${value}`);
  }
  return value;
}

export function normalizeTool(spec: ToolSpec): Tool {
  if (spec.kind === 'endmill') {
    return { kind: 'endmill', radiusMm: finitePositive(spec.diameterMm, 'diameterMm') / 2 };
  }
  const angle = spec.includedAngleDeg;
  if (!Number.isFinite(angle) || angle <= 0 || angle >= 180) {
    throw new RangeError(`includedAngleDeg must be in (0, 180), got ${angle}`);
  }
  // A pointed V-bit has a zero-diameter tip; only negatives are invalid.
  if (!Number.isFinite(spec.tipDiameterMm) || spec.tipDiameterMm < 0) {
    throw new RangeError(`tipDiameterMm must be finite and non-negative, got ${spec.tipDiameterMm}`);
  }
  const tipRadius = spec.tipDiameterMm / 2;
  const maxRadius = finitePositive(spec.maxCuttingDiameterMm, 'maxCuttingDiameterMm') / 2;
  if (maxRadius < tipRadius) {
    throw new RangeError('maxCuttingDiameterMm must not be smaller than the tip diameter');
  }
  const cuttingHeight = finitePositive(spec.cuttingHeightMm, 'cuttingHeightMm');
  const slope = Math.tan((angle / 2) * Math.PI / 180);
  const radialReach = Math.min(cuttingHeight * slope, maxRadius - tipRadius);
  return {
    kind: 'vbit',
    tipRadiusMm: tipRadius,
    slope,
    maxRadiusMm: tipRadius + radialReach,
    maxDepthBelowTipMm: radialReach / slope,
  };
}

export interface Resolution {
  cellMm: number;
  cappedByTexels: boolean;
  cappedByBudget: boolean;
}

/**
 * Pick the field resolution per the U8 plan: target `min(0.1 mm, detail/4)`,
 * then grow to fit the long-side texel cap, then grow again to fit the
 * worst-case dirty-cell budget (a full sheet counts as the dirty worst case).
 */
export function chooseResolution(
  rect: Pick<StockRect, 'x0' | 'y0' | 'x1' | 'y1'>,
  smallestCuttingDetailMm: number,
  maxLongSideTexels = DEFAULT_MAX_LONG_SIDE_TEXELS,
  budgetCells = DEFAULT_BUDGET_CELLS,
): Resolution {
  const width = finitePositive(rect.x1 - rect.x0, 'stock width');
  const height = finitePositive(rect.y1 - rect.y0, 'stock height');
  if (!Number.isFinite(maxLongSideTexels) || maxLongSideTexels < 1) {
    throw new RangeError('maxLongSideTexels must be at least 1');
  }
  if (!Number.isFinite(budgetCells) || budgetCells < 1) {
    throw new RangeError('budgetCells must be at least 1');
  }
  const target = Math.min(0.1, smallestCuttingDetailMm > 0 ? smallestCuttingDetailMm / 4 : 0.1);
  let cell = target;
  let cappedByTexels = false;
  const cap = Math.max(width, height) / maxLongSideTexels;
  if (cap > cell) {
    cell = cap;
    cappedByTexels = true;
  }
  let cappedByBudget = false;
  if (Math.ceil(width / cell) * Math.ceil(height / cell) > budgetCells) {
    cell = Math.max(cell, Math.sqrt((width * height) / budgetCells));
    cappedByBudget = true;
  }
  return { cellMm: cell, cappedByTexels, cappedByBudget };
}

export interface FieldStats {
  appliedMotions: number;
  cuttingMotions: number;
  dirtyCells: number;
  removedVolumeMm3: number;
  /** Removed volume attributed to the tool role that lowered each cell. */
  stageRemovedMm3: [number, number];
}

export interface StockField {
  readonly x0Mm: number;
  readonly y0Mm: number;
  readonly cellMm: number;
  readonly cellAreaMm2: number;
  readonly cols: number;
  readonly rows: number;
  readonly thicknessMm: number;
  /** Depth represented by one Uint16 level: thickness / 65535. */
  readonly quantumMm: number;
  readonly invQuantum: number;
  readonly tilesX: number;
  readonly tilesY: number;
  readonly tools: readonly Tool[];
  /** Cutting role (1 endmill, 2 V-bit) per tool index. */
  readonly toolRole: Uint8Array;
  /** Lazily allocated per tile; undefined tiles are pristine stock. */
  heights: (Uint16Array | undefined)[];
  cellOwner: (Uint8Array | undefined)[];
  /** Bumped on every cell change; incremental emission reads these. */
  tileVersions: Uint32Array;
  /**
   * Optional change notification for copy-on-write undo logs: when set, the
   * engine reports each tile once per epoch, before its first modification
   * in that epoch. See `beginEpoch`.
   */
  onTileChange?: (tile: number) => void;
  epoch: number;
  epochFlags: Uint8Array;
  stats: FieldStats;
}

export function createField(rect: StockRect, tools: Tool[], resolution: Resolution): StockField {
  const x0 = rect.x0;
  const y0 = rect.y0;
  const width = finitePositive(rect.x1 - rect.x0, 'stock width');
  const height = finitePositive(rect.y1 - rect.y0, 'stock height');
  const thickness = finitePositive(rect.thicknessMm, 'thicknessMm');
  const cell = finitePositive(resolution.cellMm, 'cellMm');
  const cols = Math.ceil(width / cell);
  const rows = Math.ceil(height / cell);
  const tilesX = Math.ceil(cols / TILE);
  const tilesY = Math.ceil(rows / TILE);
  const toolRole = new Uint8Array(tools.length);
  for (let index = 0; index < tools.length; index++) {
    const tool = tools[index];
    if (tool.kind === 'endmill' ? !(tool.radiusMm > 0) : !(tool.maxRadiusMm > 0)) {
      throw new RangeError(`tool ${index} has a non-positive radius`);
    }
    toolRole[index] = tool.kind === 'endmill' ? ENDMILL_ROLE : VBIT_ROLE;
  }
  return {
    x0Mm: x0,
    y0Mm: y0,
    cellMm: cell,
    cellAreaMm2: cell * cell,
    cols,
    rows,
    thicknessMm: thickness,
    quantumMm: thickness / 65535,
    invQuantum: 65535 / thickness,
    tilesX,
    tilesY,
    tools,
    toolRole,
    heights: new Array<Uint16Array | undefined>(tilesX * tilesY),
    cellOwner: new Array<Uint8Array | undefined>(tilesX * tilesY),
    tileVersions: new Uint32Array(tilesX * tilesY),
    onTileChange: undefined,
    epoch: 0,
    epochFlags: new Uint8Array(tilesX * tilesY),
    stats: {
      appliedMotions: 0,
      cuttingMotions: 0,
      dirtyCells: 0,
      removedVolumeMm3: 0,
      stageRemovedMm3: [0, 0],
    },
  };
}

export function allocatedTiles(field: StockField): number {
  let count = 0;
  for (const tile of field.heights) if (tile !== undefined) count++;
  return count;
}

/**
 * Begin a new change-notification epoch. With `onTileChange` set, each tile
 * is reported once, before its first modification in the new epoch, which is
 * the state an undo log must capture.
 */
export function beginEpoch(field: StockField): void {
  field.epoch++;
  field.epochFlags.fill(0);
}

/** Depth in mm at a cell, 0 for pristine or out-of-range cells. */
export function depthAt(field: StockField, col: number, row: number): number {
  if (col < 0 || row < 0 || col >= field.cols || row >= field.rows) return 0;
  const tile = (row >> 8) * field.tilesX + (col >> 8);
  const heights = field.heights[tile];
  if (heights === undefined) return 0;
  return heights[((row & 255) << 8) | (col & 255)] * field.quantumMm;
}

/** Cutting role that lowered a cell (0 pristine, 1 endmill, 2 V-bit). */
export function ownerAt(field: StockField, col: number, row: number): number {
  if (col < 0 || row < 0 || col >= field.cols || row >= field.rows) return 0;
  const tile = (row >> 8) * field.tilesX + (col >> 8);
  const owners = field.cellOwner[tile];
  if (owners === undefined) return 0;
  return owners[((row & 255) << 8) | (col & 255)];
}

/** Stable digest of all cut cells; equal fields hash equally. */
export function checksum(field: StockField): string {
  let hash = 2166136261;
  const mix = (value: number) => {
    for (let byte = 0; byte < 4; byte++) {
      hash ^= (value >>> (byte * 8)) & 255;
      hash = Math.imul(hash, 16777619) >>> 0;
    }
  };
  for (let tile = 0; tile < field.heights.length; tile++) {
    const heights = field.heights[tile];
    const owners = field.cellOwner[tile];
    if (heights === undefined || owners === undefined) continue;
    mix(tile);
    mix(field.tileVersions[tile]);
    for (let index = 0; index < heights.length; index++) {
      const depth = heights[index];
      if (depth === 0) continue;
      mix(index);
      mix(depth);
      mix(owners[index]);
    }
  }
  return hash.toString(16);
}

function allocTile(field: StockField, tile: number): Uint16Array {
  const heights = new Uint16Array(TILE * TILE);
  field.heights[tile] = heights;
  field.cellOwner[tile] = new Uint8Array(TILE * TILE);
  return heights;
}

// Segments shorter than this in XY are treated as plunges. 1e-8 mm is far
// below any motion tolerance the engine records.
const PLUNGE_AREA_EPS = 1e-16;

function applyEndmill(
  field: StockField,
  radius: number,
  role: number,
  ax: number,
  ay: number,
  az: number,
  bx: number,
  by: number,
  bz: number,
): void {
  const dx = bx - ax;
  const dy = by - ay;
  const a2 = dx * dx + dy * dy;
  const dz = bz - az;
  if (-az <= 0 && -bz <= 0) return;
  const r2 = radius * radius;
  const inv2a = a2 > PLUNGE_AREA_EPS ? 1 / (2 * a2) : 0;
  const colRange = indexRange(field.x0Mm, field.cellMm, field.cols, Math.min(ax, bx) - radius, Math.max(ax, bx) + radius);
  const rowRange = indexRange(field.y0Mm, field.cellMm, field.rows, Math.min(ay, by) - radius, Math.max(ay, by) + radius);
  if (colRange === null || rowRange === null) return;
  const [c0, c1] = colRange;
  const [r0, r1] = rowRange;
  const invQuantum = field.invQuantum;
  const thickness = field.thicknessMm;
  const cellArea = field.cellAreaMm2;
  const onTileChange = field.onTileChange;
  const epochFlags = field.epochFlags;
  for (let tr = r0 >> 8; tr <= r1 >> 8; tr++) {
    const rowStart = Math.max(r0, tr << 8);
    const rowEnd = Math.min(r1, (tr << 8) | 255);
    for (let tc = c0 >> 8; tc <= c1 >> 8; tc++) {
      const tile = tr * field.tilesX + tc;
      const colStart = Math.max(c0, tc << 8);
      const colEnd = Math.min(c1, (tc << 8) | 255);
      for (let row = rowStart; row <= rowEnd; row++) {
        const py = field.y0Mm + (row + 0.5) * field.cellMm - ay;
        const localRow = (row & 255) << 8;
        for (let col = colStart; col <= colEnd; col++) {
          const px = field.x0Mm + (col + 0.5) * field.cellMm - ax;
          const c = px * px + py * py - r2;
          let t0: number;
          let t1: number;
          if (inv2a === 0) {
            if (c > 0) continue;
            t0 = 0;
            t1 = 1;
          } else {
            const b = -2 * (px * dx + py * dy);
            const disc = b * b - 4 * a2 * c;
            if (disc < 0) continue;
            const root = Math.sqrt(disc);
            t0 = (-b - root) * inv2a;
            t1 = (-b + root) * inv2a;
            if (t0 < 0) t0 = 0;
            if (t1 > 1) t1 = 1;
            if (t0 > t1) continue;
          }
          // The flat tip at parameter t leaves depth -z(t); the deepest
          // passage while covering the cell wins, and -z is linear in t, so
          // the maximum sits at a coverage-interval endpoint.
          const depth0 = -az - dz * t0;
          const depth1 = -az - dz * t1;
          const depth = depth0 > depth1 ? depth0 : depth1;
          if (depth <= 0) continue;
          const level = depth >= thickness ? 65535 : (depth * invQuantum) | 0;
          const heights = field.heights[tile] ?? allocTile(field, tile);
          const local = localRow | (col & 255);
          const previous = heights[local];
          if (level > previous) {
            if (onTileChange !== undefined && epochFlags[tile] === 0) {
              epochFlags[tile] = 1;
              onTileChange(tile);
            }
            heights[local] = level;
            field.cellOwner[tile]![local] = role;
            const delta = (level - previous) * field.quantumMm * cellArea;
            field.stats.dirtyCells += previous === 0 ? 1 : 0;
            field.stats.removedVolumeMm3 += delta;
            field.stats.stageRemovedMm3[role - 1] += delta;
            field.tileVersions[tile]++;
          }
        }
      }
    }
  }
}

function applyVbit(
  field: StockField,
  tool: VbitTool,
  role: number,
  ax: number,
  ay: number,
  az: number,
  bx: number,
  by: number,
  bz: number,
): void {
  const dx = bx - ax;
  const dy = by - ay;
  const a2 = dx * dx + dy * dy;
  const dz = bz - az;
  const maxR = tool.maxRadiusMm;
  const tipR = tool.tipRadiusMm;
  const maxR2 = maxR * maxR;
  const tipR2 = tipR * tipR;
  const invSlope = 1 / tool.slope;
  if (-az <= 0 && -bz <= 0) return;
  const colRange = indexRange(field.x0Mm, field.cellMm, field.cols, Math.min(ax, bx) - maxR, Math.max(ax, bx) + maxR);
  const rowRange = indexRange(field.y0Mm, field.cellMm, field.rows, Math.min(ay, by) - maxR, Math.max(ay, by) + maxR);
  if (colRange === null || rowRange === null) return;
  const [c0, c1] = colRange;
  const [r0, r1] = rowRange;
  const invQuantum = field.invQuantum;
  const thickness = field.thicknessMm;
  const cellArea = field.cellAreaMm2;
  const plunge = a2 <= PLUNGE_AREA_EPS;
  const inv2a = plunge ? 0 : 1 / (2 * a2);
  const onTileChange = field.onTileChange;
  const epochFlags = field.epochFlags;
  for (let tr = r0 >> 8; tr <= r1 >> 8; tr++) {
    const rowStart = Math.max(r0, tr << 8);
    const rowEnd = Math.min(r1, (tr << 8) | 255);
    for (let tc = c0 >> 8; tc <= c1 >> 8; tc++) {
      const tile = tr * field.tilesX + tc;
      const colStart = Math.max(c0, tc << 8);
      const colEnd = Math.min(c1, (tc << 8) | 255);
      for (let row = rowStart; row <= rowEnd; row++) {
        const py = field.y0Mm + (row + 0.5) * field.cellMm - ay;
        const localRow = (row & 255) << 8;
        for (let col = colStart; col <= colEnd; col++) {
          const px = field.x0Mm + (col + 0.5) * field.cellMm - ax;
          const qq = px * px + py * py;
          if (qq > maxR2) continue;
          // Coverage interval [T0, T1]: parameters whose tip stays within the
          // cutting radius. Inside it, the cut surface height
          // z(t) + max(0, dist(t) - tipR)/slope is a linear term plus a
          // convex term, so its minimum sits at an interval endpoint, the
          // cell's projection onto the segment, or the tip-flat kink.
          let t0: number;
          let t1: number;
          if (plunge) {
            t0 = 0;
            t1 = 1;
          } else {
            const b = -2 * (px * dx + py * dy);
            const disc = b * b - 4 * a2 * (qq - maxR2);
            if (disc < 0) continue;
            const root = Math.sqrt(disc);
            t0 = (-b - root) * inv2a;
            t1 = (-b + root) * inv2a;
            if (t0 < 0) t0 = 0;
            if (t1 > 1) t1 = 1;
            if (t0 > t1) continue;
          }
          let surface = Infinity;
          for (let candidate = 0; candidate < 5; candidate++) {
            let t: number;
            if (candidate === 0) t = t0;
            else if (candidate === 1) t = t1;
            else if (candidate === 2) {
              if (plunge) continue;
              t = (px * dx + py * dy) / a2;
              if (t < t0) t = t0;
              if (t > t1) t = t1;
            } else {
              if (plunge) continue;
              const b = -2 * (px * dx + py * dy);
              const disc = b * b - 4 * a2 * (qq - tipR2);
              if (disc < 0) continue;
              const root = Math.sqrt(disc);
              const u = candidate === 3 ? (-b - root) * inv2a : (-b + root) * inv2a;
              if (u < t0 || u > t1) continue;
              t = u;
            }
            const dist = Math.sqrt(a2 * t * t - 2 * (px * dx + py * dy) * t + qq);
            const height = dist <= tipR ? 0 : (dist - tipR) * invSlope;
            const z = az + dz * t + height;
            if (z < surface) surface = z;
          }
          if (surface >= 0) continue;
          const depth = -surface;
          const level = depth >= thickness ? 65535 : (depth * invQuantum) | 0;
          const heights = field.heights[tile] ?? allocTile(field, tile);
          const local = localRow | (col & 255);
          const previous = heights[local];
          if (level > previous) {
            if (onTileChange !== undefined && epochFlags[tile] === 0) {
              epochFlags[tile] = 1;
              onTileChange(tile);
            }
            heights[local] = level;
            field.cellOwner[tile]![local] = role;
            const delta = (level - previous) * field.quantumMm * cellArea;
            field.stats.dirtyCells += previous === 0 ? 1 : 0;
            field.stats.removedVolumeMm3 += delta;
            field.stats.stageRemovedMm3[role - 1] += delta;
            field.tileVersions[tile]++;
          }
        }
      }
    }
  }
}

function indexRange(
  origin: number,
  cell: number,
  count: number,
  minMm: number,
  maxMm: number,
): readonly [number, number] | null {
  const lo = Math.max(0, Math.floor((minMm - origin) / cell));
  const hi = Math.min(count - 1, Math.floor((maxMm - origin) / cell));
  return lo > hi ? null : [lo, hi];
}

/**
 * Apply one recorded motion, optionally restricted to the sub-interval
 * [t0, t1] of the segment for fractional playback. Non-cutting kinds only
 * advance the applied counter.
 */
export function applyMotion(field: StockField, motion: SimMotion, t0 = 0, t1 = 1): void {
  field.stats.appliedMotions++;
  if (motion.kind !== 'cut' && motion.kind !== 'ramp' && motion.kind !== 'plunge') return;
  const coordinates = [motion.x0, motion.y0, motion.z0, motion.x1, motion.y1, motion.z1];
  if (coordinates.some(value => !Number.isFinite(value))) {
    throw new RangeError('simulator motions must have finite coordinates');
  }
  const tool = field.tools[motion.tool];
  if (tool === undefined) {
    throw new RangeError(`motion references unknown tool index ${motion.tool}`);
  }
  field.stats.cuttingMotions++;
  const start = Math.min(Math.max(t0, 0), 1);
  const end = Math.min(Math.max(t1, 0), 1);
  if (end <= start) return;
  const ax = motion.x0 + (motion.x1 - motion.x0) * start;
  const ay = motion.y0 + (motion.y1 - motion.y0) * start;
  const az = motion.z0 + (motion.z1 - motion.z0) * start;
  const bx = motion.x0 + (motion.x1 - motion.x0) * end;
  const by = motion.y0 + (motion.y1 - motion.y0) * end;
  const bz = motion.z0 + (motion.z1 - motion.z0) * end;
  if (tool.kind === 'endmill') {
    applyEndmill(field, tool.radiusMm, field.toolRole[motion.tool], ax, ay, az, bx, by, bz);
  } else {
    applyVbit(field, tool, field.toolRole[motion.tool], ax, ay, az, bx, by, bz);
  }
}

export function applyMotions(field: StockField, motions: Iterable<SimMotion>): void {
  for (const motion of motions) applyMotion(field, motion);
}
