import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { parseJob } from '../src/contracts/job';
import { acceptLibraryCandidate, libraryToolSchema, cuttingPresetSchema, toolLibrarySchema, slotIndex, type LibraryCandidate, type LibrarySnapshot } from '../src/contracts/library';
import { libraryText, libraryToolFields, parseLibraryTool, parseLibraryPreset, toolChanges, presetFields, resolveDraftLibraryTool, draftToolChanges, type DraftLibrarySelection } from '../src/state/library';
import { newDraft, materialize } from '../src/state/draft';
import { initialWorkspace, workspaceReducer } from '../src/state/workspace';
import { ToolLibraryDialog } from '../src/components/ToolLibraryDialog';
import { fixtureService } from '../src/service/fixture';
import { apiVersion } from '../src/contracts/wire';
import { ToolsSetup } from '../src/components/SetupEditors';

const fixture = () => parseJob(JSON.parse(readFileSync(new URL('../../fixtures/m4/finite-tip.json', import.meta.url), 'utf8')));
const fields = ['spindle_rpm','cutting_feed_mm_min','plunge_feed_mm_min','max_stepdown_mm','stepover_mm'] as const;
function savedTool(slot:'endmill'|'vbit'='endmill') {
  const job=fixture(),source=job.tools[slotIndex(job,slot)];
  const tool=libraryToolSchema.parse({id:'saved',name:'Saved tool',geometry:source.geometry,ramp_capable:source.ramp_capable,plunge_capable:source.plunge_capable,
    cutting_presets:[{id:'preset',name:'Saved cutting values',...Object.fromEntries(fields.map(key=>[key,source[key]]))}]});
  const snapshot:LibrarySnapshot={apiVersion,engineVersion:'0.7.2',instanceId:'a'.repeat(32),requestId:'test',data:{state:'ready',library:{schema_version:1,revision:3,tools:[tool]}}};
  const selection:DraftLibrarySelection={connection:{instanceId:snapshot.instanceId,engineVersion:snapshot.engineVersion},expectedRevision:3,draftRevision:0,
    slot,jobToolId:source.id,toolId:tool.id,presetId:'preset'};
  return {job,source,snapshot,selection};
}
describe('library records and job boundary', () => {
  it.each(['endmill','vbit'] as const)('copies %s geometry, capabilities and explicit preset blanks into the existing job ID',slot=>{
    const {snapshot,selection,source}=savedTool(slot);
    expect(resolveDraftLibraryTool(snapshot,selection).settings).toEqual(source);
    const geometry=resolveDraftLibraryTool(snapshot,{...selection,presetId:null}).settings;
    for (const key of fields) expect(geometry[key]).toBeNull();
    expect(geometry.geometry).toEqual(source.geometry); expect(geometry.id).toBe(source.id);
    snapshot.data.library!.tools[0].cutting_presets[0].spindle_rpm=null;
    const partial=resolveDraftLibraryTool(snapshot,selection).settings;
    expect(partial.spindle_rpm).toBeNull(); expect(partial.cutting_feed_mm_min).toBe(source.cutting_feed_mm_min);
  });
  it('rejects missing records and changed library revisions or service identities before copying',()=>{
    const {snapshot,selection}=savedTool();
    for (const patch of [{expectedRevision:2},{toolId:'missing'},{presetId:'missing'},{slot:'vbit' as const},
      {connection:{...selection.connection,instanceId:'b'.repeat(32)}},{connection:{...selection.connection,engineVersion:'other'}}]) {
      expect(()=>resolveDraftLibraryTool(snapshot,{...selection,...patch})).toThrow();
    }
    expect(()=>resolveDraftLibraryTool({...snapshot,data:{state:'missing',library:null}},selection)).toThrow(/library changed/);
  });
  it('repairs a partial tool as one undoable edit while retaining unfinished travel, other tools, and automatic defaults',()=>{
    const {job,snapshot,selection}=savedTool(); job.tools.reverse(); job.endmill_planning=null; job.vbit_planning=null;
    job.tolerances={motion_tolerance_mm:null,verification_tolerance_mm:null};
    const draft=newDraft(job),index=slotIndex(job,'endmill');
    draft.text={'endmill_planning.clearance_z_mm':'5','tools.0.spindle_rpm':'1e',[`tools.${index}.geometry.dimensions.diameter_mm`]:'1e'};
    expect(materialize(draft).job).toBeNull();
    const settings=resolveDraftLibraryTool(snapshot,selection).settings;
    expect(draftToolChanges(draft,settings,'endmill')).toContainEqual({label:'Diameter (mm)',before:'1e',after:'4'});
    const state=initialWorkspace(draft),action={type:'apply-library-tool' as const,expectedRevision:0,slot:'endmill' as const,settings};
    const applied=workspaceReducer(state,action);
    expect(applied.revision).toBe(1); expect(applied.past).toHaveLength(1);
    expect(applied.draft.text).toEqual({'endmill_planning.clearance_z_mm':'5','tools.0.spindle_rpm':'1e'});
    expect(applied.draft.base).toEqual(job); // the saved snapshot matches the base; only the partial tool input is repaired
    expect(draftToolChanges(applied.draft,settings,'endmill')).toEqual([]);
    expect(materialize(applied.draft).job).toBeNull(); // unfinished travel still blocks planning
    expect(workspaceReducer(applied,{type:'undo'}).draft).toEqual(draft);
    expect(workspaceReducer(applied,action)).toBe(applied); // delayed apply cannot overwrite a newer revision
    const matching=initialWorkspace(newDraft(job)); expect(workspaceReducer(matching,action)).toBe(matching);
    expect(()=>workspaceReducer(state,{...action,settings:{...settings,id:'different'}})).toThrow(/job slot/);
  });
  it('keeps the applied label when unrelated fields are unfinished and marks actual tool edits',()=>{
    const {job,source}=savedTool(),draft=newDraft(job),index=slotIndex(job,'endmill');
    draft.text['endmill_planning.start_xy_mm.x']='1e';
    const props={draft,fields:()=>null,dispatch:()=>{},assignments:{endmill:{toolName:'Saved tool',presetName:'Saved preset',tool:source}}};
    expect(materialize(draft).job).toBeNull();
    expect(renderToStaticMarkup(<ToolsSetup {...props} />)).toContain('Applied from library');
    draft.text[`tools.${index}.spindle_rpm`]='1e';
    expect(renderToStaticMarkup(<ToolsSetup {...props} />)).toContain('Edited since library selection');
  });
  it('tracks Rust record fields and requires explicit nullable data without hidden references', () => {
    const rust = readFileSync(new URL('../../crates/cam-core/src/tool_library.rs', import.meta.url), 'utf8');
    for (const [name, schema] of [['LibraryTool',libraryToolSchema],['CuttingPreset',cuttingPresetSchema],['ToolLibrary',toolLibrarySchema]] as const) {
      const body = rust.match(new RegExp(`pub struct ${name} \\{([\\s\\S]*?)\\n\\}`))![1];
      expect(Object.keys(schema.shape).sort()).toEqual([...body.matchAll(/pub (\w+):/g)].map(m => m[1]).sort());
    }
    expect(toolLibrarySchema.safeParse({schema_version:2,revision:0,tools:[]}).success).toBe(false);
    expect(toolLibrarySchema.safeParse({schema_version:1,revision:2**53,tools:[]}).success).toBe(false);
  });
  it('retains blanks, partial numeric text, actual zero tip size, and nullable capabilities', () => {
    const source = fixture().tools.find(t => t.geometry?.kind === 'vbit')!;
    const text = libraryText({...source,id:'bit',name:'Actual cutter'},libraryToolFields('vbit'));
    text['geometry.dimensions.tip_diameter_mm'] = '0'; text.ramp_capable = ''; text.plunge_capable = 'false';
    const tool = parseLibraryTool(text,'vbit')!;
    expect(tool.geometry.dimensions).toHaveProperty('tip_diameter_mm',0);
    expect(tool.ramp_capable).toBeNull(); expect(tool.plunge_capable).toBe(false);
    text['geometry.dimensions.cutting_height_mm'] = '1e';
    expect(parseLibraryTool(text,'vbit')).toBeNull(); expect(text['geometry.dimensions.cutting_height_mm']).toBe('1e');
    const preset = libraryText({id:'preset',name:'Partial preset'},presetFields);
    expect(parseLibraryPreset(preset)?.spindle_rpm).toBeNull();
    preset.spindle_rpm = '0'; expect(parseLibraryPreset(preset)).toBeNull();
    preset.spindle_rpm = '18000'; expect(parseLibraryPreset(preset)?.spindle_rpm).toBe(18000);
  });
  it('accepts omitted optional import values as unset, matching Rust Option fields', () => {
    const parsed = libraryToolSchema.parse({id:'minimal',name:'Minimal import',geometry:fixture().tools[0].geometry,
      cutting_presets:[{id:'blank',name:'Blank preset'}]});
    expect(parsed.ramp_capable).toBeNull(); expect(parsed.plunge_capable).toBeNull();
    for (const key of fields) expect(parsed.cutting_presets[0][key]).toBeNull();
    expect(parsed.cutting_presets[0].material).toBeNull();
  });
  it('applies to a reordered role as one undoable edit, preserving job identity and inactive text', () => {
    const original = fixture(); original.tools.reverse();
    const index = slotIndex(original,'endmill'); expect(index).toBe(1);
    const draft = newDraft(original); draft.text['endmill_planning.entry.max_angle_deg'] = '1e';
    draft.text[`tools.${index}.spindle_rpm`] = String(original.tools[index].spindle_rpm);
    const candidate = structuredClone(original); for (const key of fields) candidate.tools[index][key] = null;
    expect(toolChanges(original,candidate,'endmill')).toHaveLength(5);
    expect(toolChanges(original,candidate,'endmill').every(c => c.after === 'Not specified')).toBe(true);
    const state = initialWorkspace(draft);
    const next = workspaceReducer(state,{type:'apply-library',expectedRevision:0,original,candidate,slot:'endmill'});
    expect(next.revision).toBe(1); expect(next.past).toHaveLength(1);
    expect(next.draft.text).toEqual({'endmill_planning.entry.max_angle_deg':'1e'});
    expect(materialize(next.draft).job).toEqual(candidate);
    expect(next.draft.base.operation).toEqual(original.operation); expect(next.draft.base.machine_profile).toEqual(original.machine_profile);
    expect(workspaceReducer(next,{type:'undo'}).draft).toEqual(draft);
    expect(workspaceReducer(next,{type:'apply-library',expectedRevision:0,original,candidate,slot:'endmill'})).toBe(next);
    expect(workspaceReducer(state,{type:'apply-library',expectedRevision:0,original,candidate:original,slot:'endmill'})).toBe(state);
  });
  it('rejects stale candidate identities and any changes outside the selected tool', () => {
    const job=fixture(), candidate=structuredClone(job); candidate.tools[slotIndex(job,'endmill')].spindle_rpm=null;
    const selection={job,revision:8,documentFingerprint:'a'.repeat(64),expectedRevision:3,slot:'endmill' as const,toolId:'mill',presetId:null};
    const result:LibraryCandidate={apiVersion,engineVersion:'0.7.2',instanceId:'b'.repeat(32),requestId:'test',data:{libraryRevision:3,jobRevision:8,sourceFingerprint:selection.documentFingerprint,candidateFingerprint:'c'.repeat(64),slot:'endmill',toolId:'mill',presetId:null,job:candidate}};
    expect(acceptLibraryCandidate(result,selection)).toBe(result);
    for (const patch of [{libraryRevision:4},{jobRevision:7},{sourceFingerprint:'d'.repeat(64)},{toolId:'other'},{presetId:'other'},{slot:'vbit'}]) {
      expect(() => acceptLibraryCandidate({...result,data:{...result.data,...patch}} as LibraryCandidate,selection)).toThrow();
    }
    const other=structuredClone(result); other.data.job.operation.wall_allowance_mm=2;
    expect(() => acceptLibraryCandidate(other,selection)).toThrow(/unrelated/);
  });
  it('renders an accessible library entry without requiring a machining setup', async () => {
    const capabilities=await fixtureService.capabilities();
    const html=renderToStaticMarkup(<ToolLibraryDialog request={null} service={fixtureService} capabilities={capabilities} draft={newDraft(fixture())} revision={0} job={null} dispatch={() => {}} applied={() => {}} />);
    expect(html).toContain('aria-labelledby="library-title"'); expect(html).toContain('Reload library');
    expect(html).not.toContain('Create empty library');
  });
});
