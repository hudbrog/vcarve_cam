import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import {
  contourCatalogueResultSchema, exportResultSchema, planResultSchema, sequenceCapabilitiesSchema,
  sequenceDocumentSchema, sequenceEnvelopeSchema, sequenceApiVersion, motionPageResultSchema,
} from '../src/contracts/sequence';
import { createHttpSequenceService } from '../src/service/sequence';
import { FaceSettingsEditor } from '../src/components/FaceSettingsEditor';
import { ProfileSettingsEditor } from '../src/components/ProfileSettingsEditor';
import { SequenceTimeline } from '../src/components/SequenceTimeline';
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
      operations: [{
        operationId: 'flat-v-carve', generationStatus: 'complete', stageIds: ['flat-v-carve-vcarve-rough'],
        stockBeforeId: 'stock:initial', stockAfterId: 'stock:after:flat-v-carve',
        namedOutputs: [{
          kind: 'profile_tabs', zMm: null, covered: null,
          tabPlacements: [{
            contourId: 'pocket-0-outer', bridgeStartMm: 14.5, bridgeEndMm: 19.5,
            restrictedStartMm: 12.49, restrictedEndMm: 21.51, topZMm: -6,
            footprintMm: [[14.5, 5], [19.5, 5], [19.5, 1], [14.5, 1]],
          }],
        }],
      }],
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
function contoursData() {
  return {
    contours: [{
      id: 'pocket-0-outer', componentId: 'pocket::0', closed: true, role: 'outer',
      parentContourId: null, perimeterMm: 100.5, suggestedSide: 'outside',
      sourceFingerprint: 'fp-1',
      bounds: { minXmm: 5, minYmm: 5, maxXmm: 35, maxYmm: 25 },
    }],
  };
}
const timelineMotions = Array.from({ length: 5 }, (_, index) => ({
  id: index, operationId: 'flat-v-carve', stageId: 's', toolId: 'endmill', passId: 0, layer: 0,
  interpolation: 'linear_feed', purpose: 'rough', effect: 'milling_sweep',
  start: { x: 0, y: 0, z: -1 }, end: { x: 1, y: 0, z: -1 }, feedMmMin: 300,
}));
function motionsData(offset: number) {
  const from = Math.min(offset, timelineMotions.length);
  return { motions: { offset: from, count: timelineMotions.length - from, total: timelineMotions.length, motions: timelineMotions.slice(from) }, scope: { kind: 'allEnabled' } };
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
      if (command.operation === 'updateSettings') return Response.json(envelope(documentData(), requestId, revision));
      if (command.operation === 'contours') return Response.json(envelope(contoursData(), requestId, revision));
      if (command.operation === 'motions') return Response.json(envelope(motionsData(Number(command.offset ?? 0)), requestId, revision));
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
  it('sends face settings through UpdateSettings inside the kind envelope', async () => {
    const { fetcher, bodies } = scripted();
    const service = createHttpSequenceService(fetcher);
    const document = await service.open(legacyJob);
    await service.updateSettings(document.job, 'flat-v-carve', {
      kind: 'face',
      settings: {
        area: { kind: 'rectangle', rect: { min_x_mm: 0, min_y_mm: 0, width_mm: 40, length_mm: 30 } },
        margins: { min_x_mm: 1 },
        top: { reference: { kind: 'stock_top' }, offset_mm: 0 },
        bottom: { reference: { kind: 'stock_top' }, offset_mm: -0.5 },
        pattern: 'zig_zag',
        assignment: { tool_id: 'endmill' },
      },
    });
    const command = bodies.at(-1)?.command as Record<string, unknown>;
    expect(command.operation).toBe('updateSettings');
    expect(command.operationId).toBe('flat-v-carve');
    // The engine parses OperationSettings strictly: the adjacently tagged
    // kind envelope must survive the wire exactly.
    expect(command.settings).toEqual({
      kind: 'face',
      settings: expect.objectContaining({
        area: { kind: 'rectangle', rect: { min_x_mm: 0, min_y_mm: 0, width_mm: 40, length_mm: 30 } },
        pattern: 'zig_zag',
      }),
    });
  });
  it('pages motions and fetches the contour catalogue', async () => {
    const { fetcher, bodies } = scripted();
    const service = createHttpSequenceService(fetcher);
    const document = await service.open(legacyJob);
    const catalogue = await service.contours(document.job);
    expect(catalogue.contours[0].id).toBe('pocket-0-outer');
    expect(catalogue.contours[0].suggestedSide).toBe('outside');
    expect(catalogue.contours[0].sourceFingerprint).toBe('fp-1');
    const page = await service.planMotions(document.job, { kind: 'allEnabled' }, 0);
    expect(page.motions.total).toBe(5);
    expect(motionPageResultSchema.parse(motionsData(0)).motions.offset).toBe(0);
    const motionCommands = bodies
      .map(body => body.command as Record<string, unknown>)
      .filter(command => command.operation === 'motions');
    expect(motionCommands[0].offset).toBe(0);
    expect(motionCommands[0].scope).toEqual({ kind: 'allEnabled' });
  });
  it('sends profile entry and lead settings through UpdateSettings inside the kind envelope', async () => {
    const { fetcher, bodies } = scripted();
    const service = createHttpSequenceService(fetcher);
    const document = await service.open(legacyJob);
    await service.updateSettings(document.job, 'profile-1', {
      kind: 'profile',
      settings: {
        contours: [{ contour_id: 'pocket-0-outer', side: 'outside', traversal: null }],
        assignment: { tool_id: 'endmill' },
        top: { reference: { kind: 'stock_top' }, offset_mm: 0 },
        bottom: { reference: { kind: 'operation_top' }, offset_mm: -4 },
        stepdown_mm: 2,
        direction: 'climb',
        start: { kind: 'anchor', contour_id: 'pocket-0-outer', source_geometry_fingerprint: 'fp-1', fraction_along_source_contour: 0.15 },
        entry: { kind: 'ramp', max_angle_deg: 30, feed_mm_min: 140 },
        lead_in: { kind: 'tangent_line', length_mm: 3, feed_mm_min: 180 },
        lead_out: { kind: 'none' },
        tabs: { height_mm: 2, width_mm: 5, shape: 'rectangular', placement: { kind: 'automatic', count: 2, spacing_mm: null } },
      },
    });
    const command = bodies.at(-1)?.command as Record<string, unknown>;
    expect(command.operation).toBe('updateSettings');
    // The engine parses OperationSettings strictly: the adjacently tagged
    // kind envelope must survive the wire exactly.
    expect(command.settings).toEqual({
      kind: 'profile',
      settings: expect.objectContaining({
        entry: { kind: 'ramp', max_angle_deg: 30, feed_mm_min: 140 },
        lead_in: { kind: 'tangent_line', length_mm: 3, feed_mm_min: 180 },
        start: { kind: 'anchor', contour_id: 'pocket-0-outer', source_geometry_fingerprint: 'fp-1', fraction_along_source_contour: 0.15 },
      }),
    });
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
    updateSettings: async () => sequenceDocumentSchema.parse(documentData()),
    plan: async () => planResultSchema.parse(planData()),
    planMotions: async () => motionPageResultSchema.parse(motionsData(0)),
    contours: async () => contourCatalogueResultSchema.parse(contoursData()),
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
  it('renders the face settings editor with area, margin and raster controls', () => {
    const editorService = {
      ...service,
      updateSettings: async () => sequenceDocumentSchema.parse(documentData()),
    };
    const settings = {
      area: { kind: 'rectangle', rect: { min_x_mm: 0, min_y_mm: 0, width_mm: 40, length_mm: 30 } },
      margins: { min_x_mm: 1, max_x_mm: null, min_y_mm: null, max_y_mm: null },
      entry_overrun_mm: 2, exit_overrun_mm: 1,
      top: { reference: { kind: 'stock_top' }, offset_mm: 0 },
      bottom: { reference: { kind: 'stock_top' }, offset_mm: -0.5 },
      stepdown_mm: 0.5, stepover_mm: 3, pass_angle_deg: 0, pattern: 'zig_zag',
      assignment: { tool_id: 'endmill' },
    };
    const html = renderToStaticMarkup(
      <FaceSettingsEditor
        service={editorService as never}
        job={{ schema_version: 4 }}
        operationId="face-1"
        settings={settings as never}
        busy={false}
        onApplied={() => {}}
      />,
    );
    expect(html).toContain('face-1');
    expect(html).toContain('Rectangle min X (mm)');
    expect(html).toContain('Margin min X (mm)');
    expect(html).toContain('Stepdown (mm)');
    expect(html).toContain('Entry overrun (mm)');
    expect(html).toContain('Apply face settings');
    // The engine parses the whole settings object; the editor never edits the
    // job in place, and the form's untouched fields ride along unchanged.
    expect(html).toContain('value="40"');
    expect(html).toContain('value="1"');
  });
  it('renders the profile editor with entry, lead, start, tabs and finish controls', async () => {
    const applied: unknown[] = [];
    const editorService = {
      ...service,
      updateSettings: async (_job: unknown, operationId: string, settings: unknown) => {
        applied.push({ operationId, settings });
        return sequenceDocumentSchema.parse(documentData());
      },
    };
    const settings = {
      contours: [{ contour_id: 'pocket-0-outer', side: 'outside', traversal: null }],
      top: { reference: { kind: 'stock_top' }, offset_mm: 0 },
      bottom: { reference: { kind: 'operation_top' }, offset_mm: -4 },
      stepdown_mm: 3, through_cut_allowance_mm: 0.2, direction: 'climb',
      order: 'inner_before_outer',
      assignment: { tool_id: 'endmill', spindle_rpm: 12000, spindle_direction: 'clockwise', cutting_feed_mm_min: 350, plunge_feed_mm_min: 100, max_stepdown_mm: 8 },
      start: { kind: 'automatic' },
      finish: { enabled: true, radial_allowance_mm: 0.5, feed_mm_min: 300 },
      entry: { kind: 'ramp', max_angle_deg: 30, feed_mm_min: 140 },
      lead_in: { kind: 'tangent_line', length_mm: 3, feed_mm_min: 180 },
      lead_out: { kind: 'tangent_arc', radius_mm: 5, sweep_deg: 90, feed_mm_min: 190 },
      tabs: { height_mm: 2, width_mm: 5, shape: 'rectangular', placement: { kind: 'automatic', count: 2, spacing_mm: null } },
    };
    const html = renderToStaticMarkup(
      <ProfileSettingsEditor
        service={editorService as never}
        job={{ schema_version: 4 }}
        operationId="profile-1"
        settings={settings as never}
        faceOperations={[{ id: 'face-1' }]}
        busy={false}
        onApplied={() => {}}
      />,
    );
    expect(html).toContain('profile-1');
    expect(html).toContain('loading contour catalogue');
    // The E3 inspector fields render with the engine-parsed values intact.
    expect(html).toContain('Depth entry');
    expect(html).toContain('Ramp angle (deg)');
    expect(html).toContain('value="30"');
    expect(html).toContain('Lead-in');
    expect(html).toContain('Lead length (mm)');
    expect(html).toContain('Lead-out');
    expect(html).toContain('Arc radius (mm)');
    expect(html).toContain('value="90"');
    expect(html).toContain('Start');
    expect(html).toContain('Tab height (mm)');
    expect(html).toContain('Radial allowance (mm)');
    expect(html).toContain('Finish feed (mm/min)');
    // SSR shows the pending catalogue state; interaction arrives with it.
  });
  it('renders the stock/timeline display with operation checkpoints', () => {
    const html = renderToStaticMarkup(
      <SequenceTimeline
        service={service as never}
        job={{ schema_version: 4, tools: [{ id: 'endmill', geometry: { kind: 'endmill', dimensions: { diameter_mm: 4, cutting_length_mm: 12 } } }] }}
        planSummary={planResultSchema.parse(planData()).summary}
        stock={{ x0: 0, y0: 0, x1: 40, y1: 30, thicknessMm: 8 }}
        workZero={{ x: 20, y: 15 }}
      />,
    );
    expect(html).toContain('Stock &amp; timeline');
    expect(html).toContain('loading motions');
    expect(html).toContain('flat-v-carve');
    // Tab inspection (E3): the resolved bridge is announced with exact
    // geometry and a depth probe, independent of the display grid.
    expect(html).toContain('Tab inspection: 1 resolved bridge');
    expect(html).toContain('depth probe');
  });
});
