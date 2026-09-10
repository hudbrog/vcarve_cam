// Sequence workspace (ui-8): one ordered operation list over the canonical
// schema-4 sequenceDoc, with engine-side editing, planning and export. This is
// the A4 basic milestone; stock simulation of sequence plans arrives with the
// timeline slice and stays available through the legacy workflow meanwhile.
import { useEffect, useRef, useState } from 'react';
import type {
  ExportResult, OperationEdit, PlanResult, PlanScope, SequenceCapabilities, SequenceDocument, SequenceService,
} from '../contracts/sequence';
import { FaceSettingsEditor } from './FaceSettingsEditor';

const recoveryKey = 'flat-v-carve:sequence:v1';
const profileKey = 'flat-v-carve:sequence:profile';

interface Recovery {
  version: 1;
  job: unknown;
  savedAt: string;
}

export function readSequenceRecovery(storage: Pick<Storage, 'getItem'>): unknown | null {
  let raw: string | null;
  try { raw = storage.getItem(recoveryKey); } catch { return null; }
  if (!raw) return null;
  const data = JSON.parse(raw) as Recovery;
  if (data.version !== 1 || typeof data.job !== 'object' || data.job === null) throw new Error('Unsupported sequence recovery.');
  return data.job;
}

export function SequenceWorkspace({ service, onExit, initialDocument }: { service: SequenceService; onExit: () => void; initialDocument?: SequenceDocument | null }) {
  const [sequenceDoc, setDocument] = useState<SequenceDocument | null>(initialDocument ?? null);
  const [capabilities, setCapabilities] = useState<SequenceCapabilities | null>(null);
  const [plan, setPlan] = useState<PlanResult | null>(null);
  const [exportResult, setExportResult] = useState<ExportResult | null>(null);
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState('');
  const [selected, setSelected] = useState<string | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const profileInput = useRef<HTMLInputElement>(null);
  const [profile, setProfile] = useState<unknown>(null);

  useEffect(() => {
    const controller = new AbortController();
    service.capabilities(controller.signal)
      .then(setCapabilities, cause => { if (!controller.signal.aborted) setError(String(cause)); });
    return () => controller.abort();
  }, [service]);

  // Draft recovery is versioned independently of legacy tab drafts (16.3):
  // the opaque schema-4 job plus its operation list, nothing else.
  useEffect(() => {
    if (!sequenceDoc) return;
    try {
      sessionStorage.setItem(recoveryKey, JSON.stringify({ version: 1, job: sequenceDoc.job, savedAt: new Date().toISOString() } satisfies Recovery));
    } catch { /* Recovery stays unavailable; downloads keep the sequenceDoc. */ }
    setPlan(null);
    setExportResult(null);
  }, [sequenceDoc]);

  useEffect(() => {
    if (!sequenceDoc && !initialDocument) {
      try {
        const recovered = readSequenceRecovery({ getItem: key => sessionStorage.getItem(key) });
        if (recovered !== null) {
          void run('open', async signal => setDocument(await service.open(JSON.stringify(recovered), signal)),
            'Recovered the sequence sequenceDoc from this tab.');
          return;
        }
      } catch (cause) { setError(String(cause)); }
    }
    // The applied machine profile is a separate, explicit choice.
    try {
      const raw = localStorage.getItem(profileKey);
      if (raw) setProfile(JSON.parse(raw));
    } catch { /* No profile selected yet. */ }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function run(what: string, action: (signal: AbortSignal) => Promise<void>, message = '') {
    setError('');
    setBusy(what);
    const controller = new AbortController();
    try {
      await action(controller.signal);
      setNotice(message);
    } catch (cause) {
      if (!controller.signal.aborted) setError(String(cause));
    } finally {
      setBusy('');
    }
  }

  async function edit(edits: OperationEdit[], message: string) {
    if (!sequenceDoc) return;
    await run('edit', async signal => setDocument(await service.edit(sequenceDoc.job, edits, signal)), message);
  }

  function downloadProgram() {
    if (!exportResult) return;
    const blob = new Blob([exportResult.program.gcode], { type: 'text/plain' });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = exportResult.program.filename;
    anchor.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  function downloadJob() {
    if (!sequenceDoc) return;
    const blob = new Blob([JSON.stringify(sequenceDoc.job, null, 2) + '\n'], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = 'sequence.job.json';
    anchor.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  const missingCount = (id: string) => sequenceDoc?.missingByOperation[id]?.length ?? 0;

  return <div className="app-shell sequence-shell">
    <header className="app-bar">
      <a className="brand" href="#" onClick={event => { event.preventDefault(); onExit(); }}><span className="brand-mark">V</span><span>SEQUENCE<small>SCHEMA-4 WORKSPACE</small></span></a>
      <div className="sequenceDoc-title"><span>{sequenceDoc ? `${sequenceDoc.operations.length} operation(s)` : 'No sequenceDoc open'}</span>{capabilities && <span> · Rust {capabilities.engineVersion}</span>}</div>
      <div className="app-actions">
        <button onClick={() => fileInput.current?.click()} disabled={!!busy}>Open job…</button>
        <button onClick={downloadJob} disabled={!sequenceDoc || !!busy}>Download job</button>
        <button className="primary" disabled={!sequenceDoc || !profile || busy === 'plan'} onClick={() => sequenceDoc && run('plan', async signal => setPlan(await service.plan(sequenceDoc.job, { kind: 'allEnabled' } satisfies PlanScope, signal)))}>{busy === 'plan' ? 'Planning…' : 'Plan sequence'}</button>
        <button onClick={onExit}>V-carve workspace</button>
      </div>
    </header>
    {notice && <div className="notice" role="status"><span>{notice}</span><button onClick={() => setNotice('')} aria-label="Dismiss message">×</button></div>}
    {error && <div className="notice" role="alert"><span>{error}</span><button onClick={() => setError('')} aria-label="Dismiss error">×</button></div>}
    <input ref={fileInput} type="file" accept=".json,application/json" className="sr-only" aria-label="Sequence job file"
      onChange={event => {
        const file = event.target.files?.[0];
        event.target.value = '';
        if (!file) return;
        void run('open', async signal => {
          const text = await file.text();
          signal.throwIfAborted();
          setDocument(await service.open(text, signal));
        }, 'Document opened; migration preserved every supplied value.');
      }} />
    <div className="sequence-grid">
      <section aria-label="Operations" className="sequence-operations">
        <h2>Operations (execution order)</h2>
        {!sequenceDoc && <p className="hint">Open a legacy (schema 1–3) or schema-4 job to begin. Legacy documents migrate through the engine, preserving every value.</p>}
        {sequenceDoc && <ul className="sequence-list">
          {sequenceDoc.operations.map((operation, index) => <li key={operation.id} className={!operation.enabled ? 'disabled' : ''}>
            <div className="sequence-op-line">
              <button className="source-name" aria-pressed={selected === operation.id} onClick={() => setSelected(selected === operation.id ? null : operation.id)}>
                <strong>{index + 1}. {operation.name}</strong>
              </button>
              <small>{operation.kind}{operation.enabled ? '' : ' · disabled'}{missingCount(operation.id) ? ` · ${missingCount(operation.id)} missing` : ''}</small>
            </div>
            <div className="sequence-op-actions">
              <button aria-label={`Toggle ${operation.name}`} disabled={!!busy} onClick={() => void edit([{ edit: 'setEnabled', id: operation.id, enabled: !operation.enabled }], 'Operation list updated.')}>{operation.enabled ? 'Disable' : 'Enable'}</button>
              <button aria-label={`Move ${operation.name} up`} disabled={!!busy || index === 0} onClick={() => void edit([{ edit: 'move', id: operation.id, toIndex: index - 1 }], 'Operation moved.')}>↑</button>
              <button aria-label={`Move ${operation.name} down`} disabled={!!busy || index === sequenceDoc.operations.length - 1} onClick={() => void edit([{ edit: 'move', id: operation.id, toIndex: index + 1 }], 'Operation moved.')}>↓</button>
              <button aria-label={`Duplicate ${operation.name}`} disabled={!!busy} onClick={() => void edit([{ edit: 'duplicate', id: operation.id, newId: `${operation.id}-copy-${index + 2}` }], 'Operation duplicated with a new ID.')}>Duplicate</button>
              <button aria-label={`Delete ${operation.name}`} disabled={!!busy} onClick={() => void edit([{ edit: 'delete', id: operation.id }], 'Operation deleted.')}>Delete</button>
            </div>
          </li>)}
        </ul>}
        {(() => {
          // Face settings editor for the selected face operation. The job
          // document is read for display only; edits go through the engine.
          if (!sequenceDoc || !selected) return null;
          const operation = sequenceDoc.operations.find(op => op.id === selected);
          if (!operation || operation.kind !== 'face') return null;
          const raw = sequenceDoc.job as { operations?: { id: string; settings?: { settings?: unknown } }[] };
          const settings = raw.operations?.find(op => op.id === selected)?.settings?.settings as
            | { area: { kind: string; rect?: Record<string, number> }; margins: Record<string, number | null | undefined>; [key: string]: unknown }
            | undefined;
          if (!settings) return null;
          return <FaceSettingsEditor
            service={service}
            job={sequenceDoc.job}
            operationId={selected}
            settings={settings as never}
            busy={!!busy}
            onApplied={(document, message) => { setDocument(document); setNotice(message); }}
          />;
        })()}
      </section>
      <section aria-label="Plan" className="sequence-plan">
        <h2>Plan & export</h2>
        <div className="sequence-profile-row">
          <button onClick={() => profileInput.current?.click()} disabled={!!busy}>{profile ? 'Replace machine profile' : 'Apply legacy machine profile'}</button>
          {profile ? <small>profile applied (Z datum + spindle directions)</small> : null}
          <input ref={profileInput} type="file" accept=".json,application/json" className="sr-only" aria-label="Legacy machine profile"
            onChange={event => {
              const file = event.target.files?.[0];
              event.target.value = '';
              if (!file || !sequenceDoc) return;
              void run('profile', async signal => {
                const text = await file.text();
                signal.throwIfAborted();
                const parsed = JSON.parse(text);
                try { localStorage.setItem(profileKey, text); } catch { /* Choice still applies for this session. */ }
                setProfile(parsed);
                setDocument(await service.applyProfile(sequenceDoc.job, parsed, signal));
              }, 'Profile applied: Z datum and spindle directions moved into the job.');
            }} />
        </div>
        {plan && <div className="sequence-summary">
          <dl>
            <dt>Motions</dt><dd>{plan.summary.motionCount} ({plan.summary.cuttingMotionCount} cutting)</dd>
            <dt>Basic checks</dt><dd>{plan.summary.basicChecks.status}</dd>
            <dt>Operations</dt><dd>{plan.summary.operations.map(op => `${op.operationId}: ${op.generationStatus}`).join(' · ')}</dd>
            <dt>Stages</dt><dd>{plan.summary.stages.map(stage => `${stage.role}(${stage.motionCount})`).join(' → ')}</dd>
            {plan.summary.preparationRequirements.length > 0 && <><dt>Preparation</dt><dd>{plan.summary.preparationRequirements.map(req => req.code).join(', ')}</dd></>}
          </dl>
          <button className="primary" disabled={!profile || busy === 'export'} onClick={() => sequenceDoc && profile && run('export', async signal => setExportResult(await service.export(sequenceDoc.job, profile, signal)), 'Export verified by numeric readback.')}>{busy === 'export' ? 'Exporting…' : 'Export program'}</button>
          {plan.summary.diagnostics.length > 0 && <ul className="sequence-diagnostics">{plan.summary.diagnostics.map((diagnostic, index) => <li key={index}><strong>{diagnostic.code}</strong> {diagnostic.message}</li>)}</ul>}
        </div>}
        {exportResult && <div className="sequence-export">
          <p>{exportResult.program.filename} · {exportResult.report.motionCount} motions · {exportResult.report.outputDecimalPlaces} places · work-zero offset {exportResult.report.machineOffsetMm.map(v => v.toFixed(3)).join(', ')} mm</p>
          <button onClick={downloadProgram}>Download program</button>
          <pre aria-label="Program preview">{exportResult.program.gcode.slice(0, 4000)}{exportResult.program.gcode.length > 4000 ? '\n…' : ''}</pre>
        </div>}
      </section>
    </div>
  </div>;
}
