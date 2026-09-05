import { useEffect, useRef, useState, type Dispatch } from 'react';
import type { CamService, Capabilities } from '../contracts/service';
import { toolLibrarySchema, slotIndex, type LibraryCandidate, type LibraryConnection, type LibraryJobInput, type LibrarySelection,
  type LibrarySnapshot, type LibraryTool, type CuttingPreset, type ToolSlot, type LibraryChange } from '../contracts/library';
import { libraryText, libraryToolFields, presetFields, parseLibraryTool, parseLibraryPreset, toolChanges,
  displayLibraryValue, type LibraryFields, type LibraryField } from '../state/library';
import type { WorkspaceAction } from '../state/workspace';
import { containDialogFocus } from './dialogFocus';
import { downloadText } from './ExportPanel';
import { readPath } from '../state/draft';

export interface LibraryOpen { serial: number; slot?: ToolSlot }
type Mode = 'tool' | 'preset' | 'duplicate-tool' | 'duplicate-preset' | 'remove-tool' | 'remove-preset' | 'import' | 'capture';
interface Editor {
  mode: Mode; connection: LibraryConnection; revision: number; text: LibraryFields; geometry: ToolSlot;
  tool?: LibraryTool; preset?: CuttingPreset; json?: string; source?: LibraryJobInput; includePreset: boolean;
}
const identity = (s: LibrarySnapshot): LibraryConnection => ({ instanceId: s.instanceId, engineVersion: s.engineVersion });
const newId = () => crypto.randomUUID();
const metadataFields = (preset = false): LibraryField[] => preset ? presetFields.slice(0, 2) : libraryToolFields('endmill').slice(0, 2);
function FormFields({ fields, text, lockedId, update }: { fields: LibraryField[]; text: LibraryFields; lockedId?: boolean; update: (path: string, value: string) => void }) {
  return <div className="fields">{fields.map(f => <label className="field" key={f.path}><span>{f.label}{f.required ? ' *' : ''}</span>
    {f.kind === 'boolean' ? <select value={text[f.path] ?? ''} onChange={e => update(f.path, e.target.value)}>
      <option value="">Not specified</option><option value="true">Yes</option><option value="false">No</option>
    </select> : <input value={text[f.path] ?? ''} disabled={lockedId && f.path === 'id'} inputMode={f.kind === 'number' ? 'decimal' : undefined}
      autoComplete="off" onChange={e => update(f.path, e.target.value)} />}
  </label>)}</div>;
}

export function ToolLibraryDialog({ request, service, capabilities, job, dispatch, applied }: {
  request: LibraryOpen | null; service: CamService; capabilities: Capabilities; job: LibraryJobInput | null;
  dispatch: Dispatch<WorkspaceAction>; applied: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [snapshot, setSnapshot] = useState<LibrarySnapshot | null>(null);
  const [busy, setBusy] = useState(false), busyRef = useRef(false);
  const [error, setError] = useState(''), [message, setMessage] = useState('');
  const [query, setQuery] = useState(''), [filter, setFilter] = useState<'all' | ToolSlot>('all'), [limit, setLimit] = useState(50);
  const [selected, setSelected] = useState(''), [slot, setSlot] = useState<ToolSlot>('endmill'), [presetId, setPresetId] = useState('');
  const [editor, setEditor] = useState<Editor | null>(null);
  const [review, setReview] = useState<{ connection: LibraryConnection; selection: LibrarySelection; result: LibraryCandidate } | null>(null);
  const latestJob = useRef(job); latestJob.current = job;
  const connection: LibraryConnection | null = capabilities.planning && capabilities.toolLibrary
    ? { instanceId: capabilities.planning.instanceId, engineVersion: capabilities.engineVersion } : null;
  const connected = !!snapshot && snapshot.instanceId === connection?.instanceId && snapshot.engineVersion === connection.engineVersion;
  const library = connected ? snapshot.data.library : null;
  const tool = library?.tools.find(t => t.id === selected);
  const editorCurrent = !!editor && connected && editor.connection.instanceId === snapshot.instanceId
    && editor.connection.engineVersion === snapshot.engineVersion && editor.revision === library?.revision;
  async function run(action: () => Promise<void>) {
    if (busyRef.current) return;
    busyRef.current = true; setBusy(true); setError(''); setMessage('');
    try { await action(); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { busyRef.current = false; setBusy(false); }
  }
  function reload() { void run(async () => {
    if (!connection || !service.library) throw new Error('Reconnect to a service with tool library support.');
    setSnapshot(await service.library(connection)); setReview(null);
  }); }
  const openRef = useRef(reload); openRef.current = reload;
  useEffect(() => {
    if (!request) return;
    if (request.slot) { setSlot(request.slot); setFilter(request.slot); }
    dialog.current?.showModal(); openRef.current();
  }, [request]);
  function start(mode: Mode, owner?: LibraryTool, preset?: CuttingPreset, json?: string) {
    if (!library || !snapshot) return;
    const geometry = owner?.geometry.kind ?? slot;
    const fields = mode === 'preset' ? presetFields : libraryToolFields(geometry);
    let text = libraryText(preset ?? owner ?? {}, fields);
    if (mode.startsWith('duplicate')) text = { id: newId(), name: `${preset?.name ?? owner?.name ?? ''} copy` };
    if (!owner || (mode === 'preset' && !preset)) text = { ...text, id: newId(), name: '' };
    setEditor({ mode, connection: identity(snapshot), revision: library.revision, text, geometry, tool: owner, preset,
      json, source: mode === 'capture' && job ? structuredClone(job) : undefined, includePreset: false });
    setReview(null); setError(''); setMessage('');
  }
  function update(path: string, value: string) { setEditor(e => e && ({ ...e, text: { ...e.text, [path]: value } })); }
  const parsedTool = editor?.mode === 'tool' ? parseLibraryTool(editor.text, editor.geometry, editor.tool?.cutting_presets) : null;
  const parsedPreset = editor?.mode === 'preset' ? parseLibraryPreset(editor.text) : null;
  const basicValid = !!editor?.text.id?.match(/^[A-Za-z0-9_-]{1,100}$/) && !!editor.text.name?.trim();
  const canSave = !!editor && editorCurrent && !busy && (editor.mode === 'tool' ? !!parsedTool : editor.mode === 'preset' ? !!parsedPreset
    : editor.mode === 'import' || editor.mode.startsWith('remove') ? true : basicValid && (editor.mode !== 'capture' || (!!editor.source
      && (!editor.includePreset || (!!editor.text.presetId?.match(/^[A-Za-z0-9_-]{1,100}$/) && !!editor.text.presetName?.trim())))));
  async function save() {
    if (!editor || !canSave || !service.changeLibrary) return;
    await run(async () => {
      const e = editor; let result: LibrarySnapshot;
      if (e.mode === 'import') result = await service.importLibrary!(e.connection, e.revision, e.json!);
      else if (e.mode === 'capture') result = await service.captureLibraryTool!(e.connection, {
        ...e.source!, expectedRevision: e.revision, slot: e.geometry, toolId: e.text.id, name: e.text.name,
        preset: e.includePreset ? { id: e.text.presetId ?? '', name: e.text.presetName ?? '', material: e.text.material || null, machine: e.text.machine || null } : null,
      });
      else {
        let change: LibraryChange;
        switch (e.mode) {
          case 'tool': change = { kind: e.tool ? 'replace_tool' : 'add_tool', tool: parsedTool! }; break;
          case 'preset': change = { kind: e.preset ? 'replace_preset' : 'add_preset', tool_id: e.tool!.id, preset: parsedPreset! }; break;
          case 'duplicate-tool': change = { kind: 'duplicate_tool', tool_id: e.tool!.id, new_id: e.text.id, name: e.text.name }; break;
          case 'duplicate-preset': change = { kind: 'duplicate_preset', tool_id: e.tool!.id, preset_id: e.preset!.id, new_id: e.text.id, name: e.text.name }; break;
          case 'remove-tool': change = { kind: 'remove_tool', tool_id: e.tool!.id }; break;
          case 'remove-preset': change = { kind: 'remove_preset', tool_id: e.tool!.id, preset_id: e.preset!.id }; break;
        }
        result = await service.changeLibrary!(e.connection, e.revision, change);
      }
      setSnapshot(result); setEditor(null); setReview(null); setPresetId('');
      setSelected(e.mode === 'tool' || e.mode === 'duplicate-tool' || e.mode === 'capture' ? e.text.id : e.tool?.id ?? '');
      setMessage('Library saved. Existing jobs keep their current tool snapshots.');
    });
  }
  async function preview() {
    if (!job || !tool || !library || !snapshot || !service.applyLibraryTool) return;
    const selection: LibrarySelection = { ...structuredClone(job), expectedRevision: library.revision, slot, toolId: tool.id, presetId: presetId || null };
    const conn = identity(snapshot);
    await run(async () => { const result = await service.applyLibraryTool!(conn, selection); setReview({ connection: conn, selection, result }); });
  }
  async function apply() {
    if (!review || !service.applyLibraryTool) return;
    await run(async () => {
      const { selection } = review;
      const result = await service.applyLibraryTool!(review.connection, selection);
      if (latestJob.current?.revision !== selection.revision || latestJob.current.documentFingerprint !== selection.documentFingerprint)
        throw new Error('The job changed. Review the selection again before applying.');
      if (result.data.candidateFingerprint !== review.result.data.candidateFingerprint) throw new Error('The candidate changed. Review the selection again.');
      dispatch({ type: 'apply-library', expectedRevision: selection.revision, original: selection.job, candidate: result.data.job, slot: selection.slot });
      setReview(null); applied(); dialog.current?.close();
    });
  }
  async function importFile(file?: File) {
    if (!file) return;
    await run(async () => {
      if (file.size > (capabilities.toolLibrary?.maxBytes ?? 0)) throw new Error('This file exceeds the tool library size limit.');
      const json = await file.text(); toolLibrarySchema.parse(JSON.parse(json)); start('import', undefined, undefined, json);
    });
  }
  const visible = library?.tools.filter(t => (filter === 'all' || t.geometry.kind === filter)
    && [t.id, t.name, ...t.cutting_presets.flatMap(p => [p.name, p.material ?? '', p.machine ?? ''])].join(' ').toLowerCase().includes(query.toLowerCase())) ?? [];
  const changes = review ? toolChanges(review.selection.job, review.result.data.job, review.selection.slot) : [];
  const reviewCurrent = !!review && review.selection.revision === job?.revision && review.selection.documentFingerprint === job.documentFingerprint
    && review.selection.expectedRevision === library?.revision && review.connection.instanceId === connection?.instanceId;
  const title = !editor ? '' : ({ tool: editor.tool ? 'Edit tool' : 'New tool', preset: editor.preset ? 'Edit preset' : 'New preset',
    'duplicate-tool': 'Duplicate tool', 'duplicate-preset': 'Duplicate preset', 'remove-tool': 'Remove tool', 'remove-preset': 'Remove preset',
    import: 'Import library', capture: 'Save job tool to library' })[editor.mode];
  return <dialog ref={dialog} className="library-dialog" aria-labelledby="library-title" onKeyDown={containDialogFocus}>
    <div className="dialog-heading"><h2 id="library-title">Tool library</h2><button onClick={() => dialog.current?.close()} aria-label="Close tool library">×</button></div>
    <p className="hint library-location">{capabilities.toolLibrary?.location ?? 'Tool library unavailable'}{library && ` · Revision ${library.revision}`}</p>
    <p className="hint">Library records are reusable snapshots. Applying a selection updates one job tool and can be undone. Library edits are saved separately.</p>
    <div className="inline-actions"><button disabled={busy} onClick={reload}>Reload library</button>
      {library && <><button disabled={busy || !!editor} onClick={() => start('tool')}>New tool</button>
        <label className="library-import">Import JSON<input type="file" accept=".json,application/json" disabled={busy || !!editor} onChange={e => { void importFile(e.target.files?.[0]); e.target.value = ''; }} /></label>
        <button disabled={busy} onClick={() => { downloadText(`tool-library-r${library.revision}.json`, JSON.stringify(library, null, 2) + '\n', 'application/json'); setMessage('Library download requested. Check browser downloads to confirm it was saved.'); }}>Download library</button></>}
    </div>
    {busy && <p role="status">Working with the local library…</p>}{error && <p role="alert" className="inline-warning">{error} Reload before retrying a write; a lost response may still have saved.</p>}
    {message && <p role="status">{message}</p>}
    {connected && snapshot.data.state === 'missing' && <section className="inspector-group"><h3>No library at this location</h3><p>Create an empty library, then add tools or import your records.</p>
      <button disabled={busy} onClick={() => void run(async () => setSnapshot(await service.initializeLibrary!(connection!)))}>Create empty library</button></section>}
    {library && <div className="library-layout"><section aria-label="Library records">
      <label className="field"><span>Search tools and presets</span><input value={query} onChange={e => { setQuery(e.target.value); setLimit(50); }} /></label>
      <label className="field"><span>Cutter filter</span><select value={filter} onChange={e => { setFilter(e.target.value as typeof filter); setLimit(50); }}><option value="all">All cutters</option><option value="endmill">Endmills</option><option value="vbit">V-bits</option></select></label>
      <p className="hint">{visible.length} tools</p><div className="library-list">{visible.slice(0, limit).map(t => <button key={t.id} disabled={busy || !!editor} aria-pressed={selected === t.id}
        onClick={() => { setSelected(t.id); setPresetId(''); setReview(null); }}><strong>{t.name}</strong><small>{t.id} · {t.geometry.kind} · {t.cutting_presets.length} presets</small></button>)}</div>
      {visible.length > limit && <button onClick={() => setLimit(n => n + 50)}>Show more tools</button>}
      {!library.tools.length && <p className="hint">No tools saved yet.</p>}
      <div className="dialog-divider">CURRENT JOB</div><label className="field"><span>Job tool slot</span><select value={slot} disabled={busy || !!editor} onChange={e => { setSlot(e.target.value as ToolSlot); setReview(null); }}><option value="endmill">Endmill clearing</option><option value="vbit">V-bit rest & finish</option></select></label>
      <button disabled={busy || !!editor || !job || !job.job.tools[slotIndex(job.job, slot)]?.geometry} onClick={() => start('capture')}>Save job tool to library</button>
      {!job && <p className="hint">Finish the job fields and wait for Rust validation to capture or apply a tool.</p>}
    </section><section aria-label="Tool details" className="library-details">
      {editor ? <fieldset className="library-form" disabled={busy}><legend>{title}</legend><p className="hint">Editing library revision {editor.revision}. Closing this dialog keeps unfinished fields until this tab reloads.</p>
        {!editorCurrent && <p role="alert" className="inline-warning">The library changed. Your fields have been kept. Discard this edit and open the latest record before saving.</p>}
        {editor.mode === 'tool' && <><label className="field"><span>Cutter type</span><select disabled={!!editor.tool || busy} value={editor.geometry} onChange={e => setEditor({ ...editor, geometry: e.target.value as ToolSlot })}><option value="endmill">Endmill</option><option value="vbit">V-bit</option></select></label>
          <FormFields fields={libraryToolFields(editor.geometry)} text={editor.text} lockedId={!!editor.tool} update={update} /><p className="hint">All geometry fields are required. Nullable capabilities may be left unspecified. Existing presets are preserved.</p></>}
        {editor.mode === 'preset' && <><FormFields fields={presetFields} text={editor.text} lockedId={!!editor.preset} update={update} /><p className="hint">Blank cutting values mean not specified. Context labels describe intended use and do not validate a material or machine.</p></>}
        {editor.mode.startsWith('duplicate') && <FormFields fields={metadataFields(editor.mode === 'duplicate-preset')} text={editor.text} update={update} />}
        {editor.mode.startsWith('remove') && <p>Remove “{editor.preset?.name ?? editor.tool?.name}”{editor.mode === 'remove-tool' ? ' and all its presets' : ''} from the library? Existing jobs keep their snapshots.</p>}
        {editor.mode === 'import' && <p>Merge {toolLibrarySchema.parse(JSON.parse(editor.json!)).tools.length} tools into this library. Any duplicate tool ID rejects the entire import.</p>}
        {editor.mode === 'capture' && <><p>Capture the {editor.geometry} from job revision {editor.source?.revision}. Geometry and capabilities will be copied.</p>
          <FormFields fields={metadataFields()} text={editor.text} update={update} /><label className="library-checkbox"><input type="checkbox" checked={editor.includePreset} onChange={e => setEditor({ ...editor, includePreset: e.target.checked, text: { ...editor.text, presetId: editor.text.presetId ?? newId() } })} />Include cutting settings as a preset</label>
          {editor.includePreset && <FormFields fields={[{ path: 'presetId', label: 'Preset ID', required: true }, { path: 'presetName', label: 'Preset name', required: true }, ...presetFields.slice(2, 4)]} text={editor.text} update={update} />}
        </>}
        {(!canSave && editorCurrent && !busy) && <p className="hint">Complete the required fields with valid values. IDs use letters, numbers, underscores, or hyphens. Rust checks geometry and capabilities when saving.</p>}
        <div className="inline-actions"><button disabled={!canSave} onClick={() => void save()}>{editor.mode.startsWith('remove') ? 'Confirm removal' : editor.mode === 'import' ? 'Confirm import' : 'Save to library'}</button><button disabled={busy} onClick={() => { setEditor(null); setError(''); }}>Discard edit</button></div>
      </fieldset> : tool ? <><h3>{tool.name}</h3><p className="hint">{tool.id}</p><table className="stock-metrics"><caption>Tool geometry and capabilities</caption><tbody>{libraryToolFields(tool.geometry.kind).slice(2).map(f => <tr key={f.path}><th>{f.label}</th><td>{displayLibraryValue(readPath(tool, f.path))}</td></tr>)}</tbody></table>
        <div className="inline-actions"><button disabled={busy} onClick={() => start('tool', tool)}>Edit tool</button><button disabled={busy} onClick={() => start('duplicate-tool', tool)}>Duplicate tool</button><button disabled={busy} onClick={() => start('remove-tool', tool)}>Remove tool</button></div>
        <h3>Cutting presets</h3>{tool.cutting_presets.map(p => <details key={p.id} className="library-preset"><summary>{p.name}</summary><p className="hint">{p.id}</p><table className="stock-metrics"><tbody>{presetFields.slice(2).map(f => <tr key={f.path}><th>{f.label}</th><td>{displayLibraryValue(p[f.path as keyof CuttingPreset])}</td></tr>)}</tbody></table>
          <div className="inline-actions"><button disabled={busy} onClick={() => start('preset', tool, p)}>Edit preset</button><button disabled={busy} onClick={() => start('duplicate-preset', tool, p)}>Duplicate preset</button><button disabled={busy} onClick={() => start('remove-preset', tool, p)}>Remove preset</button></div></details>)}
        <button disabled={busy} onClick={() => start('preset', tool)}>Add cutting preset</button>
        <div className="dialog-divider">APPLY TO CURRENT JOB</div><p>Target: {slot === 'endmill' ? 'Endmill clearing' : 'V-bit rest & finish'}</p>
        <label className="field"><span>Cutting preset</span><select value={presetId} disabled={busy} onChange={e => { setPresetId(e.target.value); setReview(null); }}><option value="">Geometry only — clear cutting settings</option>{tool.cutting_presets.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}</select></label>
        <p className="hint">{presetId ? 'The selected preset replaces all five cutting values, including blanks.' : 'Geometry-only selection clears spindle speed, cutting feed, plunge feed, stepdown, and stepover.'} Job tool IDs and machine mapping stay the same.</p>
        {tool.geometry.kind !== slot && <p className="inline-warning">This cutter does not match the selected job slot.</p>}
        <button disabled={busy || !job || tool.geometry.kind !== slot} onClick={() => void preview()}>Review job changes</button>
        {review && <section aria-label="Review job changes"><h3>Review changes</h3><p className="hint">Library revision {review.selection.expectedRevision} · Job revision {review.selection.revision}</p>
          <table className="stock-metrics library-changes"><thead><tr><th>Setting</th><th>Current job</th><th>After selection</th></tr></thead><tbody>{changes.map(c => <tr key={c.label}><th>{c.label}</th><td>{c.before}</td><td>{c.after}</td></tr>)}</tbody></table>
          {!changes.length && <p>No job values would change.</p>}{!reviewCurrent && <p role="alert">This review is stale. Review the selection again.</p>}
          <button disabled={busy || !reviewCurrent || !changes.length} onClick={() => void apply()}>Apply reviewed changes</button></section>}
      </> : <p className="hint">Select a tool to inspect its geometry, manage presets, or review changes to your job.</p>}
    </section></div>}
  </dialog>;
}
