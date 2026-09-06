import { z } from 'zod';
import { jobSchema, endmillSpecSchema, vbitSpecSchema, type Job } from './job';
import { apiVersion } from './wire';

export const libraryId = z.string().regex(/^[A-Za-z0-9_-]{1,100}$/);
const label = z.string().min(1).max(1000);
const scalar = z.number().finite().positive().nullable().default(null);
const revision = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
export const slotSchema = z.enum(['endmill','vbit']);
export type ToolSlot = z.infer<typeof slotSchema>;
export const cuttingPresetSchema = z.strictObject({id:libraryId,name:label,material:label.nullable().default(null),machine:label.nullable().default(null),
  spindle_rpm:scalar,cutting_feed_mm_min:scalar,plunge_feed_mm_min:scalar,max_stepdown_mm:scalar,stepover_mm:scalar});
export const libraryToolSchema = z.strictObject({id:libraryId,name:label,
  geometry:z.discriminatedUnion('kind',[
    z.strictObject({kind:z.literal('endmill'),dimensions:endmillSpecSchema}),
    z.strictObject({kind:z.literal('vbit'),dimensions:vbitSpecSchema}),
  ]),ramp_capable:z.boolean().nullable().default(null),plunge_capable:z.boolean().nullable().default(null),cutting_presets:z.array(cuttingPresetSchema).max(100),
}).refine(t => new Set(t.cutting_presets.map(p => p.id)).size === t.cutting_presets.length);
export const toolLibrarySchema = z.strictObject({schema_version:z.literal(1),revision,tools:z.array(libraryToolSchema).max(1000)})
  .refine(l => new Set(l.tools.map(t => t.id)).size === l.tools.length);
export type CuttingPreset = z.infer<typeof cuttingPresetSchema>;
export type LibraryTool = z.infer<typeof libraryToolSchema>;
export type ToolLibrary = z.infer<typeof toolLibrarySchema>;
export interface LibraryConnection {instanceId:string;engineVersion:string}
export interface LibraryJobInput {job:Job;revision:number;documentFingerprint:string}
export interface LibrarySelection extends LibraryJobInput {expectedRevision:number;slot:ToolSlot;toolId:string;presetId:string|null}
export interface LibraryCapture extends LibraryJobInput {expectedRevision:number;slot:ToolSlot;toolId:string;name:string;
  preset:{id:string;name:string;material:string|null;machine:string|null}|null}
export type LibraryChange =
  | {kind:'add_tool'|'replace_tool';tool:LibraryTool}
  | {kind:'remove_tool';tool_id:string}
  | {kind:'duplicate_tool';tool_id:string;new_id:string;name:string}
  | {kind:'add_preset'|'replace_preset';tool_id:string;preset:CuttingPreset}
  | {kind:'remove_preset';tool_id:string;preset_id:string}
  | {kind:'duplicate_preset';tool_id:string;preset_id:string;new_id:string;name:string};
const frame = {apiVersion:z.literal(apiVersion),engineVersion:z.string(),instanceId:z.string().regex(/^[a-f0-9]{32}$/),requestId:z.string()};
export const librarySnapshotSchema = z.strictObject({...frame,data:z.discriminatedUnion('state',[
  z.strictObject({state:z.literal('missing'),library:z.null()}),
  z.strictObject({state:z.literal('ready'),library:toolLibrarySchema}),
])});
const fingerprint = z.string().regex(/^[a-f0-9]{64}$/);
export const libraryCandidateSchema = z.strictObject({...frame,data:z.strictObject({libraryRevision:revision,jobRevision:revision,
  sourceFingerprint:fingerprint,candidateFingerprint:fingerprint,slot:slotSchema,toolId:libraryId,presetId:libraryId.nullable(),job:jobSchema})});
export type LibrarySnapshot = z.infer<typeof librarySnapshotSchema>;
export type LibraryCandidate = z.infer<typeof libraryCandidateSchema>;
export const slotIndex = (job:Job,slot:ToolSlot) => job.tools.findIndex(t => t.id === job.operation[slot === 'endmill' ? 'endmill_id' : 'vbit_id']);
export function onlyToolChanged(original:Job,candidate:Job,slot:ToolSlot):boolean {
  const index = slotIndex(original,slot);
  if (index < 0 || candidate.tools[index]?.id !== original.tools[index].id || candidate.tools[index].geometry?.kind !== slot) return false;
  const expected = structuredClone(original); expected.tools[index] = candidate.tools[index];
  return JSON.stringify(jobSchema.parse(expected)) === JSON.stringify(candidate);
}
export function acceptLibraryCandidate(result:LibraryCandidate,selection:LibrarySelection):LibraryCandidate {
  const r=result.data;
  if (r.libraryRevision !== selection.expectedRevision || r.jobRevision !== selection.revision || r.sourceFingerprint !== selection.documentFingerprint
    || r.slot !== selection.slot || r.toolId !== selection.toolId || r.presetId !== selection.presetId || !onlyToolChanged(selection.job,r.job,selection.slot))
    throw new Error('The library response differs from the reviewed selection or changes unrelated job settings. The job was kept.');
  return result;
}
