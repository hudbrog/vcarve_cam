import { z } from 'zod';
import { pointSchema } from './job';
import { diagnosticSchema } from './wire';

const integer = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
export const layerKeys = ['nominalTarget', 'removedLower', 'removedUpper', 'remainingTarget', 'possibleOvercut', 'accessibleFloor', 'missingFloor', 'requestedCenters'] as const;
export type StockLayer = typeof layerKeys[number];
const layerSchema = z.enum(layerKeys);
export const boundsSchema = z.strictObject({ min: pointSchema, max: pointSchema })
  .refine(b => b.min.x <= b.max.x && b.min.y <= b.max.y);
export type DisplayBounds = z.infer<typeof boundsSchema>;
export const regionInfoSchema = z.strictObject({ key: layerSchema, areaMm2: z.number().finite().nonnegative(),
  vertexCount: integer, bounds: boundsSchema.nullable(), geometryToleranceMm: z.number().finite().positive(),
}).refine(r => (r.vertexCount === 0) === (r.bounds === null));
export const sliceInfoSchema = z.strictObject({
  id: z.string().regex(/^(endmill|combined)-\d+$/), stage: z.enum(['endmill', 'combined']), depthMm: z.number().finite().nonnegative(),
  status: z.enum(['complete', 'empty', 'incomplete', 'inconclusive']).nullable(),
  contributingMotionCount: integer, capsuleRadialErrorMm: z.number().finite().nonnegative(),
  regions: z.array(regionInfoSchema).min(5).max(8), diagnostics: z.array(diagnosticSchema).max(100), omittedDiagnostics: integer,
  unavailableReason: z.string().min(1).nullable(),
}).refine(s => s.id.startsWith(`${s.stage}-`) && (s.stage === 'endmill') === (s.status !== null)
  && new Set(s.regions.map(r => r.key)).size === s.regions.length
  && layerKeys.slice(0, 5).every(key => s.regions.some(r => r.key === key)), 'Inconsistent slice metadata');
export const stockSliceSchema = z.strictObject({
  info: sliceInfoSchema,
  geometry: z.array(z.strictObject({ key: layerSchema,
    rings: z.array(z.strictObject({ hole: z.boolean(), points: z.array(pointSchema).min(3) })),
  })).max(8).nullable(),
}).refine(s => {
  if (s.info.unavailableReason !== null) return s.geometry === null;
  if (!s.geometry || s.geometry.length !== s.info.regions.length || new Set(s.geometry.map(r => r.key)).size !== s.geometry.length) return false;
  return s.geometry.every(r => r.rings.reduce((count, ring) => count + ring.points.length, 0) === s.info.regions.find(info => info.key === r.key)?.vertexCount)
    && s.info.regions.reduce((count, r) => count + r.vertexCount, 0) <= 60_000;
}, 'Inconsistent or partial stock polygons');
export type SliceInfo = z.infer<typeof sliceInfoSchema>;
export type StockSlice = z.infer<typeof stockSliceSchema>;
export type StockRegion = NonNullable<StockSlice['geometry']>[number];
export const stockLayers: Record<StockLayer, { label: string; description: string }> = {
  nominalTarget: { label: 'Nominal target', description: 'Requested target section at this depth.' },
  removedLower: { label: 'Lower removal bound', description: 'Inner bound of material removed by the recorded motions at this depth.' },
  removedUpper: { label: 'Upper removal bound', description: 'Outer bound of removal; the difference between bounds reflects polygon approximation.' },
  remainingTarget: { label: 'Remaining target', description: 'Target outside the lower removal bound. This can include wall allowance or tool-limited material; it is not all missed reachable stock.' },
  possibleOvercut: { label: 'Possible overcut', description: 'Upper removal bound outside the nominal target. This is uncertainty to inspect, not proof of a gouge.' },
  accessibleFloor: { label: 'Accessible floor', description: 'Floor reachable by the selected endmill-center region at this planning layer.' },
  missingFloor: { label: 'Missing floor beyond tolerance', description: 'Accessible floor still outside the lower removal bound after applying the engine’s XY coverage tolerance.' },
  requestedCenters: { label: 'Requested endmill centers', description: 'Admissible endmill-center area for this layer and the chosen clearing strategy.' },
};
