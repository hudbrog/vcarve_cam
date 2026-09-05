import type { useVerification } from '../service/useVerification';
import type { VerificationOptions } from '../contracts/verificationOptions';
import type { StockVerification } from '../contracts/verification';
import { useState } from 'react';

type Verification = ReturnType<typeof useVerification>;
const number = (n: number) => new Intl.NumberFormat(undefined, {maximumSignificantDigits:6, notation:n !== 0 && Math.abs(n) < .0001 ? 'scientific' : 'standard'}).format(n);
const interval = (i: {lower:number;upper:number}) => `${number(i.lower)} – ${number(i.upper)}`;
const fields: {key:keyof VerificationOptions;label:string;hint:string}[] = [
  {key:'max_cells',label:'Maximum verification cells',hint:'1–2,000,000'},
  {key:'max_depth',label:'Maximum refinement depth',hint:'1–40'},
  {key:'reachability_max_cells',label:'Maximum reachability cells',hint:'1–1,000,000'},
  {key:'max_depth_bands',label:'Maximum depth bands',hint:'1–4,096'},
  {key:'max_findings',label:'Maximum findings',hint:'1–4,096'},
  {key:'decimal_places',label:'Coordinate decimal places',hint:'Optional 0–9; blank checks original coordinates only'},
];
const bounds: [keyof StockVerification['bounds'],string,string][] = [
  ['overcut_mm','Overcut depth','mm'],['floor_ridge_mm','Floor ridge','mm'],
  ['unreachable_detail_mm','Unreachable detail','mm'],['other_reachable_residual_mm','Other reachable residual','mm'],
  ['total_residual_mm','Total residual','mm'],['residual_volume_mm3','Residual volume','mm³'],['overcut_volume_mm3','Overcut volume','mm³'],
];
export function VerificationPanel({ verification: v, planCurrent, combined }: { verification: Verification; planCurrent: boolean; combined: boolean }) {
  const report = v.result?.report;
  const evidence = v.evidence;
  const key = `${report?.verification_fingerprint}/${v.scope}`;
  const [expanded, setExpanded] = useState({key, findings:20, bands:20});
  const shown = expanded.key === key ? expanded : {key,findings:20,bands:20};
  return <>
    <section className="inspector-group"><h2>Continuous stock verification</h2>
      <p>Check the recorded combined plan with the independent M5 verifier. Its intervals bound errors across the reported domain and depth bands.</p>
      {!v.available ? <p>Connect a service with M5 verification to run this check.</p> : <>
        {!planCurrent && <p className="inline-warning">Generate a current plan before verification.</p>}
        {planCurrent && !combined && <p className="inline-warning">M5 requires a combined endmill/V-bit plan. Endmill-only evidence remains in Plan & inspect.</p>}
        <details><summary>Verification settings</summary><p className="hint">These limits control refinement and report detail. Physical tolerances come from the job. Defaults are supplied by Rust.</p>
          <div className="fields">{fields.map(field => <div className="field" key={field.key}><label htmlFor={`verify-${field.key}`}>{field.label}</label><input id={`verify-${field.key}`} inputMode="numeric" value={v.text[field.key]} onChange={e => v.edit(field.key,e.target.value)} /><p className="hint">{field.hint}</p></div>)}</div>
          <button className="wide" onClick={v.useDefaults}>Use engine verification defaults</button>
        </details>
        {!v.options && <p role="alert" className="inline-warning">Complete verification limits with integers in the displayed ranges.</p>}
        <button className="primary wide" disabled={!v.canStart} onClick={v.start}>{v.submitting ? 'Submitting verification…' : 'Verify current combined plan'}</button>
        <p className="hint">Optional decimal precision adds a rounded-coordinate check. It does not check an emitted LinuxCNC program.</p>
      </>}
    </section>
    {(v.task || v.active || v.error || v.lost) && <section className="inspector-group"><h2>Verification task</h2>
      <p role="status" className="task-state">{v.lost ? 'Service restarted · verification task lost' : v.task ? ({queued:'Queued · waiting for the shared worker',running:'Running continuous verification',cancelling:'Cancelling · waiting for worker exit',cancelled:'Verification cancelled',succeeded:'Verification finished · review its outcome',failed:'Verification calculation failed'}[v.task.state]) : 'Checking verification submission…'}</p>
      {v.active && <button disabled={!v.task || v.task.state === 'cancelling'} onClick={() => void v.cancel()}>Cancel verification</button>}
      {v.error && <><p role="alert" className="inline-warning">{v.error}</p><button onClick={v.check}>Check verification task</button>{v.retry && <button onClick={v.retry}>Retry same verification</button>}<p className="hint">Disconnecting does not stop the worker or submit another calculation.</p></>}
      {v.task?.diagnostic && <p role="alert" className="inline-warning"><strong>{v.task.diagnostic.code}</strong> · {v.task.diagnostic.message}</p>}
      {v.task?.state === 'succeeded' && !v.task.resultAvailable && <p className="inline-warning">This report expired from the shared result cache. Run verification again for a retained current plan.</p>}
      {v.lost && <p className="hint">Previous work is never replayed automatically. Regenerate the plan and verify again.</p>}
    </section>}
    {report && evidence && <section className="inspector-group verification-report"><h2>{v.current ? 'Current verification' : 'Previous verification · stale'}</h2>
      <p className={`plan-outcome ${report.status}`}>Overall outcome: {report.status}</p>
      {!v.current && <p className="inline-warning">This report does not match the current plan and verification settings. Its locations are hidden.</p>}
      <label className="field-label" htmlFor="verification-scope">Evidence coordinates</label><select id="verification-scope" value={v.scope} onChange={e => v.setScope(e.target.value as 'original'|'rounded')}><option value="original">Original recorded coordinates</option>{report.rounded && <option value="rounded">Rounded to {report.rounded.decimal_places} decimal places</option>}</select>
      <p>Selected evidence: <strong>{evidence.status}</strong></p>
      {v.scope === 'rounded' && report.rounded && <p className="hint">Coordinate quantum {number(report.rounded.coordinate_quantum_mm)} mm; maximum change {number(report.rounded.maximum_coordinate_change_mm)} mm.</p>}
      <table className="stock-metrics"><caption>Error bounds · lower – upper</caption><tbody>{bounds.map(([key,label,unit]) => <tr key={key}><th scope="row">{label} · {unit}</th><td>{interval(evidence.bounds[key])}</td></tr>)}</tbody></table>
      <dl><dt>Verification tolerance</dt><dd>{number(evidence.verification_tolerance_mm)} mm</dd><dt>Floor ridge limit</dt><dd>{number(evidence.floor_ridge_limit_mm)} mm</dd><dt>Detail residual limit</dt><dd>{number(evidence.detail_residual_limit_mm)} mm</dd><dt>Evaluated cells</dt><dd>{number(evidence.evaluated_cells)}</dd><dt>Unresolved cells</dt><dd>{number(evidence.unresolved_cells)}</dd><dt>Maximum uncertainty</dt><dd>{number(evidence.maximum_error_uncertainty_mm)} mm</dd></dl>
      <h3>Findings · {evidence.findings.length}</h3>
      {evidence.findings.length === 0 && <p>No findings in this evidence scope.</p>}
      {evidence.findings.slice(0,shown.findings).map((finding,index) => <div className={`verification-finding ${finding.status}`} key={index}>
        <p><strong>{finding.code}</strong> · {finding.status}</p><p>{finding.message}</p>
        {finding.measured_mm && <p className="hint">Measured {interval(finding.measured_mm)} mm{finding.limit_mm !== null && ` · limit ${number(finding.limit_mm)} mm`}</p>}
        <p className="hint">X {number(finding.location.x)}, Y {number(finding.location.y)} mm{finding.motion_id !== null && ` · motion ${finding.motion_id}`}{finding.cell ? ' · bounded cell' : ' · engine-reported point'}</p>
        <button disabled={!v.current} onClick={() => v.locate(index)}>Locate finding {index + 1}</button>
      </div>)}
      {evidence.findings.length > shown.findings && <button onClick={() => setExpanded({...shown,findings:shown.findings + 20})}>Show next findings · {shown.findings} of {evidence.findings.length} shown</button>}
      {evidence.omitted_findings > 0 && <p className="hint">{evidence.omitted_findings} additional findings omitted by the report budget.</p>}
      {v.finding && <button onClick={v.clearFinding}>Clear finding highlight</button>}
      <details><summary>Depth-band bounds · {evidence.depth_bands.length}</summary><p className="hint">Area intervals apply to every horizontal slice throughout each closed depth band.</p>
        {evidence.depth_bands.slice(0,shown.bands).map((band,index) => <details key={index}><summary>{number(band.from_depth_mm)}–{number(band.to_depth_mm)} mm below stock top</summary><dl><dt>Nominal area</dt><dd>{interval(band.nominal_area_mm2)} mm²</dd><dt>Removed area</dt><dd>{interval(band.removed_area_mm2)} mm²</dd><dt>Residual area</dt><dd>{interval(band.residual_area_mm2)} mm²</dd><dt>Overcut area</dt><dd>{interval(band.overcut_area_mm2)} mm²</dd></dl></details>)}
        {evidence.depth_bands.length > shown.bands && <button onClick={() => setExpanded({...shown,bands:shown.bands + 20})}>Show next depth bands · {shown.bands} of {evidence.depth_bands.length} shown</button>}
      </details>
      <details><summary>Limits and report identity</summary><ul>{evidence.limitations.map((limit,index) => <li key={index}>{limit}</li>)}</ul><dl><dt>Engine</dt><dd>{report.engine_version}</dd><dt>Source plan task</dt><dd><code>{v.result!.task.verification.planTaskId}</code></dd><dt>Verification fingerprint</dt><dd><code>{report.verification_fingerprint}</code></dd><dt>Authenticated plan</dt><dd><code>{report.authenticated_plan_fingerprint}</code></dd><dt>Checked motions</dt><dd>{evidence.checked_motion_count}</dd><dt>Source depth error</dt><dd>{number(evidence.source_geometry_depth_error_mm)} mm</dd><dt>Arithmetic reserve</dt><dd>{number(evidence.arithmetic_reserve_mm)} mm</dd></dl></details>
      <p className="hint">Use Export to configure the machine profile and verify the exact emitted LinuxCNC program.</p>
    </section>}
  </>;
}
