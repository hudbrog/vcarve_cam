import { z } from 'zod';
import { jobSchema, pointSchema } from './job';

export const apiVersion = 'ui-2';
const integer = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
export const sessionSchema = z.strictObject({
  apiVersion: z.literal(apiVersion), engineVersion: z.string().min(1), sessionToken: z.string().regex(/^[a-f0-9]{64}$/),
});
export const planningLimitsSchema = z.strictObject({
  instanceId: z.string().regex(/^[a-f0-9]{32}$/), concurrentPlans: integer.positive(),
  maxPending: integer.positive(), maxTasks: integer.positive(), retainedResults: integer.positive(),
  timeoutSeconds: integer.positive(), previewMotions: integer.positive(), artifactBytes: integer.positive(),
});
export const capabilitiesSchema = z.strictObject({
  apiVersion: z.literal(apiVersion), engineVersion: z.string().min(1), mode: z.literal('live'),
  importArtwork: z.boolean(), openJob: z.boolean(), validateDraft: z.boolean(),
  planningStages: z.array(z.enum(['endmill', 'combined'])), verificationScopes: z.array(z.string()), exportFormats: z.array(z.string()),
  planning: planningLimitsSchema.optional(),
  limits: z.strictObject({ svgBytes: integer.positive(), jobBytes: integer.positive(), requestBytes: integer.positive(), concurrentInspections: integer.positive() }),
}).refine(value => value.planningStages.length === 0 || value.planning !== undefined, 'Planning limits are required');
export const diagnosticSchema = z.strictObject({
  code: z.string(), severity: z.enum(['info', 'warning', 'error']), message: z.string(),
  stage: z.string().optional(), sourceId: z.string().optional(), fieldPath: z.string().optional(),
});
export const displaySchema = z.strictObject({
  coordinateSpace: z.literal('source-page-mm-y-up'), widthMm: z.number().finite().positive(), heightMm: z.number().finite().positive(),
  engineVersion: z.string().min(1), geometryToleranceMm: z.number().finite().positive(), description: z.string(),
  components: z.array(z.strictObject({ id: z.string(), label: z.string(), rings: z.array(z.strictObject({ hole: z.boolean(), points: z.array(pointSchema) })) })),
});
const fingerprint = z.string().regex(/^[a-f0-9]{64}$/);
export const openedSchema = z.strictObject({
  job: jobSchema, display: displaySchema, diagnostics: z.array(diagnosticSchema),
  missingMachiningFields: z.array(z.string()), documentFingerprint: fingerprint,
});
export const validationSchema = z.strictObject({
  valid: z.boolean(), scope: z.literal('editable-job-and-svg'), authoritative: z.literal(true),
  diagnostics: z.array(diagnosticSchema), missingMachiningFields: z.array(z.string()), documentFingerprint: fingerprint.nullable(),
}).refine(value => value.valid ? value.documentFingerprint !== null && !value.diagnostics.some(d => d.severity === 'error')
  : value.documentFingerprint === null && value.diagnostics.some(d => d.severity === 'error'), 'Inconsistent validation result');
export const envelopeSchema = z.strictObject({
  apiVersion: z.literal(apiVersion), engineVersion: z.string().min(1), requestId: z.string(), revision: integer,
  data: z.unknown().optional(), diagnostic: diagnosticSchema.optional(),
});
export const errorSchema = z.strictObject({
  apiVersion: z.literal(apiVersion), engineVersion: z.string(),
  error: z.strictObject({ code: z.string(), message: z.string() }),
});
