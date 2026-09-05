import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { parseJob } from '../src/contracts/job';
import { acceptLibraryCandidate, libraryToolSchema, cuttingPresetSchema, toolLibrarySchema, slotIndex, type LibraryCandidate } from '../src/contracts/library';
import { libraryText, libraryToolFields, parseLibraryTool, parseLibraryPreset, toolChanges, presetFields } from '../src/state/library';
import { newDraft, materialize } from '../src/state/draft';
import { initialWorkspace, workspaceReducer } from '../src/state/workspace';
import { ToolLibraryDialog } from '../src/components/ToolLibraryDialog';
import { fixtureService } from '../src/service/fixture';
import { apiVersion } from '../src/contracts/wire';

const fixture = () => parseJob(JSON.parse(readFileSync(new URL('../../fixtures/m4/finite-tip.json', import.meta.url), 'utf8')));
const fields = ['spindle_rpm','cutting_feed_mm_min','plunge_feed_mm_min','max_stepdown_mm','stepover_mm'] as const;
describe('library records and job boundary', () => {
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
    const html=renderToStaticMarkup(<ToolLibraryDialog request={null} service={fixtureService} capabilities={capabilities} job={null} dispatch={() => {}} applied={() => {}} />);
    expect(html).toContain('aria-labelledby="library-title"'); expect(html).toContain('Reload library');
    expect(html).not.toContain('Create empty library');
  });
});
