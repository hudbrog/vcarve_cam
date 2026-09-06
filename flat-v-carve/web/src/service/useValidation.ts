import { useEffect, useState } from 'react';
import type { Job } from '../contracts/job';
import type { CamService, Validation } from '../contracts/service';

type Check = { revision: number; refresh: number; result?: Validation; error?: string };
export function useValidation(service: CamService, job: Job | null, revision: number, refresh: number, enabled: boolean) {
  const [check, setCheck] = useState<Check | null>(null);
  useEffect(() => {
    if (!enabled || !job) return;
    const controller = new AbortController();
    setCheck(null);
    const timer = setTimeout(() => {
      service.validateDraft(job, revision, controller.signal).then(result => {
        if (!controller.signal.aborted) {
          if (result.revision !== revision || !result.authoritative) throw new Error('The engine returned an incompatible validation receipt.');
          setCheck({ revision, refresh, result });
        }
      }).catch(error => { if (!controller.signal.aborted) setCheck({ revision, refresh, error: String(error) }); });
    }, 400);
    return () => { clearTimeout(timer); controller.abort(); };
  }, [service, job, revision, refresh, enabled]);
  // Edits invalidate the rendered evidence immediately, before effects/requests run.
  const current = enabled && job && check?.revision === revision && check.refresh === refresh ? check : null;
  const result = current?.result;
  const headline = !enabled ? 'Machining settings are not validated'
    : !job ? 'Finish incomplete fields to validate'
    : current?.error ? 'Local validation unavailable'
    : !result ? 'Checking editable job with Rust…'
    : !result.valid ? 'Rust rejected supplied settings'
    : result.missingMachiningFields?.length ? `Editable job accepted · ${result.missingMachiningFields.length} settings unset`
    : 'Editable job accepted';
  return { result, error: current?.error, headline, pending: enabled && !!job && !current };
}
