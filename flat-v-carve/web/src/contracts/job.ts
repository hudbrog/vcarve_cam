import { z } from 'zod';

// Structural boundary only. Rust owns migration, machining rules, and validation.
const number = z.number().finite();
const optionalNumber = number.nullable().default(null);
const optionalBool = z.boolean().nullable().default(null);
const optionalString = z.string().nullable().default(null);
export const pointSchema = z.strictObject({ x: number, y: number });
export const endmillSpecSchema = z.strictObject({
  diameter_mm: number, cutting_length_mm: number, plunge_capable: z.boolean(),
});
export const vbitSpecSchema = z.strictObject({
  included_angle_deg: number, tip_diameter_mm: number,
  max_cutting_diameter_mm: number, cutting_height_mm: number,
});
export const toolSchema = z.strictObject({
  id: z.string(),
  geometry: z.discriminatedUnion('kind', [
    z.strictObject({ kind: z.literal('endmill'), dimensions: endmillSpecSchema }),
    z.strictObject({ kind: z.literal('vbit'), dimensions: vbitSpecSchema }),
  ]).nullable().default(null),
  spindle_rpm: optionalNumber, cutting_feed_mm_min: optionalNumber,
  plunge_feed_mm_min: optionalNumber, max_stepdown_mm: optionalNumber,
  stepover_mm: optionalNumber, ramp_capable: optionalBool, plunge_capable: optionalBool,
});
export const importSchema = z.strictObject({
  geometry_tolerance_mm: number, ticks_per_mm: optionalNumber,
  placement: z.strictObject({ origin_mm: pointSchema, scale: number, rotation_deg: number }),
});
export const stockSchema = z.strictObject({ thickness_mm: optionalNumber });
export const operationSchema = z.strictObject({
  id: z.string(), endmill_id: z.string(), vbit_id: z.string(),
  max_depth_mm: optionalNumber, wall_allowance_mm: optionalNumber,
  max_floor_ridge_mm: optionalNumber, max_detail_residual_mm: optionalNumber,
});
export const tolerancesSchema = z.strictObject({
  motion_tolerance_mm: optionalNumber, verification_tolerance_mm: optionalNumber,
});
export const machineProfileSchema = z.strictObject({
  id: z.string(), work_offset: optionalString, clearance_z_mm: optionalNumber,
  endmill_tool_number: z.number().int().nonnegative().max(4294967295).nullable().default(null),
  vbit_tool_number: z.number().int().nonnegative().max(4294967295).nullable().default(null),
  m6_contract: optionalString,
});
export const endmillPlanningSchema = z.strictObject({
  clearance_z_mm: number, start_xy_mm: pointSchema,
  strategy: z.enum(['depth_dependent', 'deepest_region']),
  entry: z.discriminatedUnion('kind', [
    z.strictObject({ kind: z.literal('plunge') }),
    z.strictObject({ kind: z.literal('ramp'), max_angle_deg: number, feed_mm_min: number }),
  ]),
  max_layers: z.number().int().nonnegative(), max_loops_per_layer: z.number().int().nonnegative(),
  max_motions: z.number().int().nonnegative(),
});
export const vbitPlanningSchema = z.strictObject({
  max_paths: z.number().int().nonnegative(), max_motions: z.number().int().nonnegative(),
  max_curve_segments: z.number().int().nonnegative(), max_depth_passes: z.number().int().nonnegative(),
  max_cleanup_iterations: z.number().int().nonnegative(), quality_sample_spacing_mm: number,
  max_quality_samples: z.number().int().nonnegative(), reachability_max_cells: z.number().int().nonnegative(),
  stock_slices: z.number().int().nonnegative(),
});
export const sourceSchema = z.strictObject({ filename: z.string(), svg: z.string() });
export const jobSchema = z.strictObject({
  schema_version: z.literal(3), name: z.string(), source: sourceSchema,
  import: importSchema, selected_region_ids: z.array(z.string()), stock: stockSchema,
  operation: operationSchema, tools: z.array(toolSchema), tolerances: tolerancesSchema,
  machine_profile: machineProfileSchema.nullable().default(null),
  endmill_planning: endmillPlanningSchema.nullable().default(null),
  vbit_planning: vbitPlanningSchema.nullable().default(null),
});
export type Job = z.infer<typeof jobSchema>;
export type Point = z.infer<typeof pointSchema>;

export function parseJob(value: unknown): Job {
  if (typeof value === 'object' && value !== null && 'schema_version' in value && value.schema_version !== 3) {
    throw new Error('This UI accepts job schema 3. Open older jobs or plans through the Rust service for migration and identity checks. Your current draft is unchanged.');
  }
  const parsed = jobSchema.safeParse(value);
  if (!parsed.success) {
    const first = parsed.error.issues[0];
    throw new Error(`Cannot open job: ${first.path.join('.') || 'document'} — ${first.message}. Your current draft is unchanged.`);
  }
  return parsed.data;
}
