import { profileSchema, type MachineProfile } from '../contracts/export';

export type ProfileDraft = Record<string,string>;
export interface ProfileField { path:string; label:string; kind?:'number'|'boolean'|'multiline'; choices?:[string,string][] }
export const profileFields: ProfileField[] = [
  {path:'id',label:'Profile ID'},
  {path:'work_offset',label:'Work offset',choices:['G54','G55','G56','G57','G58','G59','G59.1','G59.2','G59.3'].map(v => [v,v])},
  {path:'z_datum',label:'Work Z zero',choices:[['stock_top','Stock top'],['stock_bottom','Stock bottom / table']]},
  {path:'clearance_z_mm',label:'Planning clearance above stock top (mm)',kind:'number'},
  {path:'decimal_places',label:'Output coordinate decimal places (0–9)',kind:'number'},
  {path:'spindle_spinup_seconds',label:'Spindle spin-up delay (seconds)',kind:'number'},
  {path:'coolant',label:'Coolant',choices:[['off','Off'],['flood','Flood'],['mist','Mist']]},
  {path:'length_compensation',label:'Tool length compensation',choices:[['macro_managed','Managed by M6 macro'],['tool_table','G43 H from tool table']]},
  ...[0,1].flatMap(i => [
    {path:`tools.${i}.tool_id`,label:`Tool ${i + 1} · job tool ID`},
    {path:`tools.${i}.tool_number`,label:`Tool ${i + 1} · LinuxCNC T number`,kind:'number' as const},
    {path:`tools.${i}.length_offset_number`,label:`Tool ${i + 1} · length offset H number`,kind:'number' as const},
    {path:`tools.${i}.spindle_direction`,label:`Tool ${i + 1} · spindle direction`,choices:[['clockwise','Clockwise (M3)'],['counterclockwise','Counterclockwise (M4)']] as [string,string][]},
  ]),
  {path:'start_mode',label:'Position at program start',choices:[['macro','Unknown · first M6 owns positioning'],['known','Known compensated tool-tip position']]},
  ...['x','y','z'].map(axis => ({path:`program_start_position_mm.${axis}`,label:`Startup ${axis.toUpperCase()} (work mm)`,kind:'number' as const})),
  {path:'m6.return_position.kind',label:'New compensated tool tip after M6',choices:[['caller_position','Returns to caller position'],['fixed_position','Returns to fixed work position'],['safe_retract','Unknown position · declared safe retract']]},
  ...['x','y','z'].map(axis => ({path:`m6.return_position.position_mm.${axis}`,label:`M6 return ${axis.toUpperCase()} (work mm)`,kind:'number' as const})),
  {path:'m6.return_position.z_mm',label:'Safe retract Z (work mm)',kind:'number'},
  ...['x','y'].map(axis => ({path:`m6.return_position.transit_xy_mm.${axis}`,label:`Retract-plane transit ${axis.toUpperCase()} (work mm)`,kind:'number' as const})),
  {path:'m6.reference',label:'M6 contract reference and description',kind:'multiline'},
  {path:'m6.preserves_work_datum',label:'M6 preserves work datum and rotation',kind:'boolean'},
  {path:'m6.local_offsets_unused',label:'No G52 / G92 local-offset compensation',kind:'boolean'},
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
export function parseProfileDraft(draft:ProfileDraft): {profile:MachineProfile|null;errors:Record<string,string>} {
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
