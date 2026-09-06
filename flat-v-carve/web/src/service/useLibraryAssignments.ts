import { useState } from 'react';
import { z } from 'zod';
import { toolSchema, type Job } from '../contracts/job';
import { slotIndex, type ToolSlot } from '../contracts/library';

const assignmentSchema = z.strictObject({ toolName:z.string(),presetName:z.string().nullable(),tool:toolSchema });
const assignmentsSchema = z.strictObject({endmill:assignmentSchema.optional(),vbit:assignmentSchema.optional()});
export type LibraryAssignment = z.infer<typeof assignmentSchema>;
const key = 'flat-v-carve:library-assignments';
export function assignmentMatches(job:Job, slot:ToolSlot, assignment:LibraryAssignment) {
  return JSON.stringify(job.tools[slotIndex(job,slot)]) === JSON.stringify(assignment.tool);
}
export function useLibraryAssignments(recovered = true) {
  const [assignments,setAssignments] = useState<z.infer<typeof assignmentsSchema>>(() => {
    if (!recovered) { try { sessionStorage.removeItem(key); } catch { /* Optional display metadata. */ } return {}; }
    try { return assignmentsSchema.parse(JSON.parse(sessionStorage.getItem(key) ?? '{}')); } catch { return {}; }
  });
  function save(next:typeof assignments) {
    setAssignments(next);
    try { sessionStorage.setItem(key,JSON.stringify(next)); } catch { /* Job settings are independent of these display labels. */ }
  }
  return {assignments, applied:(slot:ToolSlot,assignment:LibraryAssignment)=>save({...assignments,[slot]:assignment}),clear:()=>save({})};
}
