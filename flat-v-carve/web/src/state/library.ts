import { cuttingPresetSchema, libraryToolSchema, onlyToolChanged, slotIndex, type CuttingPreset, type LibraryTool, type ToolSlot } from '../contracts/library';
import type { Job } from '../contracts/job';
import { parseNumeric, readPath, type Draft } from './draft';

export type LibraryFields = Record<string,string>;
export interface LibraryField {path:string;label:string;kind?:'number'|'boolean';required?:boolean}
export const presetFields:LibraryField[] = [
  {path:'id',label:'Preset ID',required:true},{path:'name',label:'Preset name',required:true},
  {path:'material',label:'Material context'},{path:'machine',label:'Machine context'},
  {path:'spindle_rpm',label:'Spindle speed (rpm)',kind:'number'},
  {path:'cutting_feed_mm_min',label:'Cutting feed (mm/min)',kind:'number'},
  {path:'plunge_feed_mm_min',label:'Plunge feed (mm/min)',kind:'number'},
  {path:'max_stepdown_mm',label:'Maximum stepdown (mm)',kind:'number'},
  {path:'stepover_mm',label:'Stepover (mm)',kind:'number'},
];
export function libraryToolFields(kind:ToolSlot):LibraryField[] {
  const dimensions:LibraryField[] = kind === 'endmill' ? [
    {path:'diameter_mm',label:'Diameter (mm)'},{path:'cutting_length_mm',label:'Usable cutting length (mm)'},
    {path:'plunge_capable',label:'Geometry supports plunge',kind:'boolean'},
  ] : [
    {path:'included_angle_deg',label:'Included angle (°)'},{path:'tip_diameter_mm',label:'Actual flat-tip diameter (mm)'},
    {path:'max_cutting_diameter_mm',label:'Maximum cutting diameter (mm)'},{path:'cutting_height_mm',label:'Usable cutting height (mm)'},
  ];
  return [{path:'id',label:'Tool ID',required:true},{path:'name',label:'Tool name',required:true},
    ...dimensions.map(f => ({...f,path:`geometry.dimensions.${f.path}`,kind:f.kind ?? 'number' as const,required:true})),
    {path:'plunge_capable',label:'Plunge capability',kind:'boolean'},{path:'ramp_capable',label:'Ramp capability',kind:'boolean'}];
}
export function libraryText(record:unknown,fields:LibraryField[]):LibraryFields {
  return Object.fromEntries(fields.map(f => {const v=readPath(record,f.path);return [f.path,v === null || v === undefined ? '' : String(v)];}));
}
function values(text:LibraryFields,fields:LibraryField[]) {
  const result:Record<string,unknown>={};
  for (const field of fields) {
    const raw=text[field.path] ?? '';
    const value=field.kind === 'number' ? parseNumeric(raw) : field.kind === 'boolean' ? raw === '' ? null : raw === 'true' ? true : raw === 'false' ? false : 'invalid' : raw.trim() ? raw : null;
    const parts=field.path.split('.');let target=result;
    for (const part of parts.slice(0,-1)) {target[part]??={};target=target[part] as Record<string,unknown>;}
    target[parts.at(-1)!]=value;
  }
  return result;
}
export function parseLibraryTool(text:LibraryFields,kind:ToolSlot,presets:CuttingPreset[] = []):LibraryTool|null {
  const record=values(text,libraryToolFields(kind));
  const parsed=libraryToolSchema.safeParse({...record,geometry:{...(record.geometry as object),kind},cutting_presets:presets});
  return parsed.success ? parsed.data : null;
}
export function parseLibraryPreset(text:LibraryFields):CuttingPreset|null {
  const parsed=cuttingPresetSchema.safeParse(values(text,presetFields));return parsed.success ? parsed.data : null;
}
export function applyLibraryDraft(draft:Draft,original:Job,candidate:Job,slot:ToolSlot):Draft {
  if (!onlyToolChanged(original,candidate,slot)) throw new Error('Library selection changed unrelated job settings.');
  const prefix=`tools.${slotIndex(original,slot)}.`;
  return {base:structuredClone(candidate),text:Object.fromEntries(Object.entries(draft.text).filter(([key]) => !key.startsWith(prefix)))};
}
const labels:Record<string,string> = Object.fromEntries([...presetFields,...libraryToolFields('endmill'),...libraryToolFields('vbit')].map(f => [f.path,f.label]));
labels['geometry.kind']='Cutter type';
function leaves(value:unknown,prefix=''):string[] {
  if (value !== null && typeof value === 'object') return Object.entries(value).flatMap(([k,v]) => leaves(v,prefix ? `${prefix}.${k}` : k));
  return [prefix];
}
export const displayLibraryValue = (value:unknown) => value === null || value === undefined ? 'Not specified' : value === true ? 'Yes' : value === false ? 'No' : String(value);
export function toolChanges(original:Job,candidate:Job,slot:ToolSlot) {
  const index=slotIndex(original,slot),before=original.tools[index],after=candidate.tools[index];
  return [...new Set([...leaves(before),...leaves(after)])].filter(path => {
    const a=readPath(before,path),b=readPath(after,path);
    return a !== b && !(a && typeof a === 'object') && !(b && typeof b === 'object');
  }).map(path => ({label:labels[path] ?? path,before:displayLibraryValue(readPath(before,path)),after:displayLibraryValue(readPath(after,path))}));
}
