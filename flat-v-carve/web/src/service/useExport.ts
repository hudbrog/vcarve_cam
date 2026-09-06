import { useEffect, useMemo, useRef, useState } from 'react';
import type { CamService, Capabilities } from '../contracts/service';
import type { PlanTask } from '../contracts/planning';
import type { Job } from '../contracts/job';
import { acceptExport, currentExport, exportIdentity, exportIdentitySchema, type ExportIdentity, type ExportTask, type ExportResult,
  type ProgramLayout } from '../contracts/export';
import type { VerificationOptions } from '../contracts/verificationOptions';
import { parseProfileDraft, recoverProfile, reviewedProfile } from '../state/profile';

const recoveryKey = 'flat-v-carve:u6:export';
function recover():ExportIdentity|null {
  try { const parsed = exportIdentitySchema.safeParse(JSON.parse(sessionStorage.getItem(recoveryKey) ?? 'null')); return parsed.success ? parsed.data : null; }
  catch { return null; }
}
export function useExport(service:CamService, capabilities:Capabilities, plan:PlanTask|null, planCurrent:boolean,
  verificationOptions:VerificationOptions|null, refresh:number, job:Job) {
  const [draft,setDraft] = useState(() => recoverProfile({getItem:key => sessionStorage.getItem(key)}));
  const {profile,errors} = useMemo(() => parseProfileDraft(draft,job),[draft,job]);
  const [layout,setLayout] = useState<ProgramLayout>(() => { try { return sessionStorage.getItem('flat-v-carve:u6:layout') === 'per_tool' ? 'per_tool' : 'combined'; } catch { return 'combined'; } });
  const [recoveryError,setRecoveryError] = useState('');
  useEffect(() => {
    try { sessionStorage.setItem('flat-v-carve:u6:profile',JSON.stringify(draft)); sessionStorage.setItem('flat-v-carve:u6:layout',layout); setRecoveryError(''); }
    catch { setRecoveryError('Profile recovery is unavailable. Download the profile to preserve completed settings.'); }
  },[draft,layout]);
  // M6 rechecks original and emitted coordinates. Its profile owns formatting;
  // M5's optional rounded-coordinate check is not an output setting.
  const options = useMemo(() => verificationOptions ? {...verificationOptions,decimal_places:null} : null,[verificationOptions]);
  const [handle,setHandle] = useState(recover);
  const handleRef = useRef(handle); handleRef.current = handle;
  const [task,setTask] = useState<ExportTask|null>(null);
  const taskRef = useRef(task);
  const [result,setResult] = useState<ExportResult|null>(null);
  const [error,setError] = useState('');
  const [submitting,setSubmitting] = useState(false);
  const submittingRef = useRef(false);
  const [attempt,setAttempt] = useState(0);
  const [missing,setMissing] = useState(false);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; },[]);
  const available = capabilities.exportFormats.includes('linuxcnc') && !!capabilities.export && !!service.startExport;
  const lost = !!handle && (handle.instanceId !== capabilities.planning?.instanceId || handle.engineVersion !== capabilities.engineVersion);
  function accept(next:ExportTask, identity:ExportIdentity) {
    if (!mounted.current || handleRef.current?.taskId !== identity.taskId) return;
    const accepted = acceptExport(taskRef.current,next,identity);
    taskRef.current = accepted; setTask(accepted);
  }
  useEffect(() => {
    if (!handle || submitting || lost || !service.exportTask) return;
    const controller = new AbortController(); let timer:ReturnType<typeof setTimeout>;
    setMissing(false);
    async function poll() {
      try {
        const next = await service.exportTask!(handle!,controller.signal);
        if (controller.signal.aborted) return;
        accept(next,handle!); setError('');
        if (next.state === 'succeeded' && next.resultAvailable && service.exportResult) {
          const ready = await service.exportResult(handle!,controller.signal);
          if (!controller.signal.aborted) { accept(ready.task,handle!); setResult(ready); }
        } else if (!['cancelled','failed','succeeded'].includes(next.state)) timer = setTimeout(() => void poll(),200);
      } catch(e) { if (!controller.signal.aborted) { setError(String(e)); setMissing(String(e).includes('TASK_NOT_FOUND')); } }
    }
    void poll(); return () => { controller.abort(); clearTimeout(timer); };
  },[service,handle,lost,submitting,attempt,refresh]);
  async function submit(identity:ExportIdentity) {
    if (!service.startExport || submittingRef.current) return;
    submittingRef.current = true; setSubmitting(true); setError(''); setMissing(false); setResult(null);
    handleRef.current = identity; setHandle(identity); taskRef.current = null; setTask(null);
    try { sessionStorage.setItem(recoveryKey,JSON.stringify(identity)); } catch { /* Draft recovery warns independently. */ }
    try { accept(await service.startExport(identity),identity); }
    catch(e) { if (mounted.current) setError(String(e)); }
    finally { submittingRef.current = false; if (mounted.current) setSubmitting(false); }
  }
  const active = !!handle && !lost && !missing && (!task || !['succeeded','failed','cancelled'].includes(task.state));
  const canStart = available && planCurrent && plan?.stage === 'combined' && !!plan.summary && reviewedProfile(profile) && !!options && !active && !submitting;
  const current = !lost && currentExport(result,plan,planCurrent,profile,layout,options);
  const downloadable = current && result?.report.status === 'passed';
  return {draft,setDraft,profile,errors,layout,setLayout,recoveryError,options,available,canStart,current,downloadable,
    active,submitting,task,result,error,lost,
    start:() => { if (canStart) void submit(exportIdentity(plan!,profile!,layout,options!,crypto.randomUUID())); },
    cancel:async () => { if (!handle || !service.cancelExport) return;
      try { accept(await service.cancelExport(handle),handle); setAttempt(n => n + 1); }
      catch(e) { if (mounted.current) setError(String(e)); }
    }, check:() => setAttempt(n => n + 1), retry:handle && !task && !lost ? () => void submit(handle) : null,
    label:active ? 'Checking machine output' : current ? `Output ${result!.report.status}` : result ? 'Previous output stale' : available ? 'Profile & output needed' : 'Unavailable',
  };
}
