import { useRef, useState } from 'react';
import type { Job } from '../contracts/job';
import { profileSchema, type ProgramLayout } from '../contracts/export';
import { profileDraft, profileFields, profileFieldActive, reviewedProfile } from '../state/profile';
import type { useExport } from '../service/useExport';

type Export = ReturnType<typeof useExport>;
// Call only inside a click, after the adapter has checked report and program hashes.
export function downloadText(filename:string, text:string, type='application/json') {
  const url = URL.createObjectURL(new Blob([text],{type}));
  const anchor = document.createElement('a'); anchor.href = url; anchor.download = filename; anchor.click();
  setTimeout(() => URL.revokeObjectURL(url),1000);
}
const groups = [
  {title:'Machine coordinates & formatting',match:(p:string) => !p.startsWith('tools.') && !p.startsWith('m6.') && !p.startsWith('program_start') && p !== 'start_mode'},
  {title:'Tool mapping',match:(p:string) => p.startsWith('tools.')},
  {title:'Startup & M6 positioning',match:(p:string) => p.startsWith('program_start') || p === 'start_mode' || p.startsWith('m6.return_position.')},
  {title:'Declared M6 contract',match:(p:string) => p.startsWith('m6.') && !p.startsWith('m6.return_position.')},
];
const number = (n:number) => new Intl.NumberFormat(undefined,{maximumSignificantDigits:6,notation:n !== 0 && Math.abs(n) < .0001 ? 'scientific' : 'standard'}).format(n);
export function ExportPanel({output:o,job,planCurrent}:{output:Export;job:Job;planCurrent:boolean}) {
  const input = useRef<HTMLInputElement>(null);
  const draftRef = useRef(o.draft); draftRef.current = o.draft;
  const [fileError,setFileError] = useState('');
  const [notice,setNotice] = useState('');
  const [scope,setScope] = useState<'original'|'emitted'>('emitted');
  const [shown,setShown] = useState(20);
  const report = o.result?.report;
  const evidence = report ? scope === 'original' ? report.plan_verification.original : report.emitted_verification : null;
  async function open(file:File|undefined) {
    if (!file) return;
    const previous = draftRef.current; setFileError('');
    try {
      if (file.size > 64_000) throw new Error('Machine profile exceeds 64 KB.');
      const parsed = profileSchema.safeParse(JSON.parse(await file.text()));
      if (!parsed.success) throw new Error(`Cannot open machine profile: ${parsed.error.issues.slice(0,3).map(i => `${i.path.join('.')}: ${i.message}`).join('; ')}`);
      if (draftRef.current !== previous) throw new Error('The profile changed while this file was opening. Choose the file again; your edits were kept.');
      o.setDraft(profileDraft(parsed.data)); setNotice('Machine profile opened. Rust checks its compatibility with the plan when output is generated.');
    } catch(e) { setFileError(String(e)); }
    finally { if (input.current) input.current.value = ''; }
  }
  function download(filename:string,text:string,type?:string) {
    downloadText(filename,text,type); setNotice(`${filename} download requested. Check your browser’s downloads to confirm it was saved.`);
  }
  return <>
    {notice && <p role="status" className="hint">{notice}</p>}
    <section className="inspector-group"><h2>LinuxCNC machine profile</h2>
      <p>Configure the machine work frame, tool mapping and M6 behavior. This profile is saved separately from the portable job.</p>
      <input ref={input} type="file" accept=".json,application/json" aria-label="LinuxCNC profile file" className="sr-only" tabIndex={-1} onChange={e => void open(e.target.files?.[0])} />
      <div className="inline-actions"><button onClick={() => input.current?.click()}>Open machine profile…</button><button disabled={!o.profile} onClick={() => { if (o.profile) download('machine-profile.json',JSON.stringify(o.profile,null,2)+'\n'); }}>Download profile</button></div>
      {fileError && <p role="alert" className="inline-warning">{fileError}</p>}
      {o.recoveryError && <p role="alert" className="inline-warning">{o.recoveryError}</p>}
      <p className="hint">Profile downloads contain settings, without a verification claim. Unfinished fields are recovered in this tab.</p>
      <button className="wide" onClick={() => o.setDraft(previous => ({...previous,
        'tools.0.tool_id':job.operation.endmill_id,'tools.1.tool_id':job.operation.vbit_id,
        clearance_z_mm:String(job.endmill_planning?.clearance_z_mm ?? ''),
      }))}>Copy job tool IDs and planning clearance</button>
      {groups.map(group => <details key={group.title} open={group.title === 'Machine coordinates & formatting' ? true : undefined}>
        <summary>{group.title}</summary>
        {group.title === 'Startup & M6 positioning' && <p className="hint">Startup and return positions use the selected work frame and the new compensated tool tip. Safe retract declares a clear upward corridor to Z and clear XY travel at that plane. An unknown startup requires this retract contract.</p>}
        {group.title === 'Declared M6 contract' && <p className="hint">Record the actual macro/configuration reference. Reviewing a declaration does not establish machine testing.</p>}
        <div className="fields">{profileFields.filter(f => group.match(f.path) && profileFieldActive(f.path,o.draft)).map(field => {
          const value = o.draft[field.path] ?? '';
          const error = value ? o.errors[field.path] : undefined;
          const edit = (text:string) => o.setDraft(previous => ({...previous,[field.path]:text}));
          const id = `profile-${field.path}`;
          return <div className="field" key={field.path}>
            {field.kind === 'boolean' ? <label className="profile-check"><input id={id} type="checkbox" checked={value === 'true'} onChange={e => edit(String(e.target.checked))} />{field.label}</label> : <>
              <label htmlFor={id}>{field.label}</label>
              {field.choices ? <select id={id} value={value} onChange={e => edit(e.target.value)}><option value="">Choose…</option>{field.choices.map(([v,label]) => <option value={v} key={v}>{label}</option>)}</select>
                : field.kind === 'multiline' ? <textarea id={id} rows={5} value={value} onChange={e => edit(e.target.value)} />
                : <input id={id} value={value} placeholder="Not specified" inputMode={field.kind === 'number' ? 'decimal' : 'text'} aria-invalid={!!error} aria-describedby={error ? `${id}-error` : undefined} onChange={e => edit(e.target.value)} />}
              {error && <span className="field-error" id={`${id}-error`}>{error}</span>}
            </>}
          </div>;
        })}</div>
      </details>)}
      {!o.profile && <p className="inline-warning">Complete the profile fields before generating output.</p>}
      {o.profile && !reviewedProfile(o.profile) && <p className="inline-warning">Supply a reference, confirm the M6 declarations and mark the contract reviewed.</p>}
      <p className="hint">Planning clearance is above stock top. Stock-bottom Z output uses the job’s stock thickness. Rust checks precision, clearance and any legacy machine constraints in the job.</p>
    </section>
    <section className="inspector-group"><h2>Generate & check output</h2>
      {!o.available && <p className="inline-warning">Connect a service with LinuxCNC export support.</p>}
      {!planCurrent && <p className="inline-warning">Generate a current combined plan first.</p>}
      <label className="field-label" htmlFor="export-layout">Program layout</label><select id="export-layout" value={o.layout} onChange={e => o.setLayout(e.target.value as ProgramLayout)}><option value="combined">Combined program</option><option value="per_tool">Separate files per tool</option></select>
      <p className="hint">Rust rechecks the original plan, emits the program, independently reads it back and verifies the emitted motions. Verification cell and report budgets come from the Verification step; output precision comes from this profile.</p>
      <p className="hint">The drawing shows recorded plan paths. The program preview below shows emitted machine coordinates.</p>
      {!o.options && <p className="inline-warning">Complete the verification settings before generating output.</p>}
      <button className="primary wide" disabled={!o.canStart} onClick={o.start}>{o.submitting ? 'Submitting export…' : 'Generate checked LinuxCNC output'}</button>
      {(o.task || o.active || o.lost) && <p role="status" className="task-state">{o.lost ? 'Service restarted · export task lost' : o.task ? ({queued:'Queued · waiting for the shared worker',running:'Generating and verifying output',cancelling:'Cancelling · waiting for worker exit',cancelled:'Export cancelled',succeeded:'Export finished · review its outcome',failed:'Export calculation failed'}[o.task.state]) : 'Checking export submission…'}</p>}
      {o.active && <button disabled={!o.task || o.task.state === 'cancelling'} onClick={() => void o.cancel()}>Cancel export</button>}
      {o.error && <><p role="alert" className="inline-warning">{o.error}</p><button onClick={o.check}>Check export task</button>{o.retry && <button onClick={o.retry}>Retry same export</button>}</>}
      {o.task?.diagnostic && <p role="alert" className="inline-warning"><strong>{o.task.diagnostic.code}</strong> · {o.task.diagnostic.message}</p>}
      {o.task?.state === 'succeeded' && !o.task.resultAvailable && <p className="hint">Server result expired. Any already loaded, hash-checked bytes remain usable only while the plan and profile match.</p>}
    </section>
    {report && <section className="inspector-group verification-report"><h2>{o.current ? 'Current checked output' : 'Previous output · stale'}</h2>
      <p className={`plan-outcome ${report.status}`}>Output outcome: {report.status}</p>
      {!o.current && <p className="inline-warning">The plan, machine profile, layout or verification settings changed. Program downloads are disabled.</p>}
      <dl><dt>Original plan</dt><dd>{report.plan_verification.status}</dd><dt>Emitted program</dt><dd>{report.emitted_verification?.status ?? 'Not reached'}</dd><dt>Output Z offset</dt><dd>{report.machine_z_offset_mm} mm</dd><dt>Profile</dt><dd>{report.profile.id} · {report.profile.work_offset} · {report.profile.z_datum === 'stock_top' ? 'stock top' : 'stock bottom'}</dd></dl>
      {report.status !== 'passed' && <p className="inline-warning">No machine programs are available for this outcome. Review the findings and download the report.</p>}
      {report.diagnostics.map((d,i) => <p key={i} className="inline-warning"><strong>{d.code}</strong> · {d.message}</p>)}
      <button className="wide" onClick={() => download('export-report.json',o.result!.reportJson)}>Download export report</button>
      {report.programs.map((program,index) => <div className="output-program" key={program.filename}>
        <h3>{program.filename}</h3><p>{program.motion_count} checked motions · {program.tool_changes} tool changes</p>
        {program.prerequisites.map((p,i) => <p className="inline-warning" key={i}>{p}</p>)}
        <p className="hint">SHA-256 <code>{program.sha256}</code></p>
        <button className="primary wide" disabled={!o.downloadable} onClick={() => { if (o.downloadable) download(program.filename,o.result!.programs[index].gcode,'text/plain'); }}>Download {program.filename}</button>
        {o.result!.programs[index] && <details><summary>Program preview · first 80 lines</summary><pre className="program-preview">{o.result!.programs[index].gcode.split('\n',80).join('\n')}</pre><p className="hint">Preview may be truncated. Download contains every checked byte.</p></details>}
      </div>)}
      <details><summary>Verification evidence & findings</summary>
        <label className="field-label" htmlFor="export-evidence">Evidence scope</label><select id="export-evidence" value={scope} onChange={e => {setScope(e.target.value as 'original'|'emitted');setShown(20);}}><option value="emitted">Emitted program motions</option><option value="original">Original plan</option></select>
        {!evidence ? <p>Emitted stock verification was not reached. Inspect the original-plan findings or the export diagnostics.</p> : <>
          <p>{evidence.evaluated_cells} evaluated cells · {evidence.unresolved_cells} unresolved. Coordinates below are workpiece millimeters from stock top, after translating output Z back from the machine datum.</p>
          <table className="stock-metrics"><caption>Error bounds · lower – upper (mm)</caption><tbody>{([['overcut_mm','Overcut'],['floor_ridge_mm','Floor ridge'],['total_residual_mm','Total residual']] as const).map(([key,label]) => <tr key={key}><th scope="row">{label}</th><td>{number(evidence.bounds[key].lower)} – {number(evidence.bounds[key].upper)}</td></tr>)}</tbody></table>
          {!evidence.findings.length && <p>No findings in this scope.</p>}
          {evidence.findings.slice(0,shown).map((f,i) => <div className="verification-finding" key={i}><p><strong>{f.code}</strong> · {f.status}</p><p>{f.message}</p><p className="hint">X {f.location.x}, Y {f.location.y} mm{f.measured_mm && ` · measured ${f.measured_mm.lower} – ${f.measured_mm.upper} mm`}{f.limit_mm !== null && ` · limit ${f.limit_mm} mm`}</p></div>)}
          {evidence.findings.length > shown && <button onClick={() => setShown(n => n + 20)}>Show next findings</button>}
          {evidence.omitted_findings > 0 && <p>{evidence.omitted_findings} findings omitted by the report budget.</p>}
          <ul>{evidence.limitations.map((l,i) => <li key={i}>{l}</li>)}</ul>
        </>}
      </details>
      <details><summary>Machine prerequisites & output identity</summary><ul>{report.limitations.map((l,i) => <li key={i}>{l}</li>)}</ul><dl><dt>Profile fingerprint</dt><dd><code>{report.profile_fingerprint}</code></dd><dt>Emitted motions</dt><dd><code>{report.emitted_motion_fingerprint ?? 'Not available'}</code></dd><dt>Report SHA-256</dt><dd><code>{o.result!.task.summary!.reportFingerprint}</code></dd></dl></details>
    </section>}
  </>;
}
