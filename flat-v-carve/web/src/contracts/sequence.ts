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
  // Appends an incomplete face/profile operation bound to an explicit tool;
  // every machining value stays unset until edited through UpdateSettings.
  z.strictObject({ edit: z.literal('add'), id: z.string().min(1), name: z.string().min(1), kind: z.enum(['face', 'profile']), toolId: z.string().min(1) }),
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

// Resolved operation outputs: face planes and tab placements as generated.
export const namedOutputSchema = z.strictObject({
  kind: z.string(),
  zMm: z.number().nullable(),
  covered: z.strictObject({
    min_x_mm: z.number(), min_y_mm: z.number(), width_mm: z.number(), length_mm: z.number(),
  }).nullable(),
  tabPlacements: z.array(z.strictObject({
    contourId: z.string(),
    bridgeStartMm: z.number(), bridgeEndMm: z.number(),
    restrictedStartMm: z.number(), restrictedEndMm: z.number(), topZMm: z.number(),
  })),
});
export type NamedOutputInfo = z.infer<typeof namedOutputSchema>;

export const planSummarySchema = z.strictObject({
  engineVersion: z.string(), inputFingerprint: fingerprint, executionFingerprint: fingerprint,
  motionCount: integer, cuttingMotionCount: integer,
  operations: z.array(z.strictObject({
    operationId: z.string(),
    generationStatus: z.enum(['complete', 'empty', 'incomplete', 'inconclusive']),
    stageIds: z.array(z.string()), stockBeforeId: z.string(), stockAfterId: z.string(),
    namedOutputs: z.array(namedOutputSchema),
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

// One page of the complete ordered motion stream (the timeline display pages
// until total is reached; a single page is not a complete simulation).
export const motionPageResultSchema = z.strictObject({
  motions: motionPageSchema, scope: planScopeSchema,
});
export type MotionPageResult = z.infer<typeof motionPageResultSchema>;

// Contour catalogue projection (plan section 7.1) for explicit per-contour
// selection: stable IDs, roles with lineage, and the suggested side.
export const contourInfoSchema = z.strictObject({
  id: z.string(),
  componentId: z.string(),
  closed: z.boolean(),
  role: z.enum(['outer', 'hole', 'open']),
  parentContourId: z.string().nullable(),
  perimeterMm: z.number().finite().positive(),
  suggestedSide: z.enum(['inside', 'outside', 'on']),
  bounds: z.strictObject({
    minXmm: z.number(), minYmm: z.number(), maxXmm: z.number(), maxYmm: z.number(),
  }),
});
export const contourCatalogueResultSchema = z.strictObject({
  contours: z.array(contourInfoSchema),
});
export type ContourInfo = z.infer<typeof contourInfoSchema>;
export type ContourCatalogueResult = z.infer<typeof contourCatalogueResultSchema>;

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

// Operation settings are opaque to the UI (strict engine parsing); only the
// face editor composes them, field by field, through UpdateSettings.
export type FaceSettingsInput = {
  area: { kind: 'entireStock' } | { kind: 'rectangle'; rect: { min_x_mm: number; min_y_mm: number; width_mm: number; length_mm: number } };
  margins: { min_x_mm?: number | null; max_x_mm?: number | null; min_y_mm?: number | null; max_y_mm?: number | null };
  entry_overrun_mm?: number | null;
  exit_overrun_mm?: number | null;
  top: unknown;
  bottom: unknown;
  stepdown_mm?: number | null;
  stepover_mm?: number | null;
  pass_angle_deg?: number | null;
  pattern: 'zigzag' | 'one_way';
  assignment: unknown;
};

export interface SequenceService {
  capabilities(signal?: AbortSignal): Promise<SequenceCapabilities>;
  open(json: string, signal?: AbortSignal): Promise<SequenceDocument>;
  edit(job: unknown, edits: OperationEdit[], signal?: AbortSignal): Promise<SequenceDocument>;
  applyProfile(job: unknown, profile: unknown, signal?: AbortSignal): Promise<SequenceDocument>;
  updateSettings(job: unknown, operationId: string, settings: unknown, signal?: AbortSignal): Promise<SequenceDocument>;
  plan(job: unknown, scope: PlanScope, signal?: AbortSignal): Promise<PlanResult>;
  planMotions(job: unknown, scope: PlanScope, offset: number, signal?: AbortSignal): Promise<MotionPageResult>;
  contours(job: unknown, signal?: AbortSignal): Promise<ContourCatalogueResult>;
  export(job: unknown, profile: unknown, signal?: AbortSignal): Promise<ExportResult>;
}
