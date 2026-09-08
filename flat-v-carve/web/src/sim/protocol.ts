// Messages between the main thread and the simulator worker (U8 §5.4).
// The main thread builds the compact store once and transfers array copies
// to the worker (zero object cloning); seeks then flow both ways with
// dirty-tile deltas. Strict shapes, like the service contracts.
import { z } from 'zod';
import type { Resolution, StockRect, ToolSpec } from './engine';
import type { MotionStore } from './store';
import type { SessionStats, TileDelta } from './session';

const integer = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
export const stockRectSchema = z.strictObject({
  x0: z.number().finite(),
  y0: z.number().finite(),
  x1: z.number().finite(),
  y1: z.number().finite(),
  thicknessMm: z.number().finite().positive(),
}).refine(rect => rect.x1 > rect.x0 && rect.y1 > rect.y0);
export const toolSpecSchema = z.discriminatedUnion('kind', [
  z.strictObject({ kind: z.literal('endmill'), diameterMm: z.number().finite().positive() }),
  z.strictObject({
    kind: z.literal('vbit'),
    includedAngleDeg: z.number().finite().positive().max(180),
    tipDiameterMm: z.number().finite().min(0),
    maxCuttingDiameterMm: z.number().finite().positive(),
    cuttingHeightMm: z.number().finite().positive(),
  }),
]);
export const resolutionSchema = z.strictObject({
  cellMm: z.number().finite().positive(),
  cappedByTexels: z.boolean(),
  cappedByBudget: z.boolean(),
});
const tileDeltaSchema = z.strictObject({
  x: integer,
  y: integer,
  heights: z.instanceof(Uint16Array),
  owners: z.instanceof(Uint8Array),
});
const sessionStatsSchema = z.strictObject({
  applied: integer,
  dirtyCells: integer,
  removedVolumeMm3: z.number().finite().nonnegative(),
  stageRemovedMm3: z.tuple([z.number().finite().nonnegative(), z.number().finite().nonnegative()]),
});

/** Store payloads validated once at the boundary; typed arrays arrive
 * transferred, so the checks are shape-level only. */
const storeSchema = z.strictObject({
  count: integer,
  x0: z.instanceof(Float64Array),
  y0: z.instanceof(Float64Array),
  z0: z.instanceof(Float64Array),
  x1: z.instanceof(Float64Array),
  y1: z.instanceof(Float64Array),
  z1: z.instanceof(Float64Array),
  kind: z.instanceof(Uint8Array),
  tool: z.instanceof(Uint8Array),
  layer: z.instanceof(Int32Array),
  lengthMm: z.instanceof(Float64Array),
  feedMmMin: z.instanceof(Float32Array),
  assumedFeedCount: integer,
}).refine(store => store.x0.length === store.count
  && store.y0.length === store.count && store.z0.length === store.count
  && store.x1.length === store.count && store.y1.length === store.count && store.z1.length === store.count
  && store.kind.length === store.count && store.tool.length === store.count
  && store.layer.length === store.count && store.lengthMm.length === store.count
  && store.feedMmMin.length === store.count, 'Inconsistent store array lengths');

export const toWorkerSchema = z.discriminatedUnion('type', [
  z.strictObject({
    type: z.literal('init'),
    stock: stockRectSchema,
    tools: z.array(toolSpecSchema).min(1),
    toolIds: z.array(z.string().min(1)).min(1),
    resolution: resolutionSchema,
    store: storeSchema,
  }),
  z.strictObject({ type: z.literal('seek'), index: integer, fraction: z.number().finite().min(0).max(1) }),
]);
export type ToWorker = z.infer<typeof toWorkerSchema>;

export const fromWorkerSchema = z.discriminatedUnion('type', [
  z.strictObject({ type: z.literal('ready') }),
  z.strictObject({
    type: z.literal('seekDone'),
    applied: integer,
    fraction: z.number().finite().min(0).max(1),
    tiles: z.array(tileDeltaSchema),
    stats: sessionStatsSchema,
  }),
  z.strictObject({ type: z.literal('error'), message: z.string().min(1) }),
]);
export type FromWorker = z.infer<typeof fromWorkerSchema>;

// The worker works with these statically typed payloads.
export type InitMessage = {
  type: 'init';
  stock: StockRect;
  tools: ToolSpec[];
  toolIds: string[];
  resolution: Resolution;
  store: MotionStore;
};
export type SeekMessage = { type: 'seek'; index: number; fraction: number };
export type { SessionStats, TileDelta };
