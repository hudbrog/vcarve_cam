import { describe, expect, it } from 'vitest';
import { motionPageSchema } from '../../src/contracts/planning';
import type { Motion, PlanTask } from '../../src/contracts/planning';
import { apiVersion } from '../../src/contracts/wire';

// U8 phase-1 experiment 2: cost of JSON.parse plus zod validation for the
// paged motion stream. This is a measurement, not a functional gate: it logs
// throughput and extrapolates the flower-scale (~100 MB) total, with a very
// generous floor assertion so slow CI runners cannot flake.
const PAGE = 20_000;
const PAGES = 13; // ~260k motions, ~50 MB; extrapolate to flower scale
const flowerBytes = 100_000_000;

function pageTask(total: number): PlanTask {
  return {
    apiVersion,
    engineVersion: '0.7.6',
    instanceId: 'a'.repeat(32),
    taskId: 'bench-plan',
    revision: 1,
    documentFingerprint: 'b'.repeat(64),
    stage: 'combined',
    sequence: 1,
    state: 'succeeded',
    diagnostic: null,
    resultAvailable: true,
    summary: {
      engineVersion: '0.7.6',
      status: 'complete',
      inputFingerprint: 'c'.repeat(64),
      motionFingerprint: 'd'.repeat(64),
      meaning: 'measurement fixture',
      limitations: [],
      motionCount: total,
      cuttingMotionCount: total,
      previewMotionCount: total,
      omittedMotionCount: 0,
      diagnostics: [],
      omittedDiagnostics: 0,
      generationIssues: [],
      omittedGenerationIssues: 0,
    },
  };
}

describe('motion page parse and validation throughput', () => {
  it('measures JSON.parse and zod page validation rates', () => {
    const total = PAGE * PAGES;
    const task = pageTask(total);
    const random = (() => {
      let state = 987654321;
      return () => (state = (Math.imul(state, 1664525) + 1013904223) >>> 0) / 2 ** 32;
    })();
    const strings: string[] = [];
    let id = 0;
    const generateStart = performance.now();
    for (let page = 0; page < PAGES; page++) {
      const motions: Motion[] = [];
      for (let index = 0; index < PAGE; index++) {
        motions.push({
          id: id++,
          tool_id: index % 3 === 0 ? 'vbit' : 'endmill',
          operation_id: 'carve',
          layer: (id / 50_000) | 0,
          kind: index % 7 === 0 ? 'rapid_x_y' : index % 5 === 0 ? 'plunge' : 'cut',
          start: { x: +(random() * 900).toFixed(6), y: +(random() * 600).toFixed(6), z: -+(random() * 3).toFixed(6) },
          end: { x: +(random() * 900).toFixed(6), y: +(random() * 600).toFixed(6), z: -+(random() * 3).toFixed(6) },
          feed_mm_min: index % 2 === 0 ? 1800 : 1000,
        });
      }
      strings.push(JSON.stringify({
        task,
        coordinateSpace: 'workpiece-mm-z-up',
        motions,
        offset: page * PAGE,
        nextMotionOffset: page + 1 === PAGES ? null : (page + 1) * PAGE,
      }));
    }
    const generateMs = performance.now() - generateStart;

    const bytes = strings.reduce((sum, text) => sum + text.length, 0);
    const parseStart = performance.now();
    const parsed = strings.map(text => JSON.parse(text));
    const parseMs = performance.now() - parseStart;

    const validateStart = performance.now();
    for (const page of parsed) {
      const result = motionPageSchema.safeParse(page);
      if (!result.success) throw new Error(`page validation failed: ${result.error.message}`);
    }
    const validateMs = performance.now() - validateStart;

    const parseRate = bytes / (parseMs / 1000);
    const validateRate = bytes / (validateMs / 1000);
    const combined = parseMs + validateMs;
    console.log([
      `motion pages: ${PAGES} x ${PAGE} motions, ${(bytes / 1e6).toFixed(1)} MB (generation ${generateMs.toFixed(0)} ms, excluded)`,
      `JSON.parse: ${(parseRate / 1e6).toFixed(1)} MB/s -> ${parseMs.toFixed(0)} ms`,
      `zod validation: ${(validateRate / 1e6).toFixed(1)} MB/s -> ${validateMs.toFixed(0)} ms`,
      `combined: ${combined.toFixed(0)} ms here; projected ~100 MB flower stream: ${(combined * flowerBytes / bytes / 1000).toFixed(1)} s`,
    ].join('\n'));
    // Floor far below measured desktop rates (~tens of MB/s); only guards
    // against a pathological regression in the contract schemas.
    expect(validateRate).toBeGreaterThan(1_000_000);
  });
});
