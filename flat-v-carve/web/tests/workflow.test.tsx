import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { parseJob } from '../src/contracts/job';
import type { Validation } from '../src/contracts/service';
import { libraryToolSchema, slotIndex } from '../src/contracts/library';
import { missingPlanningSettings, planningIssueField, setupField } from '../src/state/setupNeeds';
import { computationDefaults, defaultVbitComputation, defaultTolerances } from '../src/state/computation';
import { materialize, newDraft } from '../src/state/draft';
import { initialWorkspace, workspaceReducer } from '../src/state/workspace';
import { assignmentMatches } from '../src/service/useLibraryAssignments';
import { LibraryApplyPanel } from '../src/components/LibraryApplyPanel';
import { applyLibraryDraft } from '../src/state/library';

const fixture=()=>parseJob(JSON.parse(readFileSync(new URL('../../fixtures/m4/wide-floor.json',import.meta.url),'utf8')));
const receipt=(missingMachiningFields:string[]):Validation=>({revision:0,authoritative:true,valid:true,diagnostics:[],missingMachiningFields});
describe('setup guidance and library application',()=>{
  it('exposes a missing V-bit computation group before combined planning and links failed tasks to that editor',()=>{
    const job=fixture(); job.vbit_planning=null;
    expect(missingPlanningSettings(job,receipt(['vbit_planning']),'combined')).toEqual([{path:'vbit_planning',label:'V-bit computation settings',message:expect.stringContaining('Carve & tools')}]);
    expect(missingPlanningSettings(job,receipt(['vbit_planning']),'endmill')).toEqual([]);
    expect(planningIssueField(job,{code:'MISSING_VBIT_SETTINGS',message:'configure vbit_planning'})).toHaveProperty('path','vbit_planning');
  });
  it('maps IDs to labeled fields after reordering, with overlapping IDs and endmill-only requirements',()=>{
    const job=fixture(); job.tools.reverse(); job.tools[0].id='cutter.long'; job.tools[1].id='cutter';
    job.operation.vbit_id='cutter.long'; job.operation.endmill_id='cutter';
    expect(setupField(job,'tools.cutter.long.spindle_rpm')).toHaveProperty('path','tools.0.spindle_rpm');
    expect(setupField(job,'tools.cutter.geometry')).toHaveProperty('path','tools.1.geometry.dimensions.diameter_mm');
    job.endmill_planning!.entry={kind:'ramp',max_angle_deg:2,feed_mm_min:80};
    job.tools.push({...job.tools[0],id:'unused'});
    const needs=missingPlanningSettings(job,receipt(['tools.cutter.long.geometry','tools.cutter.long.spindle_rpm','tools.cutter.plunge_feed_mm_min','operation.max_floor_ridge_mm','tools.unused.geometry']),'endmill');
    expect(needs).toHaveLength(1); expect(needs[0].label).toBe('V-bit · Included angle');
  });
  it('uses defaults for computation blanks without replacing zero, partial text, or machining values',()=>{
    const job=fixture(); job.vbit_planning=null; const draft=newDraft(job);
    const candidate=materialize(draft).job!;
    expect(candidate.vbit_planning).toEqual(defaultVbitComputation);
    expect({...candidate,vbit_planning:null}).toEqual(job);
    draft.text['vbit_planning.max_cleanup_iterations']='0'; draft.text['vbit_planning.max_paths']='1e';
    expect(materialize(draft).job).toBeNull();
    draft.text['vbit_planning.max_paths']='';
    expect(materialize(draft).job?.vbit_planning?.max_cleanup_iterations).toBe(0);
    expect(materialize(draft).job?.vbit_planning?.max_paths).toBe(defaultVbitComputation.max_paths);
  });
  it('resets overrides with one undo and freezes resolved defaults in portable jobs',()=>{
    const job=fixture(),draft=newDraft(job);
    let state=initialWorkspace(draft);
    state=workspaceReducer(state,{type:'clear-fields',paths:Object.keys(computationDefaults)});
    const resolved=materialize(state.draft).job!;
    expect(resolved.vbit_planning).toEqual(defaultVbitComputation); expect(resolved.tolerances).toEqual(defaultTolerances);
    expect(resolved.tools).toEqual(job.tools); expect(resolved.operation).toEqual(job.operation);
    expect(materialize(newDraft(parseJob(JSON.parse(JSON.stringify(resolved))))).job).toEqual(resolved);
    expect(materialize(workspaceReducer(state,{type:'undo'}).draft).job).toEqual(job);
  });
  it('requires an explicit preset choice and keeps review distinct from applying',()=>{
    const source=fixture().tools[0];
    const tool=libraryToolSchema.parse({id:'saved',name:'Named cutter',geometry:source.geometry,ramp_capable:source.ramp_capable,plunge_capable:source.plunge_capable,cutting_presets:[]});
    const base={tool,slot:'endmill' as const,onPreset:()=>{},review:null,current:false,jobAvailable:true,busy:false,onReview:()=>{},onApply:()=>{}};
    const html=renderToStaticMarkup(<LibraryApplyPanel {...base} presetId={undefined} />);
    expect(html).toContain('Selecting a record only opens it for review');
    expect(html).toContain('disabled="">3. Review changes'); expect(html).not.toContain('Apply to job’s');
    const geometry=renderToStaticMarkup(<LibraryApplyPanel {...base} presetId={null} />);
    expect(geometry).toContain('Geometry only clears spindle speed'); expect(geometry).not.toContain('disabled="">3. Review changes');
  });
  it('marks applied labels as edited when job values change, without depending on array order',()=>{
    const job=fixture(),tool=structuredClone(job.tools[slotIndex(job,'endmill')]);
    const assignment={toolName:'Saved endmill',presetName:'Saved cut',tool};
    job.tools.reverse(); expect(assignmentMatches(job,'endmill',assignment)).toBe(true);
    job.tools[slotIndex(job,'endmill')].spindle_rpm=null;
    expect(assignmentMatches(job,'endmill',assignment)).toBe(false);
  });
  it('keeps automatic settings in the draft when applying an unrelated library tool',()=>{
    const job=fixture(); job.vbit_planning=null; job.tolerances={motion_tolerance_mm:null,verification_tolerance_mm:null};
    const draft=newDraft(job),original=materialize(draft).job!,candidate=structuredClone(original);
    candidate.tools[slotIndex(candidate,'endmill')].spindle_rpm=11000;
    const applied=applyLibraryDraft(draft,original,candidate,'endmill');
    expect(applied.base.vbit_planning).toBeNull(); expect(applied.base.tolerances).toEqual(job.tolerances);
    expect(materialize(applied).job).toEqual(candidate);
  });
});
