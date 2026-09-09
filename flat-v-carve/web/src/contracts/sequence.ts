import { z } from 'zod';

// ui-8 sequence wire contracts. The canonical job document itself (schema 4)
// stays snake_case like every persisted artifact; the envelope and these
// projections are camelCase like the rest of the UI wire.
export const sequenceApiVersion = 'ui-8';
const integer = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
const fingerprint = z.string().regex(/^[a-f0-9]{64}$/);

export const sequenceDiagnosticSchema = z.strictObject({
  code: z.string(), severity: z.enum(['info', 'warning', 'error']), message: z.string(),
  stage: z.string().optional(), sourceId: z.string().optional(),
});
export const sequenceEnvelopeSchema = z.strictObject({
  apiVersion: z.literal(sequenceApiVersion), engineVersion: z.string().min(1),
  requestId: z.string(), revision: integer,
  data: z.unknown().optional(), diagnostic: sequenceDiagnosticSchema.optional(),
});

export const planScopeSchema = z.discriminatedUnion('kind', [
  z.strictObject({ kind: z.literal('allEnabled') }),
  z.strictObject({ kind: z.literal('throughOperation'), operationId: z.string().min(1) }),
]);
export type PlanScope = z.infer<typeof planScopeSchema>;

export const operationEditSchema = z.discriminatedUnion('edit', [
  z.strictObject({ edit: z.literal('rename'), id: z.string(), name: z.string().min(1) }),
  z.strictObject({ edit: z.literal('setEnabled'), id: z.string(), enabled: z.boolean() }),
  z.strictObject({ edit: z.literal('move'), id: z.string(), toIndex: integer }),
  z.strictObject({ edit: z.literal('delete'), id: z.string() }),
  z.strictObject({ edit: z.literal('duplicate'), id: z.string(), newId: z.string().min(1) }),
]);
export type OperationEdit = z.infer<typeof operationEditSchema>;

// The schema-4 job document is opaque to the UI (edited only through engine
// commands) but its identity and operation summaries are projected.
export const sequenceOperationSchema = z.strictObject({
  id: z.string(), name: z.string(), enabled: z.boolean(),
  kind: z.enum(['flat_vcarve', 'face', 'profile', 'drag_knife']),
  toolIds: z.array(z.string()), depth: z.unknown(),
});
export const missingEntrySchema = z.strictObject({
  fieldPath: z.string(), message: z.string(), toolId: z.string().nullable(),
});
export const sequenceDocumentSchema = z.strictObject({
  job: z.unknown(), migrated: z.boolean(),
  operations: z.array(sequenceOperationSchema),
  missingByOperation: z.record(z.string(), z.array(missingEntrySchema)),
  setup: z.strictObject({
    stock: z.strictObject({
      thicknessMm: z.number().positive().nullable(),
      xy: z.strictObject({
        minXmm: z.number(), minYmm: z.number(), widthMm: z.number().positive(), lengthMm: z.number().positive(),
      }).nullable(),
      physicalXy: z.boolean(),
    }),
    workZero: z.strictObject({ xy: z.unknown(), z: z.enum(['stock_top', 'stock_bottom']) }),
    clearanceAboveStockMm: z.number().positive().nullable(),
  }),
  documentFingerprint: fingerprint,
});
export type SequenceDocument = z.infer<typeof sequenceDocumentSchema>;

export const plannedMotionSchema = z.strictObject({
  id: integer, operationId: z.string(), stageId: z.string(), toolId: z.string(),
  contourId: z.string().nullable().optional(), passId: integer, layer: integer,
  interpolation: z.enum(['rapid', 'linear_feed']),
  purpose: z.enum(['clearance', 'approach', 'entry', 'rough', 'finish', 'tab_transition', 'lead_in', 'lead_out', 'knife_cut', 'knife_align', 'knife_swivel']),
  effect: z.enum(['none', 'milling_sweep', 'knife_trace']),
  start: z.strictObject({ x: z.number(), y: z.number(), z: z.number() }),
  end: z.strictObject({ x: z.number(), y: z.number(), z: z.number() }),
  feedMmMin: z.number().positive().nullable().optional(),
});
export type PlannedMotion = z.infer<typeof plannedMotionSchema>;

export const planSummarySchema = z.strictObject({
  engineVersion: z.string(), inputFingerprint: fingerprint, executionFingerprint: fingerprint,
  motionCount: integer, cuttingMotionCount: integer,
  operations: z.array(z.strictObject({
    operationId: z.string(),
    generationStatus: z.enum(['complete', 'empty', 'incomplete', 'inconclusive']),
    stageIds: z.array(z.string()), stockBeforeId: z.string(), stockAfterId: z.string(),
  })),
  stages: z.array(z.strictObject({
    stageId: z.string(), operationId: z.string(), toolId: z.string(),
    role: z.enum(['face', 'profile_rough', 'profile_finish', 'vcarve_rough', 'vcarve_finish', 'knife']),
    motionCount: integer,
  })),
  execution: z.array(z.unknown()),
  basicChecks: z.strictObject({
    status: z.enum(['passed', 'failed', 'inconclusive']),
    findings: z.array(z.strictObject({
      code: z.string(), message: z.string(),
      operationId: z.string().nullable().optional(), stageId: z.string().nullable().optional(),
    })),
    exportReady: z.boolean(),
  }),
  preparationRequirements: z.array(z.strictObject({
    code: z.string(), message: z.string(),
    operationId: z.string().nullable().optional(), stageId: z.string().nullable().optional(),
  })),
  diagnostics: z.array(z.strictObject({
    code: z.string(), message: z.string(),
    operationId: z.string().nullable().optional(), stageId: z.string().nullable().optional(),
  })),
});
export type PlanSummary = z.infer<typeof planSummarySchema>;

export const motionPageSchema = z.strictObject({
  offset: integer, count: integer, total: integer, motions: z.array(plannedMotionSchema),
});
export type MotionPage = z.infer<typeof motionPageSchema>;

export const planResultSchema = z.strictObject({
  summary: planSummarySchema, motions: motionPageSchema, scope: planScopeSchema,
});
export type PlanResult = z.infer<typeof planResultSchema>;

export const exportResultSchema = z.strictObject({
  program: z.strictObject({ filename: z.string(), gcode: z.string() }),
  report: z.strictObject({
    engineVersion: z.string(), outputDecimalPlaces: integer,
    machineOffsetMm: z.tuple([z.number(), z.number(), z.number()]),
    motionCount: integer, programSha256: z.string(),
    basicChecks: z.strictObject({
      status: z.enum(['passed', 'failed', 'inconclusive']),
      findings: z.array(z.strictObject({ code: z.string(), message: z.string() })),
      exportReady: z.boolean(),
    }),
    diagnostics: z.array(z.string()),
  }),
  documentFingerprint: fingerprint,
});
export type ExportResult = z.infer<typeof exportResultSchema>;

export const sequenceCapabilitiesSchema = z.strictObject({
  apiVersion: z.literal(sequenceApiVersion), engineVersion: z.string(),
  operationKinds: z.array(z.string()), planScopes: z.array(z.string()),
  features: z.record(z.string(), z.boolean()),
  limits: z.strictObject({ pageMotions: integer, jobBytes: integer }),
});
export type SequenceCapabilities = z.infer<typeof sequenceCapabilitiesSchema>;

export interface SequenceService {
  capabilities(signal?: AbortSignal): Promise<SequenceCapabilities>;
  open(json: string, signal?: AbortSignal): Promise<SequenceDocument>;
  edit(job: unknown, edits: OperationEdit[], signal?: AbortSignal): Promise<SequenceDocument>;
  applyProfile(job: unknown, profile: unknown, signal?: AbortSignal): Promise<SequenceDocument>;
  plan(job: unknown, scope: PlanScope, signal?: AbortSignal): Promise<PlanResult>;
  export(job: unknown, profile: unknown, signal?: AbortSignal): Promise<ExportResult>;
}
