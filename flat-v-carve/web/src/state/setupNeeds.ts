import type { Job } from '../contracts/job';
import type { Validation, Diagnostic } from '../contracts/service';
import type { PlanningStage } from '../contracts/planning';
import { allFields, toolFields } from './draft';

export interface SetupNeed { path: string; label: string; message: string }
// Translate Rust's job-tool IDs into editor array paths, including reordered tools
// and IDs containing dots. Rust remains the authority for which values are unset.
export function setupField(job: Job, path: string): SetupNeed {
  if (path === 'vbit_planning') return { path, label: 'V-bit computation settings', message: 'Review advanced V-bit computation in Carve & tools. Blank values use defaults.' };
  if (path === 'endmill_budgets') return {path, label:'Endmill computation settings',message:'Review advanced endmill computation in Carve & tools. Blank values use defaults.'};
  if (path === 'endmill_planning') return { path: 'endmill_planning.clearance_z_mm', label: 'Endmill travel and planning settings', message: 'Set clearance and starting position in Stock & origin, then strategy and entry in Carve & tools. Computation budgets have defaults.' };
  if (path === 'selected_region_ids') return { path, label: 'Regions to machine', message: 'Include at least one artwork region.' };
  const tool = job.tools.map((t, index) => ({ ...t, index })).sort((a,b) => b.id.length - a.id.length).find(t => path.startsWith(`tools.${t.id}.`));
  let editorPath = tool ? `tools.${tool.index}.${path.slice(`tools.${tool.id}.`.length)}` : path;
  const slot = tool?.id === job.operation.endmill_id ? 'endmill' : tool?.id === job.operation.vbit_id ? 'vbit' : null;
  const prefix = slot === 'endmill' ? 'Endmill · ' : slot === 'vbit' ? 'V-bit · ' : '';
  if (slot && editorPath.endsWith('.geometry')) editorPath = toolFields(job, slot)[0].path;
  const label = prefix + (allFields(job).find(f => f.path === editorPath)?.label ?? path);
  return { path: editorPath, label, message: 'Not specified. Open this field to complete the setup.' };
}
export function missingPlanningSettings(job: Job, validation: Validation | undefined, stage: PlanningStage): SetupNeed[] {
  return (validation?.missingMachiningFields ?? []).filter(path => {
    const owner = job.tools.slice().sort((a,b)=>b.id.length-a.id.length).find(t=>path.startsWith(`tools.${t.id}.`));
    if (owner && owner.id !== job.operation.endmill_id && owner.id !== job.operation.vbit_id) return false;
    const vbitPrefix = `tools.${job.operation.vbit_id}.`;
    if (stage === 'endmill' && (path === 'vbit_planning' || path === 'operation.max_floor_ridge_mm' || path === 'operation.max_detail_residual_mm'
      || (path.startsWith(vbitPrefix) && path !== `${vbitPrefix}geometry`))) return false;
    if (job.endmill_planning?.entry.kind === 'ramp' && path === `tools.${job.operation.endmill_id}.plunge_feed_mm_min`) return false;
    return true;
  }).map(path => setupField(job, path));
}
export function planningIssueField(job: Job, issue: Pick<Diagnostic,'code'|'message'|'fieldPath'>): SetupNeed | null {
  if (issue.code === 'MISSING_VBIT_SETTINGS' || issue.code === 'VBIT_RESOURCE_SETTINGS' || issue.code === 'VBIT_SAMPLE_SPACING') return setupField(job, 'vbit_planning');
  if (issue.code === 'MISSING_PLANNING_SETTINGS') return setupField(job, 'endmill_planning');
  if (['PLANNING_RESOURCE_LIMIT','PLANNING_LIMIT','MOTION_LIMIT','ENTRY_RESOURCE_LIMIT'].includes(issue.code)) return setupField(job,'endmill_budgets');
  if (['VBIT_PATH_LIMIT','VBIT_PASS_LIMIT','VBIT_MOTION_LIMIT','MEDIAL_RESOURCE_LIMIT','QUALITY_SAMPLE_LIMIT'].includes(issue.code)) return setupField(job,'vbit_planning');
  if (['MOTION_PRECISION','VERIFICATION_PRECISION','VBIT_PRECISION'].includes(issue.code)) return {path:'numerical_tolerances',label:'Numerical tolerances and import precision',message:'Refine import precision or review explicit tolerances. Planning never relaxes accuracy automatically.'};
  if (issue.fieldPath) return setupField(job, issue.fieldPath);
  return null;
}
