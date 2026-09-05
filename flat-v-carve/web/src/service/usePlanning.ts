import { useEffect, useRef, useState } from 'react';
import type { Job } from '../contracts/job';
import type { CamService, Capabilities, Validation } from '../contracts/service';
import { acceptTask, currentPlan, taskIdentitySchema, terminal, type PlanResult, type PlanTask, type PlanningStage, type TaskIdentity } from '../contracts/planning';

const recoveryKey = 'flat-v-carve:u3:task';
type Handle = { identity: TaskIdentity; restored: boolean };
function recover(): Handle | null {
  try {
    const parsed = taskIdentitySchema.safeParse(JSON.parse(sessionStorage.getItem(recoveryKey) ?? 'null'));
    return parsed.success ? { identity: parsed.data, restored: true } : null;
  } catch { return null; }
}
export function usePlanning(service: CamService, capabilities: Capabilities, validation: Validation | undefined, revision: number, stage: PlanningStage, refresh: number) {
  const [handle, setHandle] = useState<Handle | null>(recover);
  const handleRef = useRef(handle); handleRef.current = handle;
  const [task, setTask] = useState<PlanTask | null>(null);
  const taskRef = useRef(task);
  const [result, setResult] = useState<{ value: PlanResult; restored: boolean } | null>(null);
  const [error, setError] = useState('');
  const [submissionError, setSubmissionError] = useState('');
  const [missing, setMissing] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const submittingRef = useRef(false);
  const submittedJob = useRef<Job | null>(null);
  const [attempt, setAttempt] = useState(0);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  function accept(next: PlanTask, identity: TaskIdentity) {
    if (!mounted.current || handleRef.current?.identity.taskId !== identity.taskId) return;
    const accepted = acceptTask(taskRef.current, next, identity);
    setSubmissionError('');
    taskRef.current = accepted; setTask(accepted);
  }
  const lost = !!handle && (handle.identity.instanceId !== capabilities.planning?.instanceId || handle.identity.engineVersion !== capabilities.engineVersion);
  useEffect(() => {
    if (!handle || submitting || lost || !service.planTask) return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    setError(''); setMissing(false);
    async function poll() {
      try {
        const next = await service.planTask!(handle!.identity, controller.signal);
        if (controller.signal.aborted) return;
        accept(next, handle!.identity);
        if (terminal(next)) {
          if (next.state === 'succeeded' && next.resultAvailable && service.planResult) {
            const value = await service.planResult(handle!.identity, controller.signal);
            if (!controller.signal.aborted) {
              accept(value.task, handle!.identity);
              setResult({ value, restored: handle!.restored });
            }
          }
        } else timer = setTimeout(() => void poll(), 700);
      } catch (error) {
        if (!controller.signal.aborted) {
          setError(String(error)); setMissing(String(error).includes('TASK_NOT_FOUND'));
        }
      }
    }
    void poll();
    return () => { controller.abort(); clearTimeout(timer); };
  }, [service, handle, submitting, lost, refresh, attempt]);
  async function submit(job: Job, next: Handle) {
    if (!service.startPlan || submittingRef.current) return;
    submittingRef.current = true; setSubmitting(true); setError(''); setSubmissionError(''); setMissing(false);
    submittedJob.current = structuredClone(job);
    handleRef.current = next; taskRef.current = null; setTask(null); setHandle(next);
    try { sessionStorage.setItem(recoveryKey, JSON.stringify(next.identity)); } catch { /* Planning works without recovery. */ }
    try { accept(await service.startPlan(submittedJob.current, next.identity), next.identity); }
    catch (error) { if (mounted.current) setSubmissionError(String(error)); }
    finally { submittingRef.current = false; if (mounted.current) setSubmitting(false); }
  }
  function start(job: Job) {
    if (!capabilities.planning || !validation?.valid || !validation.authoritative || validation.revision !== revision || !validation.documentFingerprint) return;
    void submit(job, { restored: false, identity: { taskId: crypto.randomUUID(), instanceId: capabilities.planning.instanceId,
      engineVersion: capabilities.engineVersion, revision, documentFingerprint: validation.documentFingerprint, stage } });
  }
  async function cancel() {
    if (!handle || !service.cancelPlan) return;
    setError('');
    try { accept(await service.cancelPlan(handle.identity), handle.identity); setAttempt(value => value + 1); }
    catch (error) { if (mounted.current) setError(String(error)); }
  }
  const active = !!handle && !lost && !missing && (!task || !terminal(task));
  const current = !!result && !result.restored && currentPlan(result.value.task, validation, revision, stage, capabilities);
  return { task, result: result?.value ?? null, current, error: [submissionError, error].filter(Boolean).join(' '), lost, restored: !!handle?.restored, active, submitting,
    start, cancel, check: () => setAttempt(value => value + 1),
    retrySubmission: submittedJob.current && handle && !task && !lost ? () => void submit(submittedJob.current!, handle) : null };
}
