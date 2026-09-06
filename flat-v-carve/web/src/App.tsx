import { useExport } from './service/useExport';
import { ExportPanel } from './components/ExportPanel';
import { useEffect, useMemo, useReducer, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { parseJob, type Job } from './contracts/job';
import { editableDownloadAllowed, type ArtworkDisplay, type CamService, type Capabilities } from './contracts/service';
import { fixtureService } from './service/fixture';
import { useValidation } from './service/useValidation';
import { usePlanning } from './service/usePlanning';
import { planningInputMatches } from './contracts/planning';
import { PlanPanel } from './components/PlanPanel';
import { useInspection } from './service/useInspection';
import { InspectionToolbar } from './components/StockInspector';
import { useVerification } from './service/useVerification';
import { VerificationPanel } from './components/VerificationPanel';
import { allFields, fieldStep, materialize, newDraft, setupWarnings, type Draft } from './state/draft';
import { initialWorkspace, workspaceReducer } from './state/workspace';
import { Fields } from './components/Fields';
import { Viewport } from './components/Viewport';
import { MachineSetup, StockSetup, ToolsSetup } from './components/SetupEditors';
import { containDialogFocus } from './components/dialogFocus';
import { ToolLibraryDialog, type LibraryOpen } from './components/ToolLibraryDialog';
import { missingPlanningSettings, planningIssueField } from './state/setupNeeds';
import { useLibraryAssignments } from './service/useLibraryAssignments';

const steps = [
  { id: 'artwork', label: 'Artwork', description: 'Source & regions', number: '01' },
  { id: 'stock', label: 'Stock & origin', description: 'Material & placement', number: '02' },
  { id: 'tools', label: 'Carve & tools', description: 'Shape & cutting setup', number: '03' },
  { id: 'plan', label: 'Plan & inspect', description: 'Paths & task results', number: '04' },
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
    void (async () => {
      const capabilities = await service.capabilities(controller.signal);
      if (controller.signal.aborted) return;
      let recovered: Draft | null = null;
      if (!skipRecovery) {
        try { recovered = readRecovery({ getItem: key => sessionStorage.getItem(key) }); }
        catch { throw new Error('This tab’s recovery draft could not be read. It has been kept unchanged. Retry, or explicitly replace it with the bundled example.'); }
      }
      const draft = recovered ?? newDraft((await service.openExample(controller.signal)).job);
      if (!controller.signal.aborted) setBoot({ draft, capabilities, recovered: !!recovered });
    })().catch(error => { if (!controller.signal.aborted) setBootError(String(error)); });
    return () => controller.abort();
  }, [service, skipRecovery]);
  if (bootError) return <main className="startup"><h1>Cannot open the workspace</h1><p role="alert">{bootError}</p><div className="inline-actions"><button onClick={() => location.reload()}>Retry</button><button onClick={() => setSkipRecovery(true)}>Open example (replace recovery)</button></div></main>;
  if (!boot) return <main className="startup" role="status">Opening the CAM workspace…</main>;
  return <Workspace key={boot.capabilities.engineVersion} initial={boot.draft} recovered={boot.recovered} service={service} capabilities={boot.capabilities} />;
}

function Workspace({ initial, recovered, service, capabilities: initialCapabilities }: { initial: Draft; recovered: boolean; service: CamService; capabilities: Capabilities }) {
  const [capabilities, setCapabilities] = useState(initialCapabilities);
  const [refresh, setRefresh] = useState(0);
  const [connecting, setConnecting] = useState(false);
  const [state, dispatch] = useReducer(workspaceReducer, initial, initialWorkspace);
  const [step, setStep] = useState<Step>('artwork');
  const [displayResult, setDisplayResult] = useState<{ identity: string; value: ArtworkDisplay | null } | null>(null);
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
  const [libraryOpen, setLibraryOpen] = useState<LibraryOpen | null>(null);
  const libraryAssignments = useLibraryAssignments(recovered);
  const fileRequest = useRef(0);
  const fileController = useRef<AbortController | null>(null);
  const [opening, setOpening] = useState(false);
  const latestRevision = useRef(state.revision);
  latestRevision.current = state.revision;
  const openDialog = useRef<HTMLDialogElement>(null);
  const shortcutsDialog = useRef<HTMLDialogElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const svgInput = useRef<HTMLInputElement>(null);
  const [importTolerance, setImportTolerance] = useState('0.001');
  const draftResult = useMemo(() => materialize(state.draft), [state.draft]);
  const job = draftResult.job ?? draftResult.previewJob;
  const validation = useValidation(service, draftResult.job, state.revision, refresh, capabilities.validateDraft);
  const planning = usePlanning(service, capabilities, validation.result, state.revision, planMode, refresh);
  const inspection = useInspection(service, planning.result, planning.current, job, refresh);
  const verification = useVerification(service, capabilities, planning.result?.task ?? null, planning.current, refresh);
  const output = useExport(service, capabilities, planning.result?.task ?? null, planning.current, verification.options, refresh);
  const [displayError, setDisplayError] = useState('');
  const jobForDisplay = useRef(job);
  jobForDisplay.current = job;
  const sourceIdentity = JSON.stringify([job.source.svg, job.import, capabilities.engineVersion, refresh]);
  const display = displayResult?.identity === sourceIdentity ? displayResult.value : null;
  useEffect(() => {
    const controller = new AbortController();
    setDisplayResult(null);
    setDisplayError('');
    service.displayFor(jobForDisplay.current, controller.signal).then(result => {
      if (!controller.signal.aborted) { setDisplayResult({ identity: sourceIdentity, value: result }); setInspected(null); setHidden(new Set()); }
    }).catch(error => { if (!controller.signal.aborted) setDisplayError(String(error)); });
    return () => controller.abort();
  }, [service, sourceIdentity]);
  useEffect(() => () => fileController.current?.abort(), []);
  async function reconnect() {
    setConnecting(true);
    try { setCapabilities(await service.capabilities()); setRefresh(value => value + 1); setNotice('Reconnected. Checking the current draft again.'); }
    catch (error) { setNotice(String(error)); }
    finally { setConnecting(false); }
  }
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
    if (capabilities.mode === 'live' && !editableDownloadAllowed(validation.result, state.revision)) {
      setNotice(validation.pending ? 'Wait for Rust to check the current revision, then download again.' : 'Resolve the Rust validation errors or reconnect before downloading. Your draft remains in recovery.');
      setDrawer('issues'); return;
    }
    // Keep the actual download inside the user gesture. The receipt must match this
    // exact revision; edits discard it immediately, including invalid form text.
    const snapshot = draftResult.job;
    const blob = new Blob([JSON.stringify(snapshot, null, 2) + '\n'], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    anchor.href = url; anchor.download = `${snapshot.name.replace(/[^a-z0-9_-]/gi, '-').slice(0, 80) || 'untitled'}.job.json`;
    anchor.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
    dispatch({ type: 'downloaded' });
    setNotice(capabilities.mode === 'live' ? 'Rust-validated job download requested. Check your browser’s downloads to confirm it was saved.' : 'Job download requested. Machining settings still require Rust validation.');
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
  function cancelOpen() {
    ++fileRequest.current;
    fileController.current?.abort();
    setOpening(false);
  }
  async function replaceDocument(load: (signal: AbortSignal) => Promise<Job>, message: string) {
    const request = ++fileRequest.current;
    const revision = state.revision;
    fileController.current?.abort();
    const controller = new AbortController();
    fileController.current = controller;
    setOpenError('');
    setOpening(true);
    try {
      const next = await load(controller.signal);
      if (request !== fileRequest.current || controller.signal.aborted) return;
      if (latestRevision.current !== revision) throw new Error('The draft changed while the file was opening. Choose the file again to replace it; your edits have been kept.');
      dispatch({ type: 'replace', draft: newDraft(next) });
      libraryAssignments.clear();
      setNotice(`${message} Undo restores the previous draft.`);
      setStep('artwork'); openDialog.current?.close();
    } catch (error) { if (request === fileRequest.current) setOpenError(error instanceof Error ? error.message : String(error)); }
    finally { if (request === fileRequest.current) setOpening(false); }
  }
  async function openFile(file: File | undefined, svg = false) {
    if (!file) return;
    await replaceDocument(async signal => {
      const limit = svg ? capabilities.limits?.svgBytes : capabilities.limits?.jobBytes ?? 8_000_000;
      if (!limit || file.size > limit) throw new Error(`This file exceeds the local ${svg ? 'SVG' : 'job'} input limit (${limit ?? 0} bytes).`);
      const text = await file.text();
      signal.throwIfAborted();
      if (svg) {
        if (!capabilities.importArtwork || !service.importArtwork) throw new Error('SVG import needs the local Rust service.');
        if (!importTolerance.trim() || !Number.isFinite(Number(importTolerance))) throw new Error('Enter an import tolerance in millimeters.');
        return (await service.importArtwork(file.name, text, { geometry_tolerance_mm: Number(importTolerance), ticks_per_mm: null,
          placement: { origin_mm: { x: 0, y: 0 }, scale: 1, rotation_deg: 0 } }, state.revision, signal)).job;
      }
      return capabilities.openJob && service.openJob ? (await service.openJob(text, state.revision, signal)).job : parseJob(JSON.parse(text));
    }, svg ? 'SVG imported by Rust. Cutting settings need setup; wall allowance defaults to 0 mm.'
      : capabilities.openJob ? 'Job opened and checked by Rust as an editable document.' : 'Job opened as an editable draft. Geometry and settings await Rust validation.');
    if (fileInput.current) fileInput.current.value = '';
    if (svgInput.current) svgInput.current.value = '';
  }
  const selectedComponent = display?.components.find(component => component.id === inspected);
  const fields = (items: Parameters<typeof Fields>[0]['fields']) => <Fields fields={items} draft={state.draft} errors={draftResult.errors} dispatch={dispatch} />;
  const setupProps = { draft: state.draft, fields, dispatch };
  const errors = Object.entries(draftResult.errors);
  const warnings = setupWarnings(state.draft);
  const setupNeeds = missingPlanningSettings(job,validation.result,planMode);
  const currentTaskInput = planningInputMatches(planning.task,validation.result,state.revision,planMode,capabilities);
  const planningLabel = planning.active ? 'Background task active' : validation.pending ? 'Checking setup…' : !draftResult.job || !validation.result?.valid ? 'Setup needs attention' : currentTaskInput && planning.task?.state === 'failed' ? 'Planning failed'
    : setupNeeds.length ? `${setupNeeds.length} setup ${setupNeeds.length === 1 ? 'item' : 'items'} missing` : planning.current ? `Plan ${planning.result?.task.summary?.status}` : planning.result ? 'Previous plan stale' : 'Planning available';
  const planIssues = currentTaskInput && planning.task?.diagnostic ? [planning.task.diagnostic] : planning.current ? [
    ...(planning.result?.task.summary?.diagnostics ?? []),
    ...(planning.result?.task.summary?.generationIssues ?? []),
  ] : [];
  const outputIssues = output.task?.diagnostic ? [output.task.diagnostic] : output.current ? [
    ...(output.result?.report.diagnostics ?? []), ...(output.result?.report.plan_verification.original.findings ?? []),
    ...(output.result?.report.emitted_verification?.findings ?? []),
  ] : [];
  const verificationIssues = verification.current ? verification.evidence?.findings ?? [] : [];
  const stepInfo = steps.find(item => item.id === step)!;
  function focusField(path: string) {
    setStep(path === 'selected_region_ids' ? 'artwork' : fieldStep(path));
    requestAnimationFrame(() => {
      const input = document.getElementById(path);
      let parent = input;
      while (parent) { if (parent instanceof HTMLDetailsElement) parent.open = true; parent = parent.parentElement; }
      input?.scrollIntoView({block:'center'});
      input?.focus();
    });
  }
  return <div className="app-shell" style={{ '--inspector-width': `${inspectorWidth}px` } as React.CSSProperties}>
    <a className="skip-link" href="#inspector">Skip to settings</a>
    <header className="app-bar">
      <a className="brand" href="#" onClick={event => { event.preventDefault(); setStep('artwork'); }}><span className="brand-mark">V</span><span>FLAT V-CARVE<small>CAM WORKSPACE</small></span></a>
      <div className="document-title"><label className="sr-only" htmlFor="job-name">Job name</label><input id="job-name" value={state.draft.base.name} onChange={event => dispatch({ type: 'name', value: event.target.value })} onBlur={() => dispatch({ type: 'commit' })} /><span>{state.downloadedRevision === state.revision ? 'Job download requested' : recovery} · revision {state.revision}</span></div>
      <div className="app-actions"><button onClick={() => { setOpenError(''); openDialog.current?.showModal(); }}>Open</button><button onClick={download}>Download job</button><span className="action-separator" /><button aria-label="Undo edit" title="Undo (Ctrl/Cmd Z outside a field)" disabled={!state.past.length && !state.editStart} onClick={() => dispatch({ type: 'undo' })}>↶</button><button aria-label="Redo edit" title="Redo" disabled={!state.future.length} onClick={() => dispatch({ type: 'redo' })}>↷</button><button className="primary" onClick={() => { setStep('plan'); setDrawer('issues'); }}>Review setup <span aria-hidden="true">→</span></button></div>
    </header>
    <div className="environment-strip"><span><span className="status-dot" /> {capabilities.mode === 'live' ? 'Local Rust service' : 'Fixture mode'} <span className="divider">/</span> Rust {capabilities.engineVersion}</span><span>{capabilities.mode === 'live' ? <button disabled={connecting} onClick={() => void reconnect()}>{connecting ? 'Connecting…' : 'Reconnect service'}</button> : 'Local service not connected'}</span></div>
    {notice && <div className="notice" role="status"><span>{notice}</span><button onClick={() => setNotice('')} aria-label="Dismiss message">×</button></div>}
    <div className="workspace">
      <aside className="navigator" aria-label="Job workflow">
        <div className="section-caption">JOB SETUP</div>
        <nav>{steps.map(item => <button key={item.id} className={`step ${step === item.id ? 'active' : ''}`} aria-current={step === item.id ? 'step' : undefined} onClick={() => setStep(item.id)}><span className="step-number">{item.number}</span><span><strong>{item.label}</strong><small>{item.description}</small><span className="step-state">{item.id === 'artwork' ? display ? `${job.selected_region_ids.length} regions included` : 'Needs inspection' : item.id === 'stock' || item.id === 'tools' ? validation.result?.valid ? validation.result.missingMachiningFields?.length ? 'Checked · settings unset' : 'Editable job checked' : 'Draft · needs validation' : item.id === 'plan' && capabilities.planningStages.length ? planningLabel : item.id === 'verify' ? verification.label : item.id === 'export' ? output.label : 'Unavailable'}</span></span></button>)}</nav>
        <div className="navigator-footer"><label htmlFor="theme">Appearance</label><select id="theme" value={theme} onChange={event => setTheme(event.target.value as Theme)}><option value="system">System</option><option value="light">Light</option><option value="dark">Dark</option></select><button className="text-button" onClick={() => shortcutsDialog.current?.showModal()}>Keyboard shortcuts <span>?</span></button></div>
      </aside>
      <main className="work-area">
        {planning.result && <InspectionToolbar inspection={inspection} />}
        <Viewport display={display} job={job} inspected={inspected} onInspect={id => { setInspected(id); setStep('artwork'); }} hidden={hidden} motions={inspection.motions} stockInfo={inspection.current ? inspection.selected : undefined} stockGeometry={inspection.geometry} stockPending={inspection.pending} stockError={inspection.error} focus={step === 'verify' && verification.focus ? verification.focus : inspection.focus} verificationFinding={step === 'verify' ? verification.finding : undefined} verificationScope={verification.scope === 'rounded' ? `rounded to ${verification.result?.report.rounded?.decimal_places} decimal places` : 'original coordinates'} />
        <div className="drawer"><div className="drawer-tabs"><button aria-expanded={drawer === 'issues'} onClick={() => setDrawer(drawer === 'issues' ? null : 'issues')}>Issues <span className="count">{errors.length + warnings.length + setupNeeds.length + planIssues.length + verificationIssues.length + outputIssues.length + 1 + (validation.result?.diagnostics.length ?? 0) + (displayError ? 1 : 0)}</span></button><button aria-expanded={drawer === 'activity'} onClick={() => setDrawer(drawer === 'activity' ? null : 'activity')}>Activity</button><span className="drawer-summary">{errors.length ? 'Draft needs attention' : step === 'export' || output.active ? output.label : verification.label}</span></div>
          {drawer === 'issues' && <div className="drawer-content">{setupNeeds.map(need => <button key={'setup-' + need.path} className="issue issue-button" onClick={() => focusField(need.path)}><span className="issue-tag error">SETUP</span><span><strong>{need.label}</strong><span>{need.message}</span></span></button>)}{outputIssues.slice(0,20).map((issue,index) => <button className="issue issue-button" key={'export-' + index} onClick={() => setStep('export')}><span className="issue-tag">EXPORT</span><span><strong>{issue.code}</strong><span>{issue.message}</span></span></button>)}{outputIssues.length > 20 && <button onClick={() => setStep('export')}>Review all export findings</button>}{verificationIssues.slice(0,20).map((finding, index) => <button className="issue issue-button" key={"verification-" + index} onClick={() => { setStep('verify'); verification.locate(index); }}><span className="issue-tag">VERIFY</span><span><strong>{finding.code}</strong><span>{finding.status} · {finding.message}</span></span></button>)}{verificationIssues.length > 20 && <button onClick={() => setStep('verify')}>Review all {verificationIssues.length} verification findings</button>}{planIssues.map((issue, i) => <button className="issue issue-button" key={`plan-${i}`} onClick={() => { const fix = planningIssueField(job,issue); if (fix) focusField(fix.path); else setStep('plan'); }}><span className="issue-tag">PLAN</span><span><strong>{issue.code}</strong><span>{issue.message}</span></span></button>)}<div className="issue"><span className="issue-tag">INFO</span><div><strong role="status">{validation.headline}</strong><p>{capabilities.mode === 'live' ? 'This checks supplied job settings and SVG normalization. It does not establish stage readiness, cutting verification, or machine-output eligibility.' : 'The fixture adapter provides captured source geometry. Planning, geometric verification, and machine output require the local Rust service.'}</p>{validation.error && <p role="alert">{validation.error}</p>}{validation.result?.diagnostics.map((diagnostic, index) => <p key={index} role={diagnostic.severity === 'error' ? 'alert' : undefined}><strong>{diagnostic.code}</strong>: {diagnostic.message}{diagnostic.sourceId && <span> · source {diagnostic.sourceId}</span>}</p>)}</div></div>{displayError && <p role="alert">{displayError}</p>}{warnings.map(warning => <button key={warning.path} className="issue issue-button" onClick={() => focusField(warning.path)}><span className="issue-tag">REVIEW</span><span>{warning.message}</span></button>)}{errors.map(([path, error]) => <button key={path} className="issue issue-button" onClick={() => focusField(path)}><span className="issue-tag error">INPUT</span><span><strong>{allFields(state.draft.base).find(field => field.path === path)?.label ?? path}</strong><span>{error}</span></span></button>)}</div>}
          {drawer === 'activity' && <div className="drawer-content"><p>{validation.pending ? 'Checking editable job with Rust…' : output.active ? 'Machine output is being checked. Open Export for status or cancellation.' : verification.active ? 'Continuous verification is running. Open Verification for status or cancellation.' : planning.active ? 'A background planning task is active. Inspect Plan & inspect for progress or cancellation.' : 'No calculation is active.'}</p><p className="muted">Draft revision {state.revision}. {recovery}. Recovery belongs to this browser tab; download a job to keep it after closing the tab.</p></div>}
        </div>
      </main>
      <div className="resize-handle" role="separator" aria-label="Resize settings panel" aria-orientation="vertical" tabIndex={0} aria-valuemin={290} aria-valuemax={520} aria-valuenow={inspectorWidth}
        onKeyDown={event => { if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') { event.preventDefault(); setInspectorWidth(width => Math.min(520, Math.max(290, width + (event.key === 'ArrowLeft' ? 20 : -20)))); } }}
        onPointerDown={event => { event.currentTarget.setPointerCapture(event.pointerId); }} onPointerMove={event => { if (event.currentTarget.hasPointerCapture(event.pointerId)) setInspectorWidth(Math.min(520, Math.max(290, window.innerWidth - event.clientX))); }} onPointerUp={event => event.currentTarget.releasePointerCapture(event.pointerId)} />
      <aside id="inspector" className="inspector" tabIndex={-1} aria-label={`${stepInfo.label} settings`}>
        <div className="inspector-heading"><span className="section-caption">{stepInfo.number} / JOB SETTINGS</span><h1>{stepInfo.label}</h1><p>{stepInfo.description}</p></div>
        {step === 'artwork' && <>
          <Group title="Source artwork"><div className="source-file"><span className="file-type">SVG</span><div><strong>{job.source.filename}</strong><small>{display ? `${display.widthMm} × ${display.heightMm} mm source page` : 'Geometry unavailable'}</small></div></div><p className="hint">{display?.description ?? 'Open jobs retain their embedded SVG. Raw SVG is never inserted into this page.'}</p><button className="wide" disabled={!capabilities.importArtwork} onClick={() => { setOpenError(''); openDialog.current?.showModal(); }}>Import SVG…</button></Group>
          <Group title={`Regions · ${job.selected_region_ids.length} included`}><p className="hint">Inspect a region by name. Check its box to include it for machining; holes remain preserved.</p><div className="inline-actions"><button id="selected_region_ids" disabled={!display} onClick={() => dispatch({ type: 'include', ids: display!.components.map(component => component.id) })}>Include all</button><button onClick={() => dispatch({ type: 'include', ids: [] })}>Clear inclusion</button></div><ul className="source-list">{display?.components.map(component => <li key={component.id} className={inspected === component.id ? 'selected' : ''}><input type="checkbox" aria-label={`Include ${component.label} for machining`} checked={job.selected_region_ids.includes(component.id)} onChange={event => dispatch({ type: 'include', ids: event.target.checked ? [...job.selected_region_ids, component.id] : job.selected_region_ids.filter(id => id !== component.id) })} /><button className="source-name" aria-pressed={inspected === component.id} onClick={() => setInspected(component.id)}>{component.label}<small>{component.rings.filter(ring => ring.hole).length ? `${component.rings.filter(ring => ring.hole).length} preserved hole(s)` : 'Filled region'}</small></button><button className="visibility" aria-label={`${hidden.has(component.id) ? 'Show' : 'Hide'} ${component.label}`} aria-pressed={!hidden.has(component.id)} onClick={() => setHidden(previous => { const next = new Set(previous); if (next.has(component.id)) next.delete(component.id); else next.add(component.id); return next; })}>{hidden.has(component.id) ? 'Show' : 'Hide'}</button></li>)}</ul></Group>
          {selectedComponent && <Group title="Inspected region"><dl><dt>Name</dt><dd>{selectedComponent.label}</dd><dt>Component ID</dt><dd><code>{selectedComponent.id}</code></dd><dt>Rings</dt><dd>{selectedComponent.rings.length}</dd></dl><button onClick={() => setInspected(null)}>Clear inspection</button></Group>}
        </>}
        {step === 'stock' && <StockSetup {...setupProps} />}
        {step === 'tools' && <ToolsSetup {...setupProps} assignments={libraryAssignments.assignments} openLibrary={capabilities.toolLibrary ? slot => setLibraryOpen(previous => ({ serial: (previous?.serial ?? 0) + 1, slot })) : undefined} />}
        {step === 'plan' && <PlanPanel planning={planning} capabilities={capabilities} job={draftResult.job} validation={validation.result} revision={state.revision} stage={planMode} onStage={setPlanMode} inspection={inspection} onFix={focusField} />}
        {step === 'verify' && <VerificationPanel verification={verification} planCurrent={planning.current} combined={planMode === 'combined'} />}
        {step === 'export' && <><ExportPanel output={output} job={job} planCurrent={planning.current && planMode === 'combined'} /><Group title="Portable job"><p className="hint">Download source and settings independently of machine output. The snapshot contains no display caches or verification claims.</p><button className="wide" onClick={download}>Download job snapshot</button></Group><details className="inspector-group"><summary>Job machine constraints (optional)</summary><MachineSetup {...setupProps} /></details></>}
        <div className="inspector-next"><button onClick={() => setStep(steps[Math.min(steps.findIndex(item => item.id === step) + 1, steps.length - 1)].id)} disabled={step === 'export'}>Next: {steps[Math.min(steps.findIndex(item => item.id === step) + 1, steps.length - 1)].label} <span>→</span></button></div>
      </aside>
    </div>
    <ToolLibraryDialog request={libraryOpen} service={service} capabilities={capabilities} draft={state.draft} revision={state.revision}
      job={draftResult.job && editableDownloadAllowed(validation.result, state.revision) ? { job: draftResult.job, revision: state.revision, documentFingerprint: validation.result!.documentFingerprint! } : null}
      dispatch={dispatch} applied={(slot,assignment,changed) => { libraryAssignments.applied(slot,assignment); setNotice(`${slot === 'endmill' ? 'Endmill' : 'V-bit'}: ${assignment.toolName} · ${assignment.presetName ?? 'geometry only'} applied to this job.${changed ? ' Undo restores the previous settings; generate a new plan.' : ' The values already matched your job.'}`); }} />
    <dialog ref={openDialog} aria-labelledby="open-dialog-title" onKeyDown={containDialogFocus} onClose={cancelOpen}>
      <div className="dialog-heading"><h2 id="open-dialog-title">Open artwork or job</h2><button onClick={() => openDialog.current?.close()} aria-label="Close open dialog">×</button></div>
      <p>{capabilities.openJob ? 'Rust checks job files and migrates schemas 1–2 to schema 3. Plans are not supported here yet.' : 'Open a schema 3 job snapshot. Other artwork and migrations need the local Rust service.'}</p>
      <div role="alert">{openError && <p className="inline-warning">{openError}</p>}</div>
      {opening && <p role="status">Opening with the local service… Closing this dialog keeps the current draft.</p>}
      <input ref={fileInput} type="file" tabIndex={-1} aria-label="Job file" accept=".json,application/json" className="sr-only" onChange={event => void openFile(event.target.files?.[0])} />
      <button className="primary wide" disabled={opening} onClick={() => fileInput.current?.click()}>Choose job file…</button>
      {capabilities.importArtwork && <>
        <div className="dialog-divider">SVG ARTWORK</div><p className="hint">Import starts a new editable job with cutting settings unset and 0 mm wall allowance.</p>
        <label className="field-label" htmlFor="import-tolerance">Import tolerance (mm)</label><input id="import-tolerance" inputMode="decimal" value={importTolerance} onChange={event => setImportTolerance(event.target.value)} />
        <input ref={svgInput} type="file" tabIndex={-1} aria-label="SVG file" accept=".svg,image/svg+xml" className="sr-only" onChange={event => void openFile(event.target.files?.[0], true)} />
        <button className="wide" disabled={opening} onClick={() => svgInput.current?.click()}>Choose SVG file…</button>
      </>}
      <div className="dialog-divider">BUNDLED EXAMPLE</div>
      <button className="example-button" disabled={opening} onClick={() => void replaceDocument(async signal => (await service.openExample(signal)).job, 'Opened synthetic Inkscape artwork with cutting settings unset and 0 mm wall allowance.')}><strong>Inkscape geometry coupon <span>→</span></strong><span>7 regions · curved paths · preserved holes</span><small>Synthetic artwork. No machining preset.</small></button>
      <p className="hint">Opening a file or example is undoable. Download your current job to keep a separate copy.</p>
    </dialog>
    <dialog ref={shortcutsDialog} aria-labelledby="shortcuts-dialog-title" onKeyDown={containDialogFocus}><div className="dialog-heading"><h2 id="shortcuts-dialog-title">Keyboard shortcuts</h2><button onClick={() => shortcutsDialog.current?.close()} aria-label="Close shortcuts">×</button></div><dl><dt>Download job</dt><dd>Ctrl / ⌘ S</dd><dt>Undo outside fields</dt><dd>Ctrl / ⌘ Z</dd><dt>Redo outside fields</dt><dd>Ctrl / ⌘ Shift Z</dd><dt>Pan focused drawing</dt><dd>Arrow keys</dd><dt>Zoom focused drawing</dt><dd>+ / −</dd><dt>Fit job</dt><dd>Home</dd><dt>Clear inspection</dt><dd>Escape</dd><dt>Resize focused separator</dt><dd>Left / Right</dd></dl><p className="hint">Text fields keep their native editing shortcuts. Regions are also available through the source list.</p></dialog>
  </div>;
}
