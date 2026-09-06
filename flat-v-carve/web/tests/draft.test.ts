import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync } from 'node:fs';
import example from '../src/fixtures/inkscape.job.json';
import { jobSchema, parseJob } from '../src/contracts/job';
import { materialize, newDraft, parseNumeric } from '../src/state/draft';
import { initialWorkspace, workspaceReducer } from '../src/state/workspace';
import { defaultTolerances, defaultVbitComputation } from '../src/state/computation';

describe('portable job and draft boundary', () => {
  it('retains an incomplete imported job without inventing machining values', () => {
    const { job, errors } = materialize(newDraft(parseJob(example)));
    expect(errors).toEqual({});
    expect(job).toEqual({...example,tolerances:defaultTolerances,vbit_planning:defaultVbitComputation});
    expect(job?.stock.thickness_mm).toBeNull();
    expect(job?.tools.every(tool => tool.geometry === null && tool.spindle_rpm === null)).toBe(true);
  });
  it('keeps blank, zero, partial input, and boolean unset distinct', () => {
    expect(parseNumeric('')).toBeNull();
    expect(parseNumeric('   ')).toBeNull();
    expect(parseNumeric('0')).toBe(0);
    expect(parseNumeric('-')).toBe('invalid');
    expect(parseNumeric('1e')).toBe('invalid');
    expect(parseNumeric('0x10')).toBe('invalid');
    expect(parseNumeric('1,4')).toBe('invalid');
    expect(parseNumeric('Infinity')).toBe('invalid');
    expect(parseNumeric('1e309')).toBe('invalid');
    expect(parseNumeric(' -2.5e-2 ')).toBe(-.025);
    const draft = newDraft(parseJob(example));
    draft.text['tools.0.plunge_capable'] = 'false';
    expect(materialize(draft).job?.tools[0].plunge_capable).toBe(false);
    draft.text['tools.0.plunge_capable'] = '';
    expect(materialize(draft).job?.tools[0].plunge_capable).toBeNull();
  });
  it('does not download partial tool geometry or replace it with defaults', () => {
    const draft = newDraft(parseJob(example));
    draft.text['tools.0.geometry.dimensions.diameter_mm'] = '3.175';
    expect(materialize(draft).job).toBeNull();
    expect(materialize(draft).errors['tools.0.geometry.dimensions.cutting_length_mm']).toBeDefined();
    expect(draft.base.tools[0].geometry).toBeNull();
    draft.text['tools.0.geometry.dimensions.cutting_length_mm'] = '10';
    draft.text['tools.0.geometry.dimensions.plunge_capable'] = 'false';
    expect(materialize(draft).job?.tools[0].geometry).toEqual({ kind: 'endmill', dimensions: {
      diameter_mm: 3.175, cutting_length_mm: 10, plunge_capable: false,
    } });
    for (const field of Object.keys(draft.text)) draft.text[field] = '';
    expect(materialize(draft).job?.tools[0].geometry).toBeNull();
  });
  it('changes artwork scale without changing depth, tool, feed, or tolerance values', () => {
    const configured = JSON.parse(readFileSync(new URL('../../fixtures/m4/curved-medial.json', import.meta.url), 'utf8'));
    const draft = newDraft(parseJob(configured));
    draft.text['import.placement.scale'] = '2';
    const result = materialize(draft).job!;
    expect(result.import.placement.scale).toBe(2);
    expect(result.tools).toEqual(configured.tools);
    expect(result.operation).toEqual(configured.operation);
    expect(result.tolerances).toEqual(configured.tolerances);
    expect(result.endmill_planning).toEqual(configured.endmill_planning);
  });
  it('does not duplicate Rust machining constraints in structural validation', () => {
    const draft = newDraft(parseJob(example));
    draft.text['stock.thickness_mm'] = '-2';
    expect(materialize(draft).job?.stock.thickness_mm).toBe(-2);
    // This is a candidate awaiting the engine, never a validated setup.
  });
  it('preserves a machine snapshot and ramp entry through unrelated form edits', () => {
    const original = JSON.parse(readFileSync(new URL('../../fixtures/m4/curved-medial.json', import.meta.url), 'utf8'));
    original.machine_profile = { id: 'test-profile', work_offset: 'G55', clearance_z_mm: 6,
      endmill_tool_number: 8, vbit_tool_number: 12, m6_contract: 'Synthetic test snapshot; unvalidated M6 behavior' };
    original.endmill_planning.entry = { kind: 'ramp', max_angle_deg: 2, feed_mm_min: 80 };
    const draft = newDraft(parseJob(original));
    draft.text['stock.thickness_mm'] = '12';
    const result = materialize(draft).job!;
    expect(result.machine_profile).toEqual(original.machine_profile);
    expect(result.endmill_planning).toEqual(original.endmill_planning);
    expect(result.vbit_planning).toEqual(original.vbit_planning);
  });
  it('rejects unsupported schemas and injected UI/cache fields', () => {
    expect(() => parseJob({ ...example, schema_version: 99 })).toThrow(/schema 3/);
    expect(() => parseJob({ ...example, schema_version: 1 })).toThrow(/migration/);
    expect(jobSchema.safeParse({ ...example, verification_passed: true }).success).toBe(false);
    expect(jobSchema.safeParse({ ...example, stock: { ...example.stock, width_mm: 100 } }).success).toBe(false);
  });
  it('round trips every existing M4 job, including blocks with no U1 editor', () => {
    const directory = new URL('../../fixtures/m4/', import.meta.url);
    const files = readdirSync(directory).filter(file => file.endsWith('.json'));
    expect(files.length).toBeGreaterThan(5);
    for (const file of files) {
      const original = JSON.parse(readFileSync(new URL(file, directory), 'utf8'));
      const parsed = parseJob(original);
      expect(JSON.parse(JSON.stringify(materialize(newDraft(parsed)).job)), file).toEqual(original);
    }
  });
});

describe('edit history and revisions', () => {
  it('groups typing, preserves invalid text, and gives undo/redo fresh revisions', () => {
    const initial = initialWorkspace(newDraft(parseJob(example)));
    const first = workspaceReducer(initial, { type: 'text', path: 'stock.thickness_mm', value: '1' });
    const partial = workspaceReducer(first, { type: 'text', path: 'stock.thickness_mm', value: '1e' });
    const committed = workspaceReducer(partial, { type: 'commit' });
    expect(materialize(committed.draft).job).toBeNull();
    expect(committed.past).toHaveLength(1);
    const undo = workspaceReducer(committed, { type: 'undo' });
    expect(undo.draft).toEqual(initial.draft);
    const redo = workspaceReducer(undo, { type: 'redo' });
    expect(redo.draft.text['stock.thickness_mm']).toBe('1e');
    expect(redo.revision).toBeGreaterThan(undo.revision);
    expect(undo.revision).toBeGreaterThan(committed.revision);
  });
  it('a metadata edit invalidates the downloaded revision and clears redo', () => {
    let state = initialWorkspace(newDraft(parseJob(example)));
    state = workspaceReducer(state, { type: 'downloaded' });
    const downloaded = state.revision;
    state = workspaceReducer(state, { type: 'name', value: 'New name' });
    state = workspaceReducer(state, { type: 'commit' });
    expect(state.revision).toBeGreaterThan(downloaded);
    state = workspaceReducer(state, { type: 'undo' });
    state = workspaceReducer(state, { type: 'include', ids: [] });
    expect(state.future).toHaveLength(0);
    expect(state.draft.base.selected_region_ids).toEqual([]);
    expect(materialize(state.draft).job).not.toBeNull();
  });
  it('opening another job can be undone without losing unfinished fields', () => {
    let state = initialWorkspace(newDraft(parseJob(example)));
    state = workspaceReducer(state, { type: 'text', path: 'operation.max_depth_mm', value: '-' });
    state = workspaceReducer(state, { type: 'replace', draft: newDraft(parseJob({ ...example, name: 'Another job' })) });
    state = workspaceReducer(state, { type: 'undo' });
    expect(state.draft.text['operation.max_depth_mm']).toBe('-');
  });
});
