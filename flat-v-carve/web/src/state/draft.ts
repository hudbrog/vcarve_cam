import { importSchema, jobSchema, type Job } from '../contracts/job';
import { computationDefaults } from './computation';

export interface Field {
  path: string; label: string; unit?: string;
  kind?: 'boolean' | 'choice' | 'text' | 'multiline'; required?: boolean;
  integer?: boolean;
  options?: { value: string; label: string }[];
  when?: { path: string; value: string };
}
export const stockFields: Field[] = [
  { path: 'stock.thickness_mm', label: 'Stock thickness', unit: 'mm' },
];
export const placementFields: Field[] = [
  { path: 'import.placement.scale', label: 'Artwork scale', unit: '×', required: true },
  { path: 'import.placement.rotation_deg', label: 'Rotation', unit: '°', required: true },
  { path: 'import.placement.origin_mm.x', label: 'Origin X on source page', unit: 'mm', required: true },
  { path: 'import.placement.origin_mm.y', label: 'Origin Y on source page', unit: 'mm', required: true },
];
export const shapeFields: Field[] = [
  { path: 'operation.max_depth_mm', label: 'Maximum carve depth', unit: 'mm' },
  { path: 'operation.wall_allowance_mm', label: 'Endmill wall allowance', unit: 'mm' },
  { path: 'operation.max_floor_ridge_mm', label: 'Maximum floor ridge', unit: 'mm' },
  { path: 'operation.max_detail_residual_mm', label: 'Detail residual allowance', unit: 'mm' },
];
export const accuracyFields: Field[] = [
  { path: 'import.geometry_tolerance_mm', label: 'Import geometry tolerance', unit: 'mm', required: true },
  { path: 'import.ticks_per_mm', label: 'Integer precision (blank = Auto)', unit: 'ticks/mm' },
  { path: 'tolerances.motion_tolerance_mm', label: 'Motion tolerance', unit: 'mm' },
  { path: 'tolerances.verification_tolerance_mm', label: 'Verification tolerance', unit: 'mm' },
];
export const travelFields: Field[] = [
  { path: 'endmill_planning.clearance_z_mm', label: 'Planning clearance Z', unit: 'mm', required: true },
  { path: 'endmill_planning.start_xy_mm.x', label: 'Starting X', unit: 'mm', required: true },
  { path: 'endmill_planning.start_xy_mm.y', label: 'Starting Y', unit: 'mm', required: true },
];
export const strategyFields: Field[] = [
  { path: 'endmill_planning.strategy', label: 'Endmill clearing strategy', kind: 'choice', required: true, options: [
    { value: 'depth_dependent', label: 'Depth-dependent clearing' },
    { value: 'deepest_region', label: 'Deepest-region clearing' },
  ] },
  { path: 'endmill_planning.entry.kind', label: 'Endmill entry', kind: 'choice', required: true, options: [
    { value: 'plunge', label: 'Direct plunge' }, { value: 'ramp', label: 'Ramp' },
  ] },
  { path: 'endmill_planning.entry.max_angle_deg', label: 'Maximum ramp angle', unit: '°', required: true,
    when: { path: 'endmill_planning.entry.kind', value: 'ramp' } },
  { path: 'endmill_planning.entry.feed_mm_min', label: 'Ramp feed', unit: 'mm/min', required: true,
    when: { path: 'endmill_planning.entry.kind', value: 'ramp' } },
];
export const endmillLimitFields: Field[] = [
  { path: 'endmill_planning.max_layers', label: 'Maximum endmill layers', unit: 'layers', required: true, integer: true },
  { path: 'endmill_planning.max_loops_per_layer', label: 'Maximum loops per layer', unit: 'loops', required: true, integer: true },
  { path: 'endmill_planning.max_motions', label: 'Maximum endmill motions', unit: 'motions', required: true, integer: true },
];
export const vbitPlanningFields: Field[] = [
  { path: 'vbit_planning.max_paths', label: 'Maximum V-bit paths', unit: 'paths', required: true, integer: true },
  { path: 'vbit_planning.max_motions', label: 'Maximum V-bit motions', unit: 'motions', required: true, integer: true },
  { path: 'vbit_planning.max_curve_segments', label: 'Maximum curve segments', unit: 'segments', required: true, integer: true },
  { path: 'vbit_planning.max_depth_passes', label: 'Maximum depth passes', unit: 'passes', required: true, integer: true },
  { path: 'vbit_planning.max_cleanup_iterations', label: 'Maximum cleanup iterations', unit: 'iterations', required: true, integer: true },
  { path: 'vbit_planning.quality_sample_spacing_mm', label: 'Quality sample spacing', unit: 'mm', required: true },
  { path: 'vbit_planning.max_quality_samples', label: 'Maximum quality samples', unit: 'samples', required: true, integer: true },
  { path: 'vbit_planning.reachability_max_cells', label: 'Maximum reachability cells', unit: 'cells', required: true, integer: true },
  { path: 'vbit_planning.stock_slices', label: 'Stock preview slices', unit: 'slices', required: true, integer: true },
];
export const machineProfileFields: Field[] = [
  { path: 'machine_profile.id', label: 'Machine profile ID', kind: 'text', required: true },
  { path: 'machine_profile.work_offset', label: 'Work offset', kind: 'text' },
  { path: 'machine_profile.clearance_z_mm', label: 'Profile clearance Z', unit: 'mm' },
  { path: 'machine_profile.endmill_tool_number', label: 'Endmill tool number', integer: true },
  { path: 'machine_profile.vbit_tool_number', label: 'V-bit tool number', integer: true },
  { path: 'machine_profile.m6_contract', label: 'M6 tool-change description', kind: 'multiline' },
];
export const endmillPlanningFields = [...travelFields, ...strategyFields, ...endmillLimitFields];
export const setupGroups = [
  { path: 'endmill_planning', fields: endmillPlanningFields },
  { path: 'vbit_planning', fields: vbitPlanningFields },
  { path: 'machine_profile', fields: machineProfileFields },
] as const;
export function fieldStep(path: string): 'stock' | 'tools' | 'export' {
  if (path.startsWith('machine_profile.')) return 'export';
  if (path.startsWith('stock.') || path.startsWith('import.placement.') || travelFields.some(field => field.path === path)) return 'stock';
  return 'tools';
}
export function toolFields(job: Job, kind: 'endmill' | 'vbit'): Field[] {
  const index = job.tools.findIndex(tool => tool.id === job.operation[kind === 'endmill' ? 'endmill_id' : 'vbit_id']);
  if (index < 0) return [];
  const prefix = `tools.${index}.`;
  const geometry = kind === 'endmill' ? [
    { path: 'diameter_mm', label: 'Diameter', unit: 'mm' },
    { path: 'cutting_length_mm', label: 'Usable cutting length', unit: 'mm' },
    { path: 'plunge_capable', label: 'Geometry supports plunge', kind: 'boolean' as const },
  ] : [
    { path: 'included_angle_deg', label: 'Included angle', unit: '°' },
    { path: 'tip_diameter_mm', label: 'Actual flat-tip diameter', unit: 'mm' },
    { path: 'max_cutting_diameter_mm', label: 'Maximum cutting diameter', unit: 'mm' },
    { path: 'cutting_height_mm', label: 'Usable cutting height', unit: 'mm' },
  ];
  return [
    ...geometry.map(field => ({ ...field, path: `${prefix}geometry.dimensions.${field.path}` })),
    { path: `${prefix}spindle_rpm`, label: 'Spindle speed', unit: 'rpm' },
    { path: `${prefix}cutting_feed_mm_min`, label: 'Cutting feed', unit: 'mm/min' },
    { path: `${prefix}plunge_feed_mm_min`, label: 'Plunge feed', unit: 'mm/min' },
    { path: `${prefix}max_stepdown_mm`, label: 'Maximum stepdown', unit: 'mm' },
    { path: `${prefix}stepover_mm`, label: 'Stepover', unit: 'mm' },
    { path: `${prefix}plunge_capable`, label: 'Plunge capability', kind: 'boolean' },
    ...(kind === 'endmill' ? [{ path: `${prefix}ramp_capable`, label: 'Ramp capability', kind: 'boolean' as const }] : []),
  ];
}
export const allFields = (job: Job) => [...stockFields, ...placementFields, ...shapeFields, ...accuracyFields, ...toolFields(job, 'endmill'), ...toolFields(job, 'vbit'), ...setupGroups.flatMap(group => group.fields)];

export interface Draft { base: Job; text: Record<string, string> }
export const newDraft = (job: Job): Draft => ({ base: structuredClone(job), text: {} });
export function readPath(value: unknown, path: string): unknown {
  return path.split('.').reduce<unknown>((current, key) => current !== null && typeof current === 'object' ? (current as Record<string, unknown>)[key] : undefined, value);
}
function writePath(value: unknown, path: string, next: unknown) {
  const keys = path.split('.');
  let target = value as Record<string, unknown>;
  for (const key of keys.slice(0, -1)) {
    if (target[key] === null || target[key] === undefined) target[key] = {};
    target = target[key] as Record<string, unknown>;
  }
  target[keys.at(-1)!] = next;
}
export function fieldText(draft: Draft, field: Field): string {
  if (field.path in draft.text) return draft.text[field.path];
  const value = readPath(draft.base, field.path);
  return value === null || value === undefined ? '' : String(value);
}
export function parseNumeric(text: string): number | null | 'invalid' {
  const trimmed = text.trim();
  if (!trimmed) return null;
  if (!/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(trimmed)) return 'invalid';
  const value = Number(trimmed);
  return Number.isFinite(value) ? value : 'invalid';
}
export function fieldIsActive(draft: Draft, field: Field) {
  return !field.when || fieldText(draft, { path: field.when.path, label: '' }) === field.when.value;
}
function parseField(draft: Draft, field: Field, errors: Record<string, string>): unknown {
  if (!(field.path in draft.text)) return readPath(draft.base, field.path) ?? computationDefaults[field.path] ?? null;
  const raw = draft.text[field.path];
  if (!raw.trim() && computationDefaults[field.path] !== undefined) return computationDefaults[field.path];
  if (field.kind === 'text' || field.kind === 'multiline') return raw.trim() === '' ? null : raw;
  if (field.kind === 'choice') {
    if (raw === '') return null;
    if (!field.options?.some(option => option.value === raw)) errors[field.path] = 'Choose one of the listed options.';
    return raw;
  }
  const value = field.kind === 'boolean' ? (raw === '' ? null : raw === 'true' ? true : raw === 'false' ? false : 'invalid') : parseNumeric(raw);
  if (value === 'invalid') {
    errors[field.path] = field.kind === 'boolean' ? 'Choose Yes, No, or Not specified.' : 'Enter a finite number.';
    return null;
  }
  if (field.integer && value !== null && (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0)) errors[field.path] = 'Enter a nonnegative whole number within exact integer precision.';
  return value;
}
export function setupWarnings(draft: Draft): { path: string; message: string }[] {
  const planning = parseNumeric(fieldText(draft, travelFields[0]));
  const profile = parseNumeric(fieldText(draft, machineProfileFields[2]));
  return typeof planning === 'number' && typeof profile === 'number' && planning !== profile ? [{
    path: 'machine_profile.clearance_z_mm',
    message: `Planning clearance Z is ${planning} mm; profile clearance Z is ${profile} mm. Review both values before service validation. Neither value has been changed automatically.`,
  }] : [];
}
export function materialize(draft: Draft): { job: Job | null; previewJob: Job; errors: Record<string, string> } {
  const candidate = structuredClone(draft.base);
  const errors: Record<string, string> = {};
  const fields = allFields(draft.base);
  for (const field of fields) {
    if (setupGroups.some(group => field.path.startsWith(`${group.path}.`))) continue;
    if (!(field.path in draft.text) && computationDefaults[field.path] === undefined) continue;
    const value = parseField(draft, field, errors);
    if (errors[field.path]) continue;
    if (value === null && field.required) { errors[field.path] = 'This import field cannot be empty.'; continue; }
    writePath(candidate, field.path, value);
  }
  for (const group of setupGroups) {
    if (!group.fields.some(field => field.path in draft.text || computationDefaults[field.path] !== undefined)) continue;
    const activeFields = group.fields.filter(field => fieldIsActive(draft, field));
    if (group.path !== 'vbit_planning' && activeFields.every(field => fieldText(draft, field).trim() === '')) {
      writePath(candidate, group.path, null);
      continue;
    }
    // Rebuild active fields so a plunge never retains ramp-only JSON members.
    // Inactive text remains in the recovery draft for switching back to ramp.
    const block = {};
    for (const field of activeFields) {
      const value = parseField(draft, field, errors);
      if (value === null && field.required && !errors[field.path]) errors[field.path] = 'Complete this setting, or clear its settings block.';
      writePath(block, field.path.slice(group.path.length + 1), value);
    }
    writePath(candidate, group.path, block);
  }
  // Unfinished tool/planning groups must not revert readable artwork placement.
  const previewJob = structuredClone(draft.base);
  const previewImport = importSchema.safeParse(candidate.import);
  if (previewImport.success) previewJob.import = previewImport.data;
  for (const kind of ['endmill', 'vbit'] as const) {
    const index = candidate.tools.findIndex(tool => tool.id === candidate.operation[kind === 'endmill' ? 'endmill_id' : 'vbit_id']);
    if (index < 0) continue;
    const geometryFields = toolFields(draft.base, kind).filter(field => field.path.includes('.geometry.'));
    if (!geometryFields.some(field => field.path in draft.text)) continue;
    if (geometryFields.every(field => fieldText(draft, field).trim() === '')) {
      candidate.tools[index].geometry = null;
    } else {
      writePath(candidate, `tools.${index}.geometry.kind`, kind);
      for (const field of geometryFields) {
        if (fieldText(draft, field).trim() === '') errors[field.path] = 'Complete this geometry value, or clear the tool geometry.';
      }
    }
  }
  const result = jobSchema.safeParse(candidate);
  if (!result.success) for (const issue of result.error.issues) errors[issue.path.join('.')] ??= issue.message;
  return { job: result.success && Object.keys(errors).length === 0 ? result.data : null, previewJob, errors };
}
