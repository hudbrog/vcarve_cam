import { z } from 'zod';
import { apiVersion, diagnosticSchema, planningLimitsSchema } from './wire';
import type { Capabilities, Validation } from './service';
import { sliceInfoSchema, stockSliceSchema } from './stock';

const integer = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
const fingerprint = z.string().regex(/^[a-f0-9]{64}$/);
export const stageSchema = z.enum(['endmill', 'combined']);
export type PlanningStage = z.infer<typeof stageSchema>;
export const planSummarySchema = z.strictObject({
  engineVersion: z.string().min(1), status: z.enum(['complete', 'empty', 'incomplete', 'inconclusive']),
  inputFingerprint: fingerprint, motionFingerprint: fingerprint, meaning: z.string(), limitations: z.array(z.string()),
  motionCount: integer, cuttingMotionCount: integer, previewMotionCount: integer, omittedMotionCount: integer,
  diagnostics: z.array(diagnosticSchema).max(100), omittedDiagnostics: integer,
  generationIssues: z.array(z.strictObject({ code: z.string(), message: z.string() })).max(100), omittedGenerationIssues: integer,
}).refine(value => value.previewMotionCount + value.omittedMotionCount === value.motionCount && value.cuttingMotionCount <= value.motionCount);
export const taskSchema = z.strictObject({
  apiVersion: z.literal(apiVersion), engineVersion: z.string().min(1), instanceId: z.string().regex(/^[a-f0-9]{32}$/),
  taskId: z.string().regex(/^[a-zA-Z0-9-]{1,128}$/), revision: integer, documentFingerprint: fingerprint,
  stage: stageSchema, sequence: integer.positive(), state: z.enum(['queued', 'running', 'cancelling', 'cancelled', 'succeeded', 'failed']),
  diagnostic: diagnosticSchema.nullable(), summary: planSummarySchema.nullable(), resultAvailable: z.boolean(),
}).refine(task => (task.state === 'succeeded') === (task.summary !== null)
  && (task.state === 'failed') === (task.diagnostic !== null)
  && (!task.resultAvailable || task.state === 'succeeded')
  && (!task.summary || task.summary.engineVersion === task.engineVersion), 'Inconsistent task result');
const position = z.strictObject({ x: z.number().finite(), y: z.number().finite(), z: z.number().finite() });
export const motionSchema = z.strictObject({
  id: integer, tool_id: z.string(), operation_id: z.string(), layer: integer,
  kind: z.enum(['rapid_x_y', 'rapid_retract', 'approach', 'plunge', 'ramp', 'cut']),
  start: position, end: position, feed_mm_min: z.number().finite().positive().nullable(),
});
export const planResultSchema = z.strictObject({
  task: taskSchema, coordinateSpace: z.literal('workpiece-mm-z-up'), motions: z.array(motionSchema).max(20_000),
  // Core permits 256 endmill layers, 32 combined depths, and a floor-check depth.
  stockSlices: z.array(sliceInfoSchema).max(289),
}).refine(result => result.task.state === 'succeeded' && result.task.resultAvailable
  && result.motions.length === result.task.summary?.previewMotionCount
  && new Set(result.stockSlices.map(s => s.id)).size === result.stockSlices.length
  && (result.task.stage === 'combined' || result.stockSlices.every(s => s.stage === 'endmill')), 'Inconsistent motion preview');
export const sliceResponseSchema = z.strictObject({ task: taskSchema, coordinateSpace: z.literal('workpiece-mm-z-up'), slice: stockSliceSchema })
  .refine(r => r.task.state === 'succeeded' && r.task.resultAvailable && (r.task.stage === 'combined' || r.slice.info.stage === 'endmill'));
export type SliceResponse = z.infer<typeof sliceResponseSchema>;
export type PlanningLimits = z.infer<typeof planningLimitsSchema>;
export type PlanTask = z.infer<typeof taskSchema>;
export type PlanResult = z.infer<typeof planResultSchema>;
export type Motion = z.infer<typeof motionSchema>;
export interface TaskIdentity {
  taskId: string; instanceId: string; engineVersion: string; revision: number;
  documentFingerprint: string; stage: PlanningStage;
}
export const taskIdentitySchema = z.strictObject({
  taskId: z.string().regex(/^[a-zA-Z0-9-]{1,128}$/), instanceId: z.string().regex(/^[a-f0-9]{32}$/),
  engineVersion: z.string().min(1), revision: integer, documentFingerprint: fingerprint, stage: stageSchema,
});
export const terminal = (task: PlanTask) => ['succeeded', 'failed', 'cancelled'].includes(task.state);
export function sameTask(task: TaskIdentity, identity: TaskIdentity): boolean {
  return task.taskId === identity.taskId && task.instanceId === identity.instanceId && task.engineVersion === identity.engineVersion
    && task.revision === identity.revision && task.documentFingerprint === identity.documentFingerprint && task.stage === identity.stage;
}
export function acceptTask(previous: PlanTask | null, next: PlanTask, identity: TaskIdentity): PlanTask {
  if (!sameTask(next, identity)) throw new Error('The service returned a different task identity. The response was discarded.');
  if (previous && next.sequence <= previous.sequence) return previous;
  if (previous && terminal(previous) && next.state !== previous.state) throw new Error('A finished task changed state. Reconnect to check the service.');
  return next;
}
export function planningInputMatches(task: PlanTask | null, validation: Validation | undefined, revision: number, stage: PlanningStage, capabilities: Capabilities): boolean {
  return !!task && task.instanceId === capabilities.planning?.instanceId
    && task.engineVersion === capabilities.engineVersion && task.stage === stage && task.revision === revision
    && validation?.revision === revision && validation.authoritative && validation.valid === true
    && validation.documentFingerprint === task.documentFingerprint;
}
export function currentPlan(task: PlanTask | null, validation: Validation | undefined, revision: number, stage: PlanningStage, capabilities: Capabilities): boolean {
  return task?.state === 'succeeded' && planningInputMatches(task,validation,revision,stage,capabilities);
}
