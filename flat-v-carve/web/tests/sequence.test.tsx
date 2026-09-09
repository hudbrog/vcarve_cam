import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import {
  exportResultSchema, planResultSchema, sequenceCapabilitiesSchema, sequenceDocumentSchema,
  sequenceEnvelopeSchema, sequenceApiVersion,
} from '../src/contracts/sequence';
import { createHttpSequenceService } from '../src/service/sequence';
import { readSequenceRecovery, SequenceWorkspace } from '../src/components/SequenceWorkspace';

const legacyJob = readFileSync(new URL('../../fixtures/m3/rectangle.json', import.meta.url), 'utf8');
const engineVersion = '0.7.7';
const fingerprint64 = 'a'.repeat(64);

function documentData(operations: { id: string; enabled?: boolean }[] = [{ id: 'flat-v-carve' }]) {
  return {
    migrated: true,
    job: { schema_version: 4, operations: operations.map(op => ({ id: op.id, enabled: op.enabled ?? true })) },
    operations: operations.map(op => ({
      id: op.id, name: `Op ${op.id}`, enabled: op.enabled ?? true, kind: 'flat_vcarve',
      toolIds: ['endmill', 'vbit'], depth: {},
    })),
    missingByOperation: Object.fromEntries(operations.map(op => [op.id, []])),
    setup: {
      stock: { thicknessMm: 8, xy: { minXmm: 0, minYmm: 0, widthMm: 100, lengthMm: 60 }, physicalXy: true },
      workZero: { xy: { kind: 'setup_origin' }, z: 'stock_bottom' },
      clearanceAboveStockMm: 5,
    },
    documentFingerprint: fingerprint64,
  };
}
function planData() {
  return {
    summary: {
      engineVersion, inputFingerprint: fingerprint64, executionFingerprint: fingerprint64,
      motionCount: 59, cuttingMotionCount: 12,
      operations: [{ operationId: 'flat-v-carve', generationStatus: 'complete', stageIds: ['flat-v-carve-vcarve-rough'], stockBeforeId: 'stock:initial', stockAfterId: 'stock:after:flat-v-carve' }],
      stages: [{ stageId: 'flat-v-carve-vcarve-rough', operationId: 'flat-v-carve', toolId: 'endmill', role: 'vcarve_rough', motionCount: 59 }],
      execution: [],
      basicChecks: { status: 'passed', findings: [], exportReady: true },
      preparationRequirements: [],
      diagnostics: [],
    },
    motions: { offset: 0, count: 59, total: 59, motions: [] },
    scope: { kind: 'allEnabled' },
  };
}
function exportData() {
  return {
    program: { filename: 'sequence.ngc', gcode: 'T1 M6\nM3 S10000\nM2\n' },
    report: {
      engineVersion, outputDecimalPlaces: 3, machineOffsetMm: [50, 30, -8], motionCount: 2, programSha256: fingerprint64,
      basicChecks: { status: 'passed', findings: [], exportReady: true }, diagnostics: [],
    },
    documentFingerprint: fingerprint64,
  };
}
function envelope(data: unknown, requestId: string, revision: number) {
  return { apiVersion: sequenceApiVersion, engineVersion, requestId, revision, data };
}

describe('ui-8 wire contracts', () => {
  it('parse the open, plan, export and capabilities projections', () => {
    expect(sequenceEnvelopeSchema.parse(envelope(documentData(), 'r1', 0))).toBeTruthy();
    expect(sequenceDocumentSchema.parse(documentData()).migrated).toBe(true);
    expect(planResultSchema.parse(planData()).summary.basicChecks.exportReady).toBe(true);
    expect(exportResultSchema.parse(exportData()).program.filename).toBe('sequence.ngc');
    expect(sequenceCapabilitiesSchema.parse({
      apiVersion: sequenceApiVersion, engineVersion, operationKinds: ['flat_vcarve'],
      planScopes: ['all_enabled', 'through_operation'],
      features: { legacyJobMigration: true }, limits: { pageMotions: 20000, jobBytes: 64000000 },
    }).features.legacyJobMigration).toBe(true);
  });
  it('reject foreign API versions and unknown fields', () => {
    expect(sequenceEnvelopeSchema.safeParse({ ...envelope({}, 'r', 0), apiVersion: 'ui-7' }).success).toBe(false);
    expect(sequenceDocumentSchema.safeParse({ ...documentData(), surprise: 1 }).success).toBe(false);
  });
});

describe('http sequence service', () => {
  function scripted() {
    const bodies: Record<string, unknown>[] = [];
    const fetcher: typeof fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith('/session')) return new Response(JSON.stringify({ sessionToken: 't'.repeat(64) }), { status: 200 });
      const body = JSON.parse(String(init?.body ?? '{}'));
      bodies.push(body);
      const command = body.command as Record<string, unknown>;
      const requestId = body.requestId as string;
      const revision = body.revision as number;
      if (command.operation === 'capabilities') {
        return Response.json(envelope({
          apiVersion: sequenceApiVersion, engineVersion, operationKinds: ['flat_vcarve'],
          planScopes: ['all_enabled', 'through_operation'], features: {}, limits: { pageMotions: 20000, jobBytes: 64000000 },
        }, requestId, revision));
      }
      if (command.operation === 'open') return Response.json(envelope(documentData(), requestId, revision));
      if (command.operation === 'plan') return Response.json(envelope(planData(), requestId, revision));
      if (command.operation === 'export') return Response.json(envelope(exportData(), requestId, revision));
      return Response.json({ error: { status: 422, code: 'UNKNOWN', message: 'unexpected' } }, { status: 422 });
    }) as typeof fetch;
    return { fetcher, bodies };
  }
  it('runs open, plan and export with echoed identities and increasing revisions', async () => {
    const { fetcher, bodies } = scripted();
    const service = createHttpSequenceService(fetcher);
    const document = await service.open(legacyJob);
    expect(document.operations).toHaveLength(1);
    const plan = await service.plan(document.job, { kind: 'allEnabled' });
    expect(plan.summary.motionCount).toBe(59);
    const exportResult = await service.export(document.job, { profile: 1 });
    expect(exportResult.program.filename).toBe('sequence.ngc');
    expect(bodies.map(body => body.revision)).toEqual([0, 1, 2]);
    expect(bodies.every(body => typeof body.requestId === 'string')).toBe(true);
  });
  it('surfaces engine diagnostics as errors', async () => {
    const fetcher: typeof fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      if (String(input).endsWith('/session')) return Response.json({ sessionToken: 't'.repeat(64) });
      const sent = JSON.parse(String(init?.body ?? '{}')) as { requestId: string };
      return Response.json({
        apiVersion: sequenceApiVersion, engineVersion, requestId: sent.requestId, revision: 0,
        diagnostic: { code: 'PROCESS_SPINDLE_STATE', severity: 'error', message: 'unresolved direction', stage: 'post' },
      });
    }) as typeof fetch;
    const service = createHttpSequenceService(fetcher);
    await expect(service.open(legacyJob)).rejects.toThrow('PROCESS_SPINDLE_STATE');
  });
});

describe('sequence workspace', () => {
  const service = {
    capabilities: async () => sequenceCapabilitiesSchema.parse({
      apiVersion: sequenceApiVersion, engineVersion, operationKinds: ['flat_vcarve'],
      planScopes: ['all_enabled', 'through_operation'], features: {}, limits: { pageMotions: 20000, jobBytes: 64000000 },
    }),
    open: async () => sequenceDocumentSchema.parse(documentData([{ id: 'carve-1' }, { id: 'carve-2', enabled: false }])),
    edit: async () => sequenceDocumentSchema.parse(documentData()),
    applyProfile: async () => sequenceDocumentSchema.parse(documentData()),
    plan: async () => planResultSchema.parse(planData()),
    export: async () => exportResultSchema.parse(exportData()),
  };
  it('renders the ordered operation list with per-operation controls', async () => {
    const document = sequenceDocumentSchema.parse(documentData([{ id: 'carve-1' }, { id: 'carve-2', enabled: false }]));
    const html = renderToStaticMarkup(<SequenceWorkspace service={service} onExit={() => {}} initialDocument={document} />);
    expect(html).toContain('Sequence');
    expect(html).toContain('SCHEMA-4 WORKSPACE');
    expect(html).toContain('carve-1');
    expect(html).toContain('carve-2');
    expect(html).toContain('disabled');
    expect(html).toContain('Enable');
    expect(html).toContain('Open job…');
    expect(html).toContain('Apply legacy machine profile');
  });
  it('round-trips recovery documents under an independent envelope', () => {
    const storage = new Map<string, string>();
    const job = { schema_version: 4, marker: 'opaque' };
    storage.set('flat-v-carve:sequence:v1', JSON.stringify({ version: 1, job, savedAt: 'now' }));
    expect(readSequenceRecovery({ getItem: key => storage.get(key) ?? null })).toEqual(job);
    storage.set('flat-v-carve:sequence:v1', JSON.stringify({ version: 2, job }));
    expect(() => readSequenceRecovery({ getItem: key => storage.get(key) ?? null })).toThrow('Unsupported sequence recovery.');
  });
});
