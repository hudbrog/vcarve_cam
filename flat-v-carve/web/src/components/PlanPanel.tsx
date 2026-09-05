import type { Job } from '../contracts/job';
import type { Capabilities, Validation } from '../contracts/service';
import type { PlanningStage } from '../contracts/planning';
import type { usePlanning } from '../service/usePlanning';

type Planning = ReturnType<typeof usePlanning>;
export function PlanPanel({ planning, capabilities, job, validation, revision, stage, onStage }: {
  planning: Planning; capabilities: Capabilities; job: Job | null; validation: Validation | undefined;
  revision: number; stage: PlanningStage; onStage: (stage: PlanningStage) => void;
}) {
  const { task, result } = planning;
  const checked = !!job && validation?.valid && validation.authoritative && validation.revision === revision && !!validation.documentFingerprint;
  const enabled = capabilities.planningStages.includes(stage) && !!capabilities.planning;
  return <>
    <section className="inspector-group"><h2>Planning stage</h2><label className="field-label" htmlFor="plan-mode">Tool sequence</label>
      <select id="plan-mode" value={stage} onChange={event => onStage(event.target.value as PlanningStage)}><option value="combined">Combined · endmill then V-bit</option><option value="endmill">Endmill only</option></select>
      <p className="hint">{stage === 'combined' ? 'Combined planning needs explicit settings for both tools.' : 'Endmill planning still needs V-bit geometry to define the target.'} Rust checks the selected stage before calculating.</p>
      <button className="primary wide" disabled={!enabled || !checked || planning.active || planning.submitting} onClick={() => planning.start(job!)}>{planning.submitting ? 'Submitting…' : enabled ? 'Check setup & generate plan' : 'Planning unavailable'}</button>
      {!checked && <p className="hint">Resolve unfinished fields and wait for the current Rust document check.</p>}
      <p className="hint">You can keep editing during calculation. Results belong to the submitted snapshot. Planning alone does not authorize machine output.</p>
    </section>
    {(task || planning.active || planning.error || planning.lost) && <section className="inspector-group"><h2>Background task</h2>
      <p role="status" className="task-state">{planning.lost ? 'Service restarted · task lost' : task ? ({ queued: 'Queued · waiting for the planner', running: `Running ${task.stage} planner`, cancelling: 'Cancelling · waiting for worker exit', cancelled: 'Cancelled · calculation stopped', succeeded: 'Calculation finished · review its outcome', failed: 'Planning failed' }[task.state]) : planning.submitting ? 'Submitting immutable snapshot…' : 'Checking task status…'}</p>
      {task && <dl><dt>Submitted revision</dt><dd>{task.revision}</dd><dt>Stage</dt><dd>{task.stage}</dd><dt>Task</dt><dd><code>{task.taskId}</code></dd></dl>}
      {planning.active && <button className="wide" disabled={!task || task.state === 'cancelling'} onClick={() => void planning.cancel()}>Cancel calculation</button>}
      {task?.diagnostic && <p role="alert" className="inline-warning"><strong>{task.diagnostic.code}</strong> · {task.diagnostic.message}</p>}
      {planning.error && <><p role="alert" className="inline-warning">{planning.error}</p><p className="hint">A disconnected request does not stop a worker. Reconnect or check this task to learn its state.</p><button onClick={planning.check}>Check task</button>{planning.retrySubmission && <button onClick={planning.retrySubmission}>Retry same submission</button>}</>}
      {planning.lost && <p className="hint">Previous tasks are never replayed automatically. Generate a new plan when the current draft is checked.</p>}
      {planning.restored && <p className="hint">Recovered task from this tab. Regenerate any recovered plan to bind its motion preview to this visit’s draft revision.</p>}
      {task?.state === 'succeeded' && !task.resultAvailable && <p className="hint">The motion preview and plan artifact expired. The service retains the latest {capabilities.planning?.retainedResults} results.</p>}
    </section>}
    {result?.task.summary && <section className="inspector-group plan-result"><h2>{planning.current ? 'Current plan' : 'Previous plan · stale'}</h2>
      <p className={`plan-outcome ${result.task.summary.status}`}>Outcome: {result.task.summary.status}</p>
      {!planning.current && <p className="inline-warning">This result does not match the current draft, stage, or service. Its motions are hidden.</p>}
      <dl><dt>Stage / revision</dt><dd>{result.task.stage} / {result.task.revision}</dd><dt>Recorded motions</dt><dd>{result.task.summary.motionCount}</dd><dt>Cutting motions</dt><dd>{result.task.summary.cuttingMotionCount}</dd><dt>Previewed motions</dt><dd>{result.task.summary.previewMotionCount}</dd></dl>
      {result.task.summary.omittedMotionCount > 0 && <p className="inline-warning">Preview shows the first {result.task.summary.previewMotionCount} motions; {result.task.summary.omittedMotionCount} are omitted.</p>}
      <p>{result.task.summary.meaning}</p>
      {result.task.summary.diagnostics.map((d, i) => <p className="inline-warning" key={`d${i}`}><strong>{d.code}</strong> · {d.message}</p>)}
      {result.task.summary.generationIssues.map((d, i) => <p className="inline-warning" key={`g${i}`}><strong>{d.code}</strong> · {d.message}</p>)}
      {(result.task.summary.omittedDiagnostics + result.task.summary.omittedGenerationIssues > 0) && <p className="hint">Additional diagnostics remain in the engine plan artifact.</p>}
      <details><summary>Evidence limits & identity</summary><ul>{result.task.summary.limitations.map((text, i) => <li key={i}>{text}</li>)}</ul><dl><dt>Engine</dt><dd>{result.task.engineVersion}</dd><dt>Input fingerprint</dt><dd><code>{result.task.summary.inputFingerprint}</code></dd><dt>Motion fingerprint</dt><dd><code>{result.task.summary.motionFingerprint}</code></dd></dl></details>
      <p className="hint">Recorded motions are shown in workpiece coordinates. This view is not a stock simulation or a geometric certificate.</p>
    </section>}
  </>;
}
