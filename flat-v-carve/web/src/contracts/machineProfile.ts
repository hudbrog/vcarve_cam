import { z } from 'zod';

const coordinate = z.number().finite().min(-1_000_000).max(1_000_000);
const xy = z.strictObject({ x: coordinate, y: coordinate });
const xyz = xy.extend({ z: coordinate });
const toolNumber = z.number().int().min(1).max(99_999);
export const profileSchema = z.strictObject({
  schema_version: z.literal(1), id: z.string().min(1).max(128),
  work_offset: z.enum(['G54','G55','G56','G57','G58','G59','G59.1','G59.2','G59.3']),
  z_datum: z.enum(['stock_top','stock_bottom']), clearance_z_mm: z.number().finite().positive(),
  decimal_places: z.number().int().min(0).max(9), program_start_position_mm: xyz.nullable(),
  length_compensation: z.enum(['macro_managed','tool_table']),
  tools: z.array(z.strictObject({ tool_id: z.string().min(1).max(128), tool_number: toolNumber,
    length_offset_number: toolNumber.nullable(), spindle_direction: z.enum(['clockwise','counterclockwise']) })).length(2),
  spindle_spinup_seconds: z.number().finite().min(0).max(3600), coolant: z.enum(['off','flood','mist']),
  m6: z.strictObject({ reference: z.string().max(4000), reviewed: z.boolean(),
    return_position: z.discriminatedUnion('kind', [
      z.strictObject({ kind: z.literal('caller_position') }),
      z.strictObject({ kind: z.literal('fixed_position'), position_mm: xyz }),
      z.strictObject({ kind: z.literal('safe_retract'), z_mm: coordinate, transit_xy_mm: xy }),
    ]), preserves_work_datum: z.boolean(), local_offsets_unused: z.boolean(), tool_offsets_z_only: z.boolean(),
  }),
});
export type MachineProfile = z.infer<typeof profileSchema>;
