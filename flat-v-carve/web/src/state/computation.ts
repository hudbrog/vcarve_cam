import type { Job } from '../contracts/job';

// Work ceilings, not preallocated work. These match the current Rust-supported
// maxima so ordinary jobs do not need a second, lower set of user-guessed caps.
// Sampling and cleanup remain separate, explicit quality/work choices.
export const defaultEndmillBudgets = { max_layers: 256, max_loops_per_layer: 1024, max_motions: 100000 };
export const defaultVbitComputation: NonNullable<Job['vbit_planning']> = {
  max_paths: 65536, max_motions: 1000000, max_curve_segments: 1000000,
  max_depth_passes: 256, max_cleanup_iterations: 2, quality_sample_spacing_mm: 1,
  max_quality_samples: 1000000, reachability_max_cells: 100000, stock_slices: 8,
};
export const defaultTolerances = { motion_tolerance_mm: 0.01, verification_tolerance_mm: 0.05 };
const paths = (root: string, values: Record<string, number>) => Object.fromEntries(Object.entries(values).map(([key, value]) => [`${root}.${key}`, value]));
export const computationDefaults: Readonly<Record<string, number>> = {
  ...paths('endmill_planning', defaultEndmillBudgets),
  ...paths('vbit_planning', defaultVbitComputation),
  ...paths('tolerances', defaultTolerances),
};
export const computationHints: Readonly<Record<string, string>> = {
  'endmill_planning.max_layers': 'Stops excessive depth layers; actual layers follow carve depth and tool stepdown.',
  'endmill_planning.max_loops_per_layer': 'Bounds offset loops within one depth layer.',
  'endmill_planning.max_motions': 'Bounds the total endmill motion list kept in memory.',
  'vbit_planning.max_paths': 'Bounds generated V-bit contours and floor lanes.',
  'vbit_planning.max_motions': 'Bounds the total V-bit motion list kept in memory.',
  'vbit_planning.max_curve_segments': 'Stops excessive subdivision while approximating curved paths.',
  'vbit_planning.max_depth_passes': 'Stops excessive depth passes; actual passes follow carve depth and tool stepdown.',
  'vbit_planning.max_cleanup_iterations': 'Additional attempts to remove detected leftover material. Zero disables cleanup; more iterations can add cuts and calculation time.',
  'vbit_planning.quality_sample_spacing_mm': 'Spacing of the planning quality samples used to find residual material. Smaller spacing costs more work and may change cleanup paths. Separate from M5 verification.',
  'vbit_planning.max_quality_samples': 'Bounds the planning sample lattice and motion witnesses. Exhaustion leaves quality unresolved.',
  'vbit_planning.reachability_max_cells': 'Bounds each adaptive cutter-reachability search. Exhaustion can leave detail unresolved.',
  'vbit_planning.stock_slices': 'Number of stock-depth slices used in the planning analysis and preview. More slices cost more calculation.',
  'tolerances.motion_tolerance_mm': 'Numerical path approximation tolerance. Smaller is stricter and may require finer imported geometry.',
  'tolerances.verification_tolerance_mm': 'Numerical allowance used by coverage and verification. Larger values permit more error; this is separate from your requested surface finish.',
};
