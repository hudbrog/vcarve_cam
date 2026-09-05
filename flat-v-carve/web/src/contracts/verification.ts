import { z } from 'zod';
import { pointSchema } from './job';
import { boundsSchema } from './stock';
import { taskSchema, taskIdentitySchema, sameTask, type PlanTask, type TaskIdentity } from './planning';
import { sameOptions, verificationOptionsSchema, type VerificationOptions } from './verificationOptions';

const count = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
const scalar = z.number().finite().nonnegative();
const fingerprint = z.string().regex(/^[a-f0-9]{64}$/);
const status = z.enum(['passed', 'failed', 'inconclusive']);
export const intervalSchema = z.strictObject({ lower: scalar, upper: scalar }).refine(i => i.lower <= i.upper);
export const findingSchema = z.strictObject({ code: z.string(), status, message: z.string(), location: pointSchema,
  cell: boundsSchema.nullable(), motion_id: count.nullable(), measured_mm: intervalSchema.nullable(), limit_mm: scalar.nullable(),
});
const stockVerificationSchema = z.strictObject({
  status, domain: boundsSchema, verification_tolerance_mm: scalar, floor_ridge_limit_mm: scalar, detail_residual_limit_mm: scalar,
  arithmetic_reserve_mm: scalar, source_geometry_depth_error_mm: scalar,
  checked_motion_count: count, analytically_clear_motion_count: count, evaluated_cells: count, terminal_cells: count,
  unresolved_cells: count, maximum_refinement_depth: count, reachability_cells: count, maximum_error_uncertainty_mm: scalar,
  bounds: z.strictObject({ overcut_mm: intervalSchema, floor_ridge_mm: intervalSchema, unreachable_detail_mm: intervalSchema,
    other_reachable_residual_mm: intervalSchema, total_residual_mm: intervalSchema, residual_volume_mm3: intervalSchema, overcut_volume_mm3: intervalSchema }),
  depth_bands: z.array(z.strictObject({ from_depth_mm: scalar, to_depth_mm: scalar,
    nominal_area_mm2: intervalSchema, removed_area_mm2: intervalSchema, residual_area_mm2: intervalSchema, overcut_area_mm2: intervalSchema,
  }).refine(b => b.from_depth_mm <= b.to_depth_mm)).max(4096),
  // verify_plan may append one authenticated-execution finding after the budget.
  findings: z.array(findingSchema).max(4097), omitted_findings: count, limitations: z.array(z.string()),
});
export const verificationReportSchema = z.strictObject({
  artifact_kind: z.literal('verification_report'), schema_version: z.literal(1), engine_version: z.string(), status,
  input_fingerprint: fingerprint, motion_fingerprint: fingerprint, verification_fingerprint: fingerprint,
  authenticated_plan_fingerprint: fingerprint, options: verificationOptionsSchema,
  original: stockVerificationSchema,
  rounded: z.strictObject({ decimal_places: z.number().int().min(0).max(9), coordinate_quantum_mm: scalar,
    maximum_coordinate_change_mm: scalar, motion_fingerprint: fingerprint, verification: stockVerificationSchema }).nullable(),
}).refine(r => (r.options.decimal_places === null) === (r.rounded === null)
  && (!r.rounded || r.rounded.decimal_places === r.options.decimal_places)
  && r.status === ([r.original.status, r.rounded?.verification.status].includes('failed') ? 'failed'
    : [r.original.status, r.rounded?.verification.status].includes('inconclusive') ? 'inconclusive' : 'passed'), 'Inconsistent verification scope or outcome');
const sourceSchema = z.strictObject({ planTaskId: taskIdentitySchema.shape.taskId, inputFingerprint: fingerprint,
  motionFingerprint: fingerprint, options: verificationOptionsSchema });
export const verificationIdentitySchema = taskIdentitySchema.extend({ stage: z.literal('combined'), verification: sourceSchema });
const summarySchema = z.strictObject({ engineVersion: z.string(), status, verificationFingerprint: fingerprint,
  originalStatus: status, roundedStatus: status.nullable() });
export const verificationTaskSchema = z.strictObject({ ...taskSchema.shape, stage: z.literal('combined'),
  verification: sourceSchema, summary: summarySchema.nullable(),
}).refine(t => (t.state === 'succeeded') === (t.summary !== null)
  && (t.state === 'failed') === (t.diagnostic !== null) && (!t.resultAvailable || t.state === 'succeeded')
  && (!t.summary || t.summary.engineVersion === t.engineVersion), 'Inconsistent verification task');
export const verificationResultSchema = z.strictObject({ task: verificationTaskSchema,
  coordinateSpace: z.literal('workpiece-mm-z-up'), report: verificationReportSchema,
}).refine(r => r.task.state === 'succeeded' && r.task.resultAvailable
  && r.task.engineVersion === r.report.engine_version && r.task.summary?.status === r.report.status
  && r.task.summary.verificationFingerprint === r.report.verification_fingerprint
  && r.task.summary.originalStatus === r.report.original.status
  && r.task.summary.roundedStatus === (r.report.rounded?.verification.status ?? null)
  && sameOptions(r.task.verification.options, r.report.options), 'Report differs from the accepted verification');
export type VerificationIdentity = z.infer<typeof verificationIdentitySchema>;
export type VerificationTask = z.infer<typeof verificationTaskSchema>;
export type VerificationResult = z.infer<typeof verificationResultSchema>;
export type VerificationReport = z.infer<typeof verificationReportSchema>;
export type Finding = z.infer<typeof findingSchema>;
export type StockVerification = z.infer<typeof stockVerificationSchema>;
export function sameVerification(a: VerificationIdentity, b: VerificationIdentity): boolean {
  return sameTask(a, b) && a.verification.planTaskId === b.verification.planTaskId
    && a.verification.inputFingerprint === b.verification.inputFingerprint && a.verification.motionFingerprint === b.verification.motionFingerprint
    && sameOptions(a.verification.options, b.verification.options);
}
export function acceptVerification(previous: VerificationTask | null, next: VerificationTask, expected: VerificationIdentity) {
  if (!sameVerification(next, expected)) throw new Error('Verification belongs to a different task, plan, or settings. The response was discarded.');
  if (previous && next.sequence <= previous.sequence) return previous;
  if (previous && ['succeeded', 'failed', 'cancelled'].includes(previous.state) && next.state !== previous.state)
    throw new Error('A finished verification changed state. Reconnect to check the service.');
  return next;
}
export function currentVerification(result: VerificationResult | null, plan: PlanTask | null, planCurrent: boolean, options: VerificationOptions | null): boolean {
  if (!result || !plan || !planCurrent || !options) return false;
  const identity = result.task;
  return identity.instanceId === plan.instanceId && identity.engineVersion === plan.engineVersion && identity.revision === plan.revision
    && identity.documentFingerprint === plan.documentFingerprint && plan.stage === 'combined'
    && identity.verification.planTaskId === plan.taskId && identity.verification.inputFingerprint === plan.summary?.inputFingerprint
    && identity.verification.motionFingerprint === plan.summary?.motionFingerprint && sameOptions(identity.verification.options, options);
}
export function verificationIdentity(plan: PlanTask, options: VerificationOptions, id: string): VerificationIdentity {
  const identity: TaskIdentity = { taskId: id, instanceId: plan.instanceId, engineVersion: plan.engineVersion,
    revision: plan.revision, documentFingerprint: plan.documentFingerprint, stage: 'combined' };
  return { ...identity, stage: 'combined', verification: { planTaskId: plan.taskId, inputFingerprint: plan.summary!.inputFingerprint,
    motionFingerprint: plan.summary!.motionFingerprint, options: structuredClone(options) } };
}
