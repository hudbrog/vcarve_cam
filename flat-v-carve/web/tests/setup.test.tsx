import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import example from '../src/fixtures/inkscape.job.json';
import { parseJob } from '../src/contracts/job';
import { allFields, endmillPlanningFields, fieldStep, materialize, newDraft, readPath, setupGroups,
  setupWarnings, strategyFields, vbitPlanningFields, type Draft } from '../src/state/draft';
import { initialWorkspace, workspaceReducer } from '../src/state/workspace';
import { Fields } from '../src/components/Fields';
import { readRecovery } from '../src/App';
import { defaultEndmillBudgets, defaultVbitComputation, defaultTolerances } from '../src/state/computation';

const configured = parseJob(JSON.parse(readFileSync(new URL('../../fixtures/m4/curved-medial.json', import.meta.url), 'utf8')));
function completeBlock(draft: Draft, root: string) {
  const fields = setupGroups.find(group => group.path === root)!.fields;
  for (const field of fields) {
    const value = readPath(configured, field.path);
    if (value !== undefined && value !== null) draft.text[field.path] = String(value);
  }
}
describe('optional planning editors', () => {
  it('supplies computation defaults on import while leaving travel and machine setup unset', () => {
    const result = materialize(newDraft(parseJob(example)));
    expect(result.job?.endmill_planning).toBeNull();
    expect(result.job?.vbit_planning).toEqual(defaultVbitComputation);
    expect(result.job?.tolerances).toEqual(defaultTolerances);
    expect(result.job?.machine_profile).toBeNull();
  });
  it('retains partial planning separately and identifies missing settings by editable path', () => {
    const draft = newDraft(parseJob(example));
    draft.text['endmill_planning.clearance_z_mm'] = '5';
    const result = materialize(draft);
    expect(result.job).toBeNull();
    expect(result.errors['endmill_planning.entry.kind']).toMatch(/Complete/);
    expect(result.errors['endmill_planning.max_layers']).toBeUndefined();
    const knownPaths = new Set(allFields(draft.base).map(field => field.path));
    expect(Object.keys(result.errors).every(path => knownPaths.has(path))).toBe(true);
    expect(readRecovery({ getItem: () => JSON.stringify({ version: 1, draft }) })).toEqual(draft);
  });
  it('creates both planning blocks entirely from explicit input', () => {
    const draft = newDraft(parseJob(example));
    completeBlock(draft, 'endmill_planning'); completeBlock(draft, 'vbit_planning');
    const result = materialize(draft);
    expect(result.errors).toEqual({});
    expect(result.job?.endmill_planning).toEqual(configured.endmill_planning);
    expect(result.job?.vbit_planning).toEqual(configured.vbit_planning);
    expect(result.job?.tools[0].geometry).toBeNull();
  });
  it('retains inactive ramp text but excludes it and its errors from a plunge job', () => {
    const draft = newDraft(configured);
    draft.text['endmill_planning.entry.kind'] = 'ramp';
    draft.text['endmill_planning.entry.max_angle_deg'] = '1e-';
    draft.text['endmill_planning.entry.feed_mm_min'] = '80';
    expect(materialize(draft).job).toBeNull();
    draft.text['endmill_planning.entry.kind'] = 'plunge';
    expect(materialize(draft).errors).toEqual({});
    expect(materialize(draft).job?.endmill_planning?.entry).toEqual({ kind: 'plunge' });
    draft.text['endmill_planning.entry.kind'] = 'ramp';
    expect(materialize(draft).errors['endmill_planning.entry.max_angle_deg']).toBe('Enter a finite number.');
    draft.text['endmill_planning.entry.max_angle_deg'] = '2';
    expect(materialize(draft).job?.endmill_planning?.entry).toEqual({ kind: 'ramp', max_angle_deg: 2, feed_mm_min: 80 });
  });
  it('restores imported ramp parameters when switching away and back', () => {
    const ramp = structuredClone(configured);
    ramp.endmill_planning!.entry = { kind: 'ramp', max_angle_deg: 3, feed_mm_min: 90 };
    const draft = newDraft(ramp);
    draft.text['endmill_planning.entry.kind'] = 'plunge';
    expect(materialize(draft).job?.endmill_planning?.entry).toEqual({ kind: 'plunge' });
    draft.text['endmill_planning.entry.kind'] = 'ramp';
    expect(materialize(draft).job?.endmill_planning?.entry).toEqual(ramp.endmill_planning!.entry);
  });
  it.each(['2.5', '-1', '9007199254740992', '1e309'])('rejects unrepresentable integer resource input %s', raw => {
    const draft = newDraft(configured); draft.text['vbit_planning.max_paths'] = raw;
    expect(materialize(draft).job).toBeNull();
    expect(materialize(draft).errors['vbit_planning.max_paths']).toBeDefined();
  });
  it('keeps explicit zero cleanup iterations distinct from using the default', () => {
    const draft = newDraft(configured); draft.text['vbit_planning.max_cleanup_iterations'] = '0';
    expect(materialize(draft).job?.vbit_planning?.max_cleanup_iterations).toBe(0);
    draft.text['vbit_planning.max_cleanup_iterations'] = '';
    expect(materialize(draft).job?.vbit_planning?.max_cleanup_iterations).toBe(defaultVbitComputation.max_cleanup_iterations);
  });
  it('clears a complete block atomically and restores it with one undo', () => {
    let state = initialWorkspace(newDraft(configured));
    state = workspaceReducer(state, { type: 'clear-fields', paths: endmillPlanningFields.map(field => field.path) });
    expect(materialize(state.draft).job?.endmill_planning).toBeNull();
    state = workspaceReducer(state, { type: 'undo' });
    expect(materialize(state.draft).job?.endmill_planning).toEqual(configured.endmill_planning);
    expect(state.revision).toBe(2);
  });
  it('continues rendering readable placement while other settings are incomplete', () => {
    const draft = newDraft(parseJob(example));
    draft.text['import.placement.scale'] = '2';
    draft.text['endmill_planning.clearance_z_mm'] = '5';
    const result = materialize(draft);
    expect(result.job).toBeNull();
    expect(result.previewJob.import.placement.scale).toBe(2);
    expect(result.previewJob.tools).toEqual(draft.base.tools);
  });
});

describe('machine profile and field presentation', () => {
  it('requires an explicit profile ID and preserves multiline prose literally', () => {
    const draft = newDraft(configured);
    draft.text['machine_profile.work_offset'] = 'G55';
    expect(materialize(draft).errors['machine_profile.id']).toBeDefined();
    draft.text['machine_profile.id'] = 'synthetic-profile';
    draft.text['machine_profile.m6_contract'] = 'First line\n<description> & text';
    expect(materialize(draft).job?.machine_profile).toEqual({
      id: 'synthetic-profile', work_offset: 'G55', clearance_z_mm: null, endmill_tool_number: null,
      vbit_tool_number: null, m6_contract: 'First line\n<description> & text',
    });
  });
  it('warns on unequal clearances without rewriting either field or blocking a draft download', () => {
    const draft = newDraft(configured);
    draft.text['machine_profile.id'] = 'synthetic-profile';
    draft.text['machine_profile.clearance_z_mm'] = '6';
    expect(setupWarnings(draft)).toHaveLength(1);
    expect(materialize(draft).job?.machine_profile?.clearance_z_mm).toBe(6);
    expect(materialize(draft).job?.endmill_planning?.clearance_z_mm).toBe(5);
    draft.text['machine_profile.clearance_z_mm'] = '5';
    expect(setupWarnings(draft)).toEqual([]);
  });
  it('preserves untouched empty profile prose when a different field is edited', () => {
    const input = structuredClone(configured);
    input.machine_profile = { id: 'profile', work_offset: null, m6_contract: '', clearance_z_mm: null, endmill_tool_number: null, vbit_tool_number: null };
    const draft = newDraft(input); draft.text['machine_profile.endmill_tool_number'] = '8';
    expect(materialize(draft).job?.machine_profile?.m6_contract).toBe('');
  });
  it('routes every setup error to the correct inspector step', () => {
    expect(fieldStep('endmill_planning.clearance_z_mm')).toBe('stock');
    expect(fieldStep('endmill_planning.entry.kind')).toBe('tools');
    expect(fieldStep('vbit_planning.stock_slices')).toBe('tools');
    expect(fieldStep('machine_profile.id')).toBe('export');
  });
  it('exposes units to assistive technology and omits inactive ramp controls', () => {
    const draft = newDraft(configured);
    const markup = renderToStaticMarkup(<Fields fields={[...strategyFields, ...vbitPlanningFields]} draft={draft} errors={{}} dispatch={() => {}} />);
    expect(markup).not.toContain('Maximum ramp angle');
    expect(markup).toContain('aria-describedby="vbit_planning.quality_sample_spacing_mm-unit vbit_planning.quality_sample_spacing_mm-default vbit_planning.quality_sample_spacing_mm-hint"');
  });
  it('makes travel and entry usable without filling any budget fields',()=>{
    const draft=newDraft(parseJob(example));
    draft.text['endmill_planning.clearance_z_mm']='5';
    draft.text['endmill_planning.start_xy_mm.x']='0'; draft.text['endmill_planning.start_xy_mm.y']='0';
    draft.text['endmill_planning.strategy']='depth_dependent'; draft.text['endmill_planning.entry.kind']='plunge';
    const result=materialize(draft);
    expect(result.errors).toEqual({}); expect(result.job?.endmill_planning).toMatchObject(defaultEndmillBudgets);
    expect(result.job?.tools).toEqual(example.tools); expect(result.job?.operation).toEqual(example.operation);
  });
  it('shows the effective defaults and describes them to assistive technology',()=>{
    const draft=newDraft(parseJob(example));
    const markup=renderToStaticMarkup(<Fields fields={vbitPlanningFields} draft={draft} errors={{}} dispatch={()=>{}} />);
    expect(markup).toContain('Using default: 1,000,000 motions');
    expect(markup).toContain('placeholder="1000000"');
    expect(markup).toContain('Bounds the total V-bit motion list kept in memory.');
  });
});
