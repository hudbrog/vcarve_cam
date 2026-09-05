import { useEffect, useMemo, useReducer, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { parseJob } from './contracts/job';
import { outputBlockedReasons, type ArtworkDisplay, type CamService, type Capabilities } from './contracts/service';
import { fixtureService } from './service/fixture';
import { allFields, fieldStep, materialize, newDraft, setupWarnings, type Draft } from './state/draft';
import { initialWorkspace, workspaceReducer } from './state/workspace';
import { Fields } from './components/Fields';
import { Viewport } from './components/Viewport';
import { MachineSetup, StockSetup, ToolsSetup } from './components/SetupEditors';
import { containDialogFocus } from './components/dialogFocus';

const steps = [
  { id: 'artwork', label: 'Artwork', description: 'Source & regions', number: '01' },
  { id: 'stock', label: 'Stock & origin', description: 'Material & placement', number: '02' },
  { id: 'tools', label: 'Carve & tools', description: 'Shape & cutting setup', number: '03' },
  { id: 'plan', label: 'Plan & inspect', description: 'Paths & remaining stock', number: '04' },
  { id: 'verify', label: 'Verification', description: 'Evidence & limits', number: '05' },
  { id: 'export', label: 'Export', description: 'Machine program & job', number: '06' },
] as const;
type Step = typeof steps[number]['id'];
type Theme = 'system' | 'light' | 'dark';
const recoveryKey = 'flat-v-carve:u1:tab-draft';

export function readRecovery(storage: Pick<Storage, 'getItem'>): Draft | null {
  let raw: string | null;
  try { raw = storage.getItem(recoveryKey); } catch { return null; }
  if (!raw) return null;
  const data = JSON.parse(raw);
  if (data.version !== 1 || !data.draft || typeof data.draft.text !== 'object' || data.draft.text === null || Array.isArray(data.draft.text)) throw new Error('Unsupported recovery draft.');
  const base = parseJob(data.draft.base);
  const allowed = new Set(allFields(base).map(field => field.path));
  for (const [path, value] of Object.entries(data.draft.text)) if (!allowed.has(path) || typeof value !== 'string') throw new Error('Invalid recovery field.');
  return { base, text: data.draft.text };
}
function Group({ title, children }: { title: string; children: ReactNode }) {
  return <section className="inspector-group"><h2>{title}</h2>{children}</section>;
}

export function App({ service = fixtureService }: { service?: CamService }) {
  const [boot, setBoot] = useState<{ draft: Draft; capabilities: Capabilities; recovered: boolean } | null>(null);
  const [bootError, setBootError] = useState('');
  const [skipRecovery, setSkipRecovery] = useState(false);
  useEffect(() => {
    const controller = new AbortController();
    setBootError('');
    Promise.all([service.capabilities(controller.signal), service.openExample(controller.signal)]).then(([capabilities, example]) => {
      if (controller.signal.aborted) return;
      let recovered: Draft | null = null;
      if (!skipRecovery) {
        try { recovered = readRecovery({ getItem: key => sessionStorage.getItem(key) }); }
        catch { throw new Error('This tab’s recovery draft could not be read. It has been kept unchanged. Retry, or explicitly replace it with the bundled example.'); }
      }
      setBoot({ draft: recovered ?? newDraft(example.job), capabilities, recovered: !!recovered });
    }).catch(error => { if (!controller.signal.aborted) setBootError(String(error)); });
    return () => controller.abort();
  }, [service, skipRecovery]);
  if (bootError) return <main className="startup"><h1>Cannot open the workspace</h1><p role="alert">{bootError}</p><div className="inline-actions"><button onClick={() => location.reload()}>Retry</button><button onClick={() => setSkipRecovery(true)}>Open example (replace recovery)</button></div></main>;
  if (!boot) return <main className="startup" role="status">Opening the CAM workspace…</main>;
  return <Workspace key={boot.capabilities.engineVersion} initial={boot.draft} recovered={boot.recovered} service={service} capabilities={boot.capabilities} />;
}

function Workspace({ initial, recovered, service, capabilities }: { initial: Draft; recovered: boolean; service: CamService; capabilities: Capabilities }) {
  const [state, dispatch] = useReducer(workspaceReducer, initial, initialWorkspace);
  const [step, setStep] = useState<Step>('artwork');
  const [display, setDisplay] = useState<ArtworkDisplay | null>(null);
  const [inspected, setInspected] = useState<string | null>(null);
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const [notice, setNotice] = useState(recovered ? 'Restored the recovery draft for this tab, including unfinished fields.' : '');
  const [recovery, setRecovery] = useState('Unsaved changes');
  const [drawer, setDrawer] = useState<'issues' | 'activity' | null>('issues');
  const [theme, setTheme] = useState<Theme>(() => {
    try { const saved = localStorage.getItem('flat-v-carve:theme'); return saved === 'light' || saved === 'dark' ? saved : 'system'; } catch { return 'system'; }
  });
  const [inspectorWidth, setInspectorWidth] = useState(340);
  const [planMode, setPlanMode] = useState<'combined' | 'endmill'>('combined');
  const [openError, setOpenError] = useState('');
  const fileRequest = useRef(0);
  const latestRevision = useRef(state.revision);
  latestRevision.current = state.revision;
  const openDialog = useRef<HTMLDialogElement>(null);
  const shortcutsDialog = useRef<HTMLDialogElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const draftResult = useMemo(() => materialize(state.draft), [state.draft]);
  const job = draftResult.job ?? draftResult.previewJob;
  const [displayError, setDisplayError] = useState('');
  const jobForDisplay = useRef(job);
  jobForDisplay.current = job;
  const sourceIdentity = JSON.stringify([job.source.svg, job.import.geometry_tolerance_mm, job.import.ticks_per_mm]);
  useEffect(() => {
    const controller = new AbortController();
    setDisplay(null);
    setDisplayError('');
    service.displayFor(jobForDisplay.current, controller.signal).then(result => {
      if (!controller.signal.aborted) { setDisplay(result); setInspected(null); setHidden(new Set()); }
    }).catch(error => { if (!controller.signal.aborted) setDisplayError(String(error)); });
    return () => controller.abort();
  }, [service, sourceIdentity]);
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    try { localStorage.setItem('flat-v-carve:theme', theme); } catch { /* Theme still works for this visit. */ }
  }, [theme]);
  useEffect(() => {
    setRecovery('Unsaved changes');
    function saveRecovery() {
      try {
        sessionStorage.setItem(recoveryKey, JSON.stringify({ version: 1, draft: state.draft, savedAt: new Date().toISOString() }));
        setRecovery('Recovery saved in this tab');
      } catch { setRecovery('Recovery unavailable — download your job'); }
    }
    const timer = setTimeout(saveRecovery, 300);
    window.addEventListener('pagehide', saveRecovery);
    return () => { clearTimeout(timer); window.removeEventListener('pagehide', saveRecovery); };
  }, [state.draft]);
  function download() {
    if (!draftResult.job) { setNotice('Complete or clear the highlighted fields before downloading a portable job. Unfinished text remains in this tab’s recovery.'); setDrawer('issues'); return; }
    const blob = new Blob([JSON.stringify(draftResult.job, null, 2) + '\n'], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    anchor.href = url; anchor.download = `${draftResult.job.name.replace(/[^a-z0-9_-]/gi, '-').slice(0, 80) || 'untitled'}.job.json`;
    anchor.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
    dispatch({ type: 'downloaded' });
    setNotice('Job snapshot downloaded. Machining settings still require Rust validation.');
  }
  const downloadRef = useRef(download); downloadRef.current = download;
  useEffect(() => {
    function shortcut(event: KeyboardEvent) {
      if (document.querySelector('dialog[open]')) return;
      const editing = (event.target as HTMLElement).closest('input, textarea, select, [contenteditable="true"]');
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') { event.preventDefault(); downloadRef.current(); }
      if (!editing && (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'z') { event.preventDefault(); dispatch({ type: event.shiftKey ? 'redo' : 'undo' }); }
      if (!editing && (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'y') { event.preventDefault(); dispatch({ type: 'redo' }); }
      if (!editing && event.key === 'Escape') setInspected(null);
    }
    window.addEventListener('keydown', shortcut);
    return () => window.removeEventListener('keydown', shortcut);
  }, []);
  async function openFile(file: File | undefined) {
    if (!file) return;
    const request = ++fileRequest.current;
    const revision = state.revision;
    setOpenError('');
    try {
      if (file.size > 8_000_000) throw new Error('This local reader accepts job files up to 8 MB. Use the Rust service for larger artifacts.');
      const next = parseJob(JSON.parse(await file.text()));
      if (request !== fileRequest.current) return;
      if (latestRevision.current !== revision) throw new Error('The draft changed while the file was opening. Choose the file again to replace it; your edits have been kept.');
      dispatch({ type: 'replace', draft: newDraft(next) });
      setNotice('Job opened as an editable draft. Geometry and machining settings await Rust validation. Undo restores the previous draft.');
      setStep('artwork'); openDialog.current?.close();
    } catch (error) { if (request === fileRequest.current) setOpenError(error instanceof Error ? error.message : String(error)); }
    if (fileInput.current) fileInput.current.value = '';
  }
  const selectedComponent = display?.components.find(component => component.id === inspected);
  const fields = (items: Parameters<typeof Fields>[0]['fields']) => <Fields fields={items} draft={state.draft} errors={draftResult.errors} dispatch={dispatch} />;
  const setupProps = { draft: state.draft, fields, dispatch };
  const errors = Object.entries(draftResult.errors);
  const warnings = setupWarnings(state.draft);
  const blocking = outputBlockedReasons(capabilities);
  const stepInfo = steps.find(item => item.id === step)!;
  function focusField(path: string) {
    setStep(fieldStep(path));
    requestAnimationFrame(() => {
      const input = document.getElementById(path);
      let parent = input?.parentElement;
      while (parent) { if (parent instanceof HTMLDetailsElement) parent.open = true; parent = parent.parentElement; }
      input?.focus();
    });
  }
  return <div className="app-shell" style={{ '--inspector-width': `${inspectorWidth}px` } as React.CSSProperties}>
    <a className="skip-link" href="#inspector">Skip to settings</a>
    <header className="app-bar">
      <a className="brand" href="#" onClick={event => { event.preventDefault(); setStep('artwork'); }}><span className="brand-mark">V</span><span>FLAT V-CARVE<small>CAM WORKSPACE</small></span></a>
      <div className="document-title"><label className="sr-only" htmlFor="job-name">Job name</label><input id="job-name" value={state.draft.base.name} onChange={event => dispatch({ type: 'name', value: event.target.value })} onBlur={() => dispatch({ type: 'commit' })} /><span>{state.downloadedRevision === state.revision ? 'Job snapshot downloaded' : recovery} · revision {state.revision}</span></div>
      <div className="app-actions"><button onClick={() => { setOpenError(''); openDialog.current?.showModal(); }}>Open</button><button onClick={download}>Download job</button><span className="action-separator" /><button aria-label="Undo edit" title="Undo (Ctrl/Cmd Z outside a field)" disabled={!state.past.length && !state.editStart} onClick={() => dispatch({ type: 'undo' })}>↶</button><button aria-label="Redo edit" title="Redo" disabled={!state.future.length} onClick={() => dispatch({ type: 'redo' })}>↷</button><button className="primary" onClick={() => { setStep('plan'); setDrawer('issues'); }}>Review setup <span aria-hidden="true">→</span></button></div>
    </header>
    <div className="environment-strip"><span><span className="status-dot" /> Fixture mode <span className="divider">/</span> Captured Rust {capabilities.engineVersion} artwork</span><span>Local service not connected</span></div>
    {notice && <div className="notice" role="status"><span>{notice}</span><button onClick={() => setNotice('')} aria-label="Dismiss message">×</button></div>}
    <div className="workspace">
      <aside className="navigator" aria-label="Job workflow">
        <div className="section-caption">JOB SETUP</div>
        <nav>{steps.map(item => <button key={item.id} className={`step ${step === item.id ? 'active' : ''}`} aria-current={step === item.id ? 'step' : undefined} onClick={() => setStep(item.id)}><span className="step-number">{item.number}</span><span><strong>{item.label}</strong><small>{item.description}</small><span className="step-state">{item.id === 'artwork' ? display ? `${job.selected_region_ids.length} regions included` : 'Needs inspection' : item.id === 'stock' || item.id === 'tools' ? 'Draft · needs validation' : 'Needs local service'}</span></span></button>)}</nav>
        <div className="navigator-footer"><label htmlFor="theme">Appearance</label><select id="theme" value={theme} onChange={event => setTheme(event.target.value as Theme)}><option value="system">System</option><option value="light">Light</option><option value="dark">Dark</option></select><button className="text-button" onClick={() => shortcutsDialog.current?.showModal()}>Keyboard shortcuts <span>?</span></button></div>
      </aside>
      <main className="work-area">
        <Viewport display={display} job={job} inspected={inspected} onInspect={id => { setInspected(id); setStep('artwork'); }} hidden={hidden} />
        <div className="drawer"><div className="drawer-tabs"><button aria-expanded={drawer === 'issues'} onClick={() => setDrawer(drawer === 'issues' ? null : 'issues')}>Issues <span className="count">{errors.length + warnings.length + 1 + (displayError ? 1 : 0)}</span></button><button aria-expanded={drawer === 'activity'} onClick={() => setDrawer(drawer === 'activity' ? null : 'activity')}>Activity</button><span className="drawer-summary">{errors.length ? 'Draft needs attention' : 'No verification result'}</span></div>
          {drawer === 'issues' && <div className="drawer-content"><div className="issue"><span className="issue-tag">INFO</span><div><strong>Machining settings are not validated</strong><p>The fixture adapter provides captured source geometry. Planning, geometric verification, and machine output require the local Rust service.</p></div></div>{displayError && <p role="alert">{displayError}</p>}{warnings.map(warning => <button key={warning.path} className="issue issue-button" onClick={() => focusField(warning.path)}><span className="issue-tag">REVIEW</span><span>{warning.message}</span></button>)}{errors.map(([path, error]) => <button key={path} className="issue issue-button" onClick={() => focusField(path)}><span className="issue-tag error">INPUT</span><span><strong>{allFields(state.draft.base).find(field => field.path === path)?.label ?? path}</strong><span>{error}</span></span></button>)}</div>}
          {drawer === 'activity' && <div className="drawer-content"><p>No calculation is running.</p><p className="muted">Draft revision {state.revision}. {recovery}. Recovery belongs to this browser tab; download a job to keep it after closing the tab.</p></div>}
        </div>
      </main>
      <div className="resize-handle" role="separator" aria-label="Resize settings panel" aria-orientation="vertical" tabIndex={0} aria-valuemin={290} aria-valuemax={520} aria-valuenow={inspectorWidth}
        onKeyDown={event => { if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') { event.preventDefault(); setInspectorWidth(width => Math.min(520, Math.max(290, width + (event.key === 'ArrowLeft' ? 20 : -20)))); } }}
        onPointerDown={event => { event.currentTarget.setPointerCapture(event.pointerId); }} onPointerMove={event => { if (event.currentTarget.hasPointerCapture(event.pointerId)) setInspectorWidth(Math.min(520, Math.max(290, window.innerWidth - event.clientX))); }} onPointerUp={event => event.currentTarget.releasePointerCapture(event.pointerId)} />
      <aside id="inspector" className="inspector" tabIndex={-1} aria-label={`${stepInfo.label} settings`}>
        <div className="inspector-heading"><span className="section-caption">{stepInfo.number} / JOB SETTINGS</span><h1>{stepInfo.label}</h1><p>{stepInfo.description}</p></div>
        {step === 'artwork' && <>
          <Group title="Source artwork"><div className="source-file"><span className="file-type">SVG</span><div><strong>{job.source.filename}</strong><small>{display ? `${display.widthMm} × ${display.heightMm} mm source page` : 'Geometry unavailable'}</small></div></div><p className="hint">{display ? 'Synthetic Inkscape artwork. Review the current machining values under Carve & tools.' : 'Open jobs retain their embedded SVG. Raw SVG is never inserted into this page.'}</p><button className="wide" disabled title="Requires the local Rust import service">Replace SVG · needs service</button></Group>
          <Group title={`Regions · ${job.selected_region_ids.length} included`}><p className="hint">Inspect a region by name. Check its box to include it for machining; holes remain preserved.</p><div className="inline-actions"><button disabled={!display} onClick={() => dispatch({ type: 'include', ids: display!.components.map(component => component.id) })}>Include all</button><button onClick={() => dispatch({ type: 'include', ids: [] })}>Clear inclusion</button></div><ul className="source-list">{display?.components.map(component => <li key={component.id} className={inspected === component.id ? 'selected' : ''}><input type="checkbox" aria-label={`Include ${component.label} for machining`} checked={job.selected_region_ids.includes(component.id)} onChange={event => dispatch({ type: 'include', ids: event.target.checked ? [...job.selected_region_ids, component.id] : job.selected_region_ids.filter(id => id !== component.id) })} /><button className="source-name" aria-pressed={inspected === component.id} onClick={() => setInspected(component.id)}>{component.label}<small>{component.rings.filter(ring => ring.hole).length ? `${component.rings.filter(ring => ring.hole).length} preserved hole(s)` : 'Filled region'}</small></button><button className="visibility" aria-label={`${hidden.has(component.id) ? 'Show' : 'Hide'} ${component.label}`} aria-pressed={!hidden.has(component.id)} onClick={() => setHidden(previous => { const next = new Set(previous); if (next.has(component.id)) next.delete(component.id); else next.add(component.id); return next; })}>{hidden.has(component.id) ? 'Show' : 'Hide'}</button></li>)}</ul></Group>
          {selectedComponent && <Group title="Inspected region"><dl><dt>Name</dt><dd>{selectedComponent.label}</dd><dt>Component ID</dt><dd><code>{selectedComponent.id}</code></dd><dt>Rings</dt><dd>{selectedComponent.rings.length}</dd></dl><button onClick={() => setInspected(null)}>Clear inspection</button></Group>}
        </>}
        {step === 'stock' && <StockSetup {...setupProps} />}
        {step === 'tools' && <ToolsSetup {...setupProps} />}
        {step === 'plan' && <><Group title="Planning stage"><label className="field-label" htmlFor="plan-mode">Tool sequence</label><select id="plan-mode" value={planMode} onChange={event => setPlanMode(event.target.value as typeof planMode)}><option value="combined">Combined · endmill then V-bit</option><option value="endmill">Endmill only</option></select><p className="hint">{planMode === 'combined' ? 'Combined planning needs explicit settings for both tools.' : 'Endmill-only planning still needs V-bit geometry to define the target.'}</p></Group><Group title="Before generating"><ol className="checklist"><li>Set stock, target, tool geometry, and cutting values.</li><li>Configure travel, entry, tolerances, and resource limits.</li><li>Validate the draft with the local Rust service.</li></ol><button className="primary wide" disabled>Generate plan · needs service</button><p className="hint">The artwork view contains no toolpaths or stock simulation. No calculation runs in the browser.</p></Group><Group title="Result identity"><dl><dt>Draft revision</dt><dd>{state.revision}</dd><dt>Plan</dt><dd>Not generated</dd><dt>Engine identity</dt><dd>Not accepted</dd></dl><p className="hint">Every serialized edit requires a new identity check, including names and machine profiles.</p></Group></>}
        {step === 'verify' && <><Group title="No verification result"><p>Required geometric verification is unavailable in fixture mode.</p><p className="hint">Captured source geometry establishes no cutting or finish result. M4 planning checks concern continuous clearance, slices, and samples; sampled maxima are not global error bounds.</p></Group><Group title="Evidence to review"><dl><dt>Continuous clearance</dt><dd>Not run</dd><dt>Stock slices / samples</dt><dd>Not run</dd><dt>Geometric bounds</dt><dd>Not available</dd><dt>Formatted motions</dt><dd>Not available</dd></dl></Group><Group title="Inspection remains available"><p className="hint">You can inspect artwork, edit incomplete settings, and download a job while verification is unavailable.</p><button onClick={() => setStep('artwork')}>Inspect artwork</button></Group></>}
        {step === 'export' && <><Group title="Machine program unavailable"><ul className="blocked-reasons">{blocking.map(reason => <li key={reason}>{reason}</li>)}</ul><button className="primary wide" disabled>Export LinuxCNC program</button></Group><Group title="Portable job"><p className="hint">Download source and settings independently of machine output. The snapshot contains no display caches or verification claims.</p><button className="wide" onClick={download}>Download job snapshot</button></Group><MachineSetup {...setupProps} /></>}
        <div className="inspector-next"><button onClick={() => setStep(steps[Math.min(steps.findIndex(item => item.id === step) + 1, steps.length - 1)].id)} disabled={step === 'export'}>Next: {steps[Math.min(steps.findIndex(item => item.id === step) + 1, steps.length - 1)].label} <span>→</span></button></div>
      </aside>
    </div>
    <dialog ref={openDialog} aria-labelledby="open-dialog-title" onKeyDown={containDialogFocus}><div className="dialog-heading"><h2 id="open-dialog-title">Open a job</h2><button onClick={() => openDialog.current?.close()} aria-label="Close open dialog">×</button></div><p>Open a schema 3 job snapshot. The local Rust service is needed to inspect new artwork, migrate older jobs, and reopen plans.</p><div role="alert">{openError && <p className="inline-warning">{openError}</p>}</div><input ref={fileInput} type="file" tabIndex={-1} aria-label="Job file" accept=".json,application/json" className="sr-only" onChange={event => void openFile(event.target.files?.[0])} /><button className="primary wide" onClick={() => fileInput.current?.click()}>Choose job file…</button><div className="dialog-divider">BUNDLED EXAMPLE</div><button className="example-button" onClick={() => { void service.openExample().then(example => { dispatch({ type: 'replace', draft: newDraft(example.job) }); setStep('artwork'); setInspected(null); setHidden(new Set()); setNotice('Opened synthetic Inkscape artwork with all machining values unset. Undo restores the previous draft.'); openDialog.current?.close(); }).catch(error => setNotice(String(error))); }}><strong>Inkscape geometry coupon <span>→</span></strong><span>7 regions · curved paths · preserved holes</span><small>Synthetic artwork. No machining preset.</small></button><p className="hint">Opening a file or example is undoable. Download your current job to keep a separate copy.</p></dialog>
    <dialog ref={shortcutsDialog} aria-labelledby="shortcuts-dialog-title" onKeyDown={containDialogFocus}><div className="dialog-heading"><h2 id="shortcuts-dialog-title">Keyboard shortcuts</h2><button onClick={() => shortcutsDialog.current?.close()} aria-label="Close shortcuts">×</button></div><dl><dt>Download job</dt><dd>Ctrl / ⌘ S</dd><dt>Undo outside fields</dt><dd>Ctrl / ⌘ Z</dd><dt>Redo outside fields</dt><dd>Ctrl / ⌘ Shift Z</dd><dt>Pan focused drawing</dt><dd>Arrow keys</dd><dt>Zoom focused drawing</dt><dd>+ / −</dd><dt>Fit job</dt><dd>Home</dd><dt>Clear inspection</dt><dd>Escape</dd><dt>Resize focused separator</dt><dd>Left / Right</dd></dl><p className="hint">Text fields keep their native editing shortcuts. Regions are also available through the source list.</p></dialog>
  </div>;
}
