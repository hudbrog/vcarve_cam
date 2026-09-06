import { z } from 'zod';

// Computation budgets and optional coordinate rounding, separate from physical
// job tolerances and from the M6 machine profile.
export const verificationOptionsSchema = z.strictObject({
  max_cells: z.number().int().min(1).max(2_000_000),
  max_depth: z.number().int().min(1).max(40),
  reachability_max_cells: z.number().int().min(1).max(1_000_000),
  max_depth_bands: z.number().int().min(1).max(4096),
  max_findings: z.number().int().min(1).max(4096),
  decimal_places: z.number().int().min(0).max(9).nullable(),
});
export type VerificationOptions = z.infer<typeof verificationOptionsSchema>;
export function sameOptions(a: VerificationOptions, b: VerificationOptions): boolean {
  return (Object.keys(a) as (keyof VerificationOptions)[]).every(key => a[key] === b[key]);
}
