import { profileSchema, type MachineProfile } from './machineProfile';
export { profileSchema, type MachineProfile } from './machineProfile';
import { z } from 'zod';
import { sameTask, taskSchema, taskIdentitySchema, type PlanTask } from './planning';
import { stockVerificationSchema, verificationReportSchema } from './verification';
import { sameOptions, verificationOptionsSchema, type VerificationOptions } from './verificationOptions';

const count = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
const fingerprint = z.string().regex(/^[a-f0-9]{64}$/);
const status = z.enum(['passed', 'failed', 'inconclusive']);
const coordinate = z.number().finite().min(-1_000_000).max(1_000_000);
export const layoutSchema = z.enum(['combined','per_tool']);
export type ProgramLayout = z.infer<typeof layoutSchema>;
const filename = z.enum(['combined.ngc','endmill.ngc','vbit.ngc']);
export const exportReportSchema = z.strictObject({
  artifact_kind: z.literal('linuxcnc_export_report'), schema_version: z.literal(1), engine_version: z.string(), status,
  profile: profileSchema, profile_fingerprint: fingerprint, layout: layoutSchema, machine_z_offset_mm: coordinate,
  output_decimal_places: z.number().int().min(0).max(9),
  plan_verification: verificationReportSchema, emitted_verification: stockVerificationSchema.nullable(),
  emitted_motion_fingerprint: fingerprint.nullable(),
  programs: z.array(z.strictObject({ filename, sha256: fingerprint, motion_count: count, clearance_link_count: count,
    contract_positioning_blocks: count, tool_changes: count, prerequisites: z.array(z.string()) })).max(2),
  diagnostics: z.array(z.strictObject({ code: z.string(), severity: z.enum(['warning','error']), stage: z.string(),
    message: z.string(), source_id: z.string().optional() })), limitations: z.array(z.string()),
}).refine(r => r.output_decimal_places >= r.profile.decimal_places && r.plan_verification.rounded === null && r.plan_verification.engine_version === r.engine_version
  && (r.status !== 'passed' || (r.plan_verification.status === 'passed' && r.emitted_verification?.status === 'passed'
    && r.emitted_motion_fingerprint !== null && r.programs.length > 0 && !r.diagnostics.some(d => d.severity === 'error')))
  && new Set(r.programs.map(p => p.filename)).size === r.programs.length
  && r.programs.every(p => r.layout === 'combined' ? p.filename === 'combined.ngc' : p.filename !== 'combined.ngc'), 'Inconsistent export outcome');
const sourceSchema = z.strictObject({ planTaskId: taskIdentitySchema.shape.taskId, inputFingerprint: fingerprint,
  motionFingerprint: fingerprint, profile: profileSchema, layout: layoutSchema, options: verificationOptionsSchema });
export const exportIdentitySchema = taskIdentitySchema.extend({ stage: z.literal('combined'), export: sourceSchema });
export const exportTaskSchema = z.strictObject({ ...taskSchema.shape, stage: z.literal('combined'), export: sourceSchema,
  summary: z.strictObject({ engineVersion: z.string(), status, profileFingerprint: fingerprint, reportFingerprint: fingerprint,
    originalStatus: status, emittedStatus: status.nullable() }).nullable(),
}).refine(t => (t.state === 'succeeded') === (t.summary !== null) && (t.state === 'failed') === (t.diagnostic !== null)
  && (!t.resultAvailable || t.state === 'succeeded') && (!t.summary || t.summary.engineVersion === t.engineVersion));
export const exportResultSchema = z.strictObject({ task: exportTaskSchema, report: exportReportSchema,
  reportJson: z.string().max(16_000_000), programs: z.array(z.strictObject({ filename, gcode: z.string().max(8_000_000) })).max(2),
}).refine(r => r.task.state === 'succeeded' && r.task.resultAvailable && r.task.engineVersion === r.report.engine_version
  && r.task.summary?.status === r.report.status && r.task.summary.profileFingerprint === r.report.profile_fingerprint
  && r.task.summary.originalStatus === r.report.plan_verification.status && r.task.summary.emittedStatus === (r.report.emitted_verification?.status ?? null)
  && sameProfile(r.task.export.profile, r.report.profile) && r.task.export.layout === r.report.layout
  && sameOptions({ ...r.task.export.options, decimal_places: null }, r.report.plan_verification.options)
  && (r.report.status === 'passed' ? r.programs.length === r.report.programs.length
    && r.programs.every((p,i) => p.filename === r.report.programs[i].filename) : r.programs.length === 0), 'Export differs from its task or report');
export type ExportIdentity = z.infer<typeof exportIdentitySchema>;
export type ExportTask = z.infer<typeof exportTaskSchema>;
export type ExportResult = z.infer<typeof exportResultSchema>;
export function sameProfile(a: MachineProfile, b: MachineProfile): boolean {
  return JSON.stringify(profileSchema.parse(a)) === JSON.stringify(profileSchema.parse(b));
}
export function sameExport(a: ExportIdentity, b: ExportIdentity): boolean {
  return sameTask(a,b) && a.export.planTaskId === b.export.planTaskId && a.export.inputFingerprint === b.export.inputFingerprint
    && a.export.motionFingerprint === b.export.motionFingerprint && a.export.layout === b.export.layout
    && sameProfile(a.export.profile,b.export.profile) && sameOptions(a.export.options,b.export.options);
}
export function acceptExport(previous: ExportTask | null, next: ExportTask, expected: ExportIdentity): ExportTask {
  if (!sameExport(next,expected)) throw new Error('Export belongs to a different plan, profile, layout, or verification settings. Response discarded.');
  if (previous && next.sequence <= previous.sequence) return previous;
  if (previous && ['succeeded','failed','cancelled'].includes(previous.state) && next.state !== previous.state)
    throw new Error('A finished export changed state. Reconnect to check the service.');
  return next;
}
export function exportIdentity(plan: PlanTask, profile: MachineProfile, layout: ProgramLayout, options: VerificationOptions, id: string): ExportIdentity {
  return { taskId:id, instanceId:plan.instanceId, engineVersion:plan.engineVersion, revision:plan.revision,
    documentFingerprint:plan.documentFingerprint, stage:'combined', export: { planTaskId:plan.taskId,
      inputFingerprint:plan.summary!.inputFingerprint, motionFingerprint:plan.summary!.motionFingerprint,
      profile:structuredClone(profile), layout, options:structuredClone(options) } };
}
export function currentExport(result: ExportResult | null, plan: PlanTask | null, planCurrent: boolean,
  profile: MachineProfile | null, layout: ProgramLayout, options: VerificationOptions | null): boolean {
  return !!result && !!plan && planCurrent && plan.stage === 'combined' && !!plan.summary && !!profile && !!options
    && sameExport(result.task,exportIdentity(plan,profile,layout,options,result.task.taskId));
}
export async function checkExportBytes(result: ExportResult): Promise<void> {
  const bytes = (s:string) => new TextEncoder().encode(s);
  const hash = async (s:string) => Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes(s))), b => b.toString(16).padStart(2,'0')).join('');
  // A separately parsed report cannot claim a different file set from the bytes
  // offered for download. Compare schemas in their canonical field order.
  const raw = exportReportSchema.parse(JSON.parse(result.reportJson));
  const authenticatedPlan = await hash(JSON.stringify([result.task.export.inputFingerprint,result.task.export.motionFingerprint]));
  if (JSON.stringify(raw) !== JSON.stringify(result.report) || bytes(result.reportJson).length > 16_000_000
    || authenticatedPlan !== result.report.plan_verification.authenticated_plan_fingerprint
    || await hash(result.reportJson) !== result.task.summary!.reportFingerprint
    || result.programs.reduce((n,p) => n + bytes(p.gcode).length,0) > 8_000_000)
    throw new Error('Export report bytes or size differ from the accepted result. Downloads withheld.');
  for (const [i,program] of result.programs.entries()) {
    if (await hash(program.gcode) !== result.report.programs[i].sha256)
      throw new Error('Program bytes differ from the checked SHA-256. Downloads withheld.');
  }
}
