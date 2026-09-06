import { useEffect, useMemo, useRef, useState } from 'react';
import type { CamService, Capabilities } from '../contracts/service';
import type { PlanTask } from '../contracts/planning';
import { acceptVerification, currentVerification, verificationIdentity, verificationIdentitySchema,
  type VerificationIdentity, type VerificationTask, type VerificationResult } from '../contracts/verification';
import { verificationOptionsSchema, type VerificationOptions } from '../contracts/verificationOptions';

const recoveryKey = 'flat-v-carve:u5:verification';
const optionsKey = 'flat-v-carve:u5:verification-options';
type OptionText = Record<keyof VerificationOptions, string>;
function recover(): VerificationIdentity | null {
  try { const parsed = verificationIdentitySchema.safeParse(JSON.parse(sessionStorage.getItem(recoveryKey) ?? 'null')); return parsed.success ? parsed.data : null; }
  catch { return null; }
}
function textOptions(options?: VerificationOptions): OptionText {
  return Object.fromEntries(Object.keys(verificationOptionsSchema.shape).map(key => [key, String(options?.[key as keyof VerificationOptions] ?? '')])) as OptionText;
}
function recoverOptions(fallback?: VerificationOptions): OptionText {
  try {
    const saved: unknown = JSON.parse(sessionStorage.getItem(optionsKey) ?? 'null');
    if (saved && typeof saved === 'object' && !Array.isArray(saved)
      && Object.keys(saved).length === Object.keys(verificationOptionsSchema.shape).length
      && Object.keys(verificationOptionsSchema.shape).every(key => typeof (saved as Record<string,unknown>)[key] === 'string')) return saved as OptionText;
  } catch { /* Missing/invalid options do not discard the editable job. */ }
  return textOptions(fallback);
}
export function parseVerificationOptions(text: OptionText): VerificationOptions | null {
  const values = Object.fromEntries(Object.entries(text).map(([key, value]) => [key,
    key === 'decimal_places' && value.trim() === '' ? null : /^\d+$/.test(value.trim()) ? Number(value) : NaN]));
  const parsed = verificationOptionsSchema.safeParse(values); return parsed.success ? parsed.data : null;
}
export function useVerification(service: CamService, capabilities: Capabilities, plan: PlanTask | null, planCurrent: boolean, refresh: number) {
  const [handle, setHandle] = useState(recover);
  const handleRef = useRef(handle); handleRef.current = handle;
  const [text, setText] = useState(() => recoverOptions(handle?.verification.options ?? capabilities.verification?.defaultOptions));
  useEffect(() => { try { sessionStorage.setItem(optionsKey,JSON.stringify(text)); } catch { /* Options remain editable for this visit. */ } }, [text]);
  const options = useMemo(() => parseVerificationOptions(text), [text]);
  const [task, setTask] = useState<VerificationTask | null>(null);
  const taskRef = useRef(task);
  const [result, setResult] = useState<VerificationResult | null>(null);
  const [error, setError] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const submittingRef = useRef(false);
  const [attempt, setAttempt] = useState(0);
  const [missing, setMissing] = useState(false);
  const [scope, setScope] = useState<'original' | 'rounded'>('original');
  const [selection, setSelection] = useState<{ key: string; index: number; serial: number } | null>(null);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const available = capabilities.verificationScopes.includes('continuous-stock') && !!capabilities.verification && !!service.startVerification;
  const lost = !!handle && (handle.instanceId !== capabilities.planning?.instanceId || handle.engineVersion !== capabilities.engineVersion);
  function accept(next: VerificationTask, identity: VerificationIdentity) {
    if (!mounted.current || handleRef.current?.taskId !== identity.taskId) return;
    const accepted = acceptVerification(taskRef.current, next, identity);
    taskRef.current = accepted; setTask(accepted);
  }
  useEffect(() => {
    if (!handle || submitting || lost || !service.verificationTask) return;
    const controller = new AbortController(); let timer: ReturnType<typeof setTimeout>;
    setMissing(false);
    async function poll() {
      try {
        const next = await service.verificationTask!(handle!, controller.signal);
        if (controller.signal.aborted) return;
        accept(next, handle!); setError('');
        if (next.state === 'succeeded' && next.resultAvailable && service.verificationResult) {
          const nextResult = await service.verificationResult(handle!, controller.signal);
          if (!controller.signal.aborted) { accept(nextResult.task, handle!); setResult(nextResult); }
        } else if (!['cancelled', 'failed', 'succeeded'].includes(next.state)) timer = setTimeout(() => void poll(), 700);
      } catch (e) { if (!controller.signal.aborted) { setError(String(e)); setMissing(String(e).includes('TASK_NOT_FOUND')); } }
    }
    void poll(); return () => { controller.abort(); clearTimeout(timer); };
  }, [service, handle, lost, submitting, attempt, refresh]);
  async function submit(identity: VerificationIdentity) {
    if (!service.startVerification || submittingRef.current) return;
    submittingRef.current = true; setSubmitting(true); setError(''); setMissing(false);
    handleRef.current = identity; setHandle(identity); taskRef.current = null; setTask(null);
    try { sessionStorage.setItem(recoveryKey, JSON.stringify(identity)); } catch { /* Calculation still works. */ }
    try { accept(await service.startVerification(identity), identity); }
    catch (e) { if (mounted.current) setError(String(e)); }
    finally { submittingRef.current = false; if (mounted.current) setSubmitting(false); }
  }
  const active = !!handle && !lost && !missing && (!task || !['succeeded', 'failed', 'cancelled'].includes(task.state));
  const canStart = available && planCurrent && plan?.stage === 'combined' && !!plan.summary && !!options && !active && !submitting;
  const current = currentVerification(result, plan, planCurrent, options);
  const reportScope = scope === 'rounded' && result?.report.rounded ? 'rounded' : 'original';
  const evidence = result ? reportScope === 'rounded' ? result.report.rounded!.verification : result.report.original : null;
  const key = current && result ? `${result.task.taskId}/${reportScope}` : null;
  const finding = key && selection?.key === key ? evidence?.findings[selection.index] : undefined;
  const focus = useMemo(() => finding && selection ? { bounds: finding.cell ?? {
    min: {x:finding.location.x - 1,y:finding.location.y - 1}, max:{x:finding.location.x + 1,y:finding.location.y + 1},
  }, serial: selection.serial } : null, [finding, selection]);
  return { text, options, edit: (key: keyof VerificationOptions, value: string) => setText(previous => ({...previous,[key]:value})),
    useDefaults: () => setText(textOptions(capabilities.verification?.defaultOptions)), available, canStart, current, active, submitting, task, result, error, lost,
    start: () => { if (canStart) void submit(verificationIdentity(plan!, options!, crypto.randomUUID())); },
    cancel: async () => {
      if (!handle || !service.cancelVerification) return;
      try { accept(await service.cancelVerification(handle), handle); setAttempt(n => n + 1); }
      catch (e) { if (mounted.current) setError(String(e)); }
    },
    check: () => setAttempt(n => n + 1), retry: handle && !task && !lost ? () => void submit(handle) : null,
    scope: reportScope, setScope, evidence, finding, focus,
    locate: (index: number) => { if (key) setSelection(previous => ({key,index,serial:(previous?.serial ?? 0) + 1})); },
    clearFinding: () => setSelection(null),
    label: active ? 'Verification running' : current ? `Verification ${result!.report.status}` : result ? 'Previous verification stale' : available ? 'Not verified' : 'Unavailable',
  };
}
