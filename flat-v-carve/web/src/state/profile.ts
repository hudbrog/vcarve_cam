import { profileSchema, type MachineProfile } from '../contracts/export';
import type { Job } from '../contracts/job';

export type ProfileDraft = Record<string,string>;
export interface ProfileField { path:string; label:string; help?:string; kind?:'number'|'boolean'|'multiline'; choices?:[string,string][] }
export const profileFields: ProfileField[] = [
  {path:'id',label:'Profile ID',help:'A name for this machine setup. If the job has a machine profile ID, use the same ID here.'},
  {path:'work_offset',label:'Work offset',help:'Select the work coordinate system where you set the job origin on the controller.',choices:['G54','G55','G56','G57','G58','G59','G59.1','G59.2','G59.3'].map(v => [v,v])},
  {path:'z_datum',label:'Work Z zero',help:'Where Z = 0 is set on the machine. Stock bottom requires the job’s stock thickness.',choices:[['stock_top','Stock top'],['stock_bottom','Stock bottom / table']]},
  {path:'clearance_z_mm',label:'Planning clearance above stock top (mm)',kind:'number',help:'Must match the clearance used to generate the plan. Use the copy button above to copy it from the job.'},
  {path:'decimal_places',label:'Minimum coordinate decimal places (0–9)',kind:'number',help:'Export increases precision up to 9 places when needed to preserve motion, reports the digits used, and verifies the rounded program.'},
  {path:'spindle_spinup_seconds',label:'Spindle spin-up delay (seconds)',kind:'number',help:'Pause after starting the spindle, before moving to cut. Use the time your spindle needs to reach speed.'},
  {path:'coolant',label:'Coolant',choices:[['off','Off'],['flood','Flood'],['mist','Mist']]},
  {path:'length_compensation',label:'Tool length compensation',help:'Choose who applies each tool’s measured length: your M6 tool-change macro, or this program using G43 H. This must match your controller setup.',choices:[['macro_managed','Managed by M6 macro'],['tool_table','G43 H from tool table']]},
  ...[0,1].flatMap(i => [
    {path:`tools.${i}.tool_id`,label:`Mapping ${i + 1} · job tool`,help:'Select the tool used by the job. Its ID can be a name such as endmill or vbit.'},
    {path:`tools.${i}.tool_number`,label:`Mapping ${i + 1} · LinuxCNC T number`,kind:'number' as const,help:'Machine tool number used in Tn M6. Enter a different whole number from 1 to 99999 for each tool.'},
    {path:`tools.${i}.length_offset_number`,label:`Mapping ${i + 1} · length offset H number`,kind:'number' as const,help:'Tool-table entry used in G43 Hn. Use H1 for T1 and H2 for T2 only if those entries contain the corresponding measured tool lengths.'},
    {path:`tools.${i}.spindle_direction`,label:`Mapping ${i + 1} · spindle direction`,choices:[['clockwise','Clockwise (M3)'],['counterclockwise','Counterclockwise (M4)']] as [string,string][]},
  ]),
  {path:'start_mode',label:'Position at program start',help:'Known means you position the compensated tool tip at the declared XYZ before running the program. Unknown requires the safe-retract option below; no axis move is emitted before the first M6.',choices:[['macro','Unknown · first M6 owns positioning'],['known','Known compensated tool-tip position']]},
  ...['x','y','z'].map(axis => ({path:`program_start_position_mm.${axis}`,label:`Startup ${axis.toUpperCase()} (work mm)`,kind:'number' as const})),
  {path:'m6.return_position.kind',label:'New compensated tool tip after M6',help:'What your tool-change routine guarantees after length compensation: the calling position, a fixed XYZ, or room for the program to retract upward and then travel in XY.',choices:[['caller_position','Returns to caller position'],['fixed_position','Returns to fixed work position'],['safe_retract','Unknown position · declared safe retract']]},
  ...['x','y','z'].map(axis => ({path:`m6.return_position.position_mm.${axis}`,label:`M6 return ${axis.toUpperCase()} (work mm)`,kind:'number' as const})),
  {path:'m6.return_position.z_mm',label:'Safe retract Z (work mm)',kind:'number',help:'The program first moves upward to this Z, then to the transit XY. The machine setup must provide a clear path for both moves. Z must be at least the stock-top clearance in your selected work coordinates.'},
  ...['x','y'].map(axis => ({path:`m6.return_position.transit_xy_mm.${axis}`,label:`Retract-plane transit ${axis.toUpperCase()} (work mm)`,kind:'number' as const})),
  {path:'m6.reference',label:'M6 contract reference and description',kind:'multiline',help:'Identify your tool-change macro or controller configuration and describe its positioning and length-compensation behavior.'},
  {path:'m6.preserves_work_datum',label:'M6 keeps the work origin and rotation unchanged',kind:'boolean'},
  {path:'m6.local_offsets_unused',label:'M6 does not use G52 / G92 local offsets for compensation',kind:'boolean'},
  {path:'m6.tool_offsets_z_only',label:'Tool offsets change Z only',kind:'boolean'},
  {path:'m6.reviewed',label:'I have reviewed this declared M6 contract',kind:'boolean'},
];
export function profileFieldActive(path:string, draft:ProfileDraft): boolean {
  if (path.includes('length_offset_number')) return draft.length_compensation === 'tool_table';
  if (path.startsWith('program_start_position_mm.')) return draft.start_mode === 'known';
  if (path.startsWith('m6.return_position.position_mm.')) return draft['m6.return_position.kind'] === 'fixed_position';
  if (path === 'm6.return_position.z_mm' || path.startsWith('m6.return_position.transit_xy_mm.')) return draft['m6.return_position.kind'] === 'safe_retract';
  return true;
}
export function profileDraft(profile?:MachineProfile): ProfileDraft {
  const draft:ProfileDraft = {};
  for (const field of profileFields) {
    let value:unknown = profile;
    for (const part of field.path.split('.')) value = value && typeof value === 'object' ? (value as Record<string,unknown>)[part] : undefined;
    draft[field.path] = value === null || value === undefined ? '' : String(value);
  }
  if (profile) draft.start_mode = profile.program_start_position_mm ? 'known' : 'macro';
  return draft;
}
export function parseProfileDraft(draft:ProfileDraft, job?:Pick<Job,'operation'>): {profile:MachineProfile|null;errors:Record<string,string>} {
  const number = (key:string) => /^[-+]?(?:\d+\.?\d*|\.\d+)(?:e[-+]?\d+)?$/i.test(draft[key]?.trim() ?? '') ? Number(draft[key]) : NaN;
  const xyz = (prefix:string) => ({x:number(`${prefix}.x`),y:number(`${prefix}.y`),z:number(`${prefix}.z`)});
  const kind = draft['m6.return_position.kind'];
  const result = profileSchema.safeParse({schema_version:1,id:draft.id,work_offset:draft.work_offset,z_datum:draft.z_datum,
    clearance_z_mm:number('clearance_z_mm'),decimal_places:number('decimal_places'),
    program_start_position_mm:draft.start_mode === 'known' ? xyz('program_start_position_mm') : null,
    length_compensation:draft.length_compensation,tools:[0,1].map(i => ({tool_id:draft[`tools.${i}.tool_id`],
      tool_number:number(`tools.${i}.tool_number`),length_offset_number:draft.length_compensation === 'macro_managed' ? null : number(`tools.${i}.length_offset_number`),
      spindle_direction:draft[`tools.${i}.spindle_direction`]})),
    spindle_spinup_seconds:number('spindle_spinup_seconds'),coolant:draft.coolant,
    m6:{reference:draft['m6.reference'],reviewed:draft['m6.reviewed'] === 'true',
      preserves_work_datum:draft['m6.preserves_work_datum'] === 'true',local_offsets_unused:draft['m6.local_offsets_unused'] === 'true',
      tool_offsets_z_only:draft['m6.tool_offsets_z_only'] === 'true',return_position:kind === 'fixed_position' ? {kind,position_mm:xyz('m6.return_position.position_mm')}
        : kind === 'safe_retract' ? {kind,z_mm:number('m6.return_position.z_mm'),transit_xy_mm:{x:number('m6.return_position.transit_xy_mm.x'),y:number('m6.return_position.transit_xy_mm.y')}} : {kind}},
  });
  const errors = result.success ? {} : Object.fromEntries(result.error.issues.map(issue => [issue.path.join('.'),
    issue.code === 'invalid_type' && issue.expected === 'number' ? 'Enter a complete, finite number.' : issue.message]));
  // Form guidance only; Rust still validates the profile against the source plan.
  const ids = [draft['tools.0.tool_id'],draft['tools.1.tool_id']];
  const numbers = [number('tools.0.tool_number'),number('tools.1.tool_number')];
  for (const i of [0,1]) {
    const idPath = `tools.${i}.tool_id`;
    const tPath = `tools.${i}.tool_number`;
    const hPath = `tools.${i}.length_offset_number`;
    if (ids[i] && job && ![job.operation.endmill_id,job.operation.vbit_id].includes(ids[i]))
      errors[idPath] = `Job tool ID “${ids[i]}” is not used by this job. Select endmill “${job.operation.endmill_id}” or V-bit “${job.operation.vbit_id}”; enter the machine number in the T field.`;
    else if (ids[i] && ids[0] === ids[1]) errors[idPath] = 'This job tool is mapped twice. Map the endmill and V-bit once each.';
    if (!Number.isInteger(numbers[i]) || numbers[i] < 1 || numbers[i] > 99999)
      errors[tPath] = 'Enter a whole T number from 1 to 99999.';
    else if (numbers[0] === numbers[1]) errors[tPath] = `T${numbers[i]} is assigned twice. Use a different T number for each job tool.`;
    if (draft.length_compensation === 'tool_table' && (!Number.isInteger(number(hPath)) || number(hPath) < 1 || number(hPath) > 99999))
      errors[hPath] = 'Tool-table compensation requires an H number from 1 to 99999 for this tool’s measured length.';
  }
  if (!['known','macro'].includes(draft.start_mode)) errors.start_mode = 'Choose the startup positioning contract.';
  return {profile:result.success && !Object.keys(errors).length ? result.data : null, errors};
}
export function reviewedProfile(profile:MachineProfile|null): boolean {
  return !!profile && profile.m6.reviewed && !!profile.m6.reference.trim() && profile.m6.preserves_work_datum
    && profile.m6.local_offsets_unused && profile.m6.tool_offsets_z_only;
}
export function recoverProfile(storage:Pick<Storage,'getItem'>):ProfileDraft {
  try {
    const saved:unknown = JSON.parse(storage.getItem('flat-v-carve:u6:profile') ?? 'null');
    const empty = profileDraft();
    if (saved && typeof saved === 'object' && !Array.isArray(saved) && Object.keys(saved).length === Object.keys(empty).length
      && Object.keys(empty).every(key => typeof (saved as ProfileDraft)[key] === 'string')) return saved as ProfileDraft;
  } catch { /* Missing recovery starts an empty profile, never a machine preset. */ }
  return profileDraft();
}
