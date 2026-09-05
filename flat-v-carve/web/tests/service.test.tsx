import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { fixtureService } from '../src/service/fixture';
import { outputBlockedReasons } from '../src/contracts/service';
import { Viewport, placePoint } from '../src/components/Viewport';
import { readRecovery } from '../src/App';
import { newDraft } from '../src/state/draft';

describe('fixture service', () => {
  it('provides immutable captured geometry with original IDs and holes', async () => {
    const first = await fixtureService.openExample();
    expect(first.display.components).toHaveLength(7);
    expect(first.display.components.find(component => component.id === 'letter-b::0')?.rings.filter(ring => ring.hole)).toHaveLength(2);
    const o = first.display.components.find(component => component.id === 'letter-o::0')!;
    expect(Math.min(...o.rings[0].points.map(point => point.x))).toBe(5);
    first.job.name = 'Edited'; first.display.components.length = 0;
    const second = await fixtureService.openExample();
    expect(second.job.name).not.toBe('Edited');
    expect(second.display.components).toHaveLength(7);
  });
  it('declines source or precision changes instead of returning unrelated geometry', async () => {
    const { job } = await fixtureService.openExample();
    expect(await fixtureService.displayFor(job)).not.toBeNull();
    job.import.geometry_tolerance_mm *= 2;
    expect(await fixtureService.displayFor(job)).toBeNull();
    const next = (await fixtureService.openExample()).job;
    next.source.svg = '<svg onload="alert(1)" />';
    expect(await fixtureService.displayFor(next)).toBeNull();
  });
  it('does not adopt the legacy planning_available boolean as an engine capability', async () => {
    const capabilities = await fixtureService.capabilities();
    expect(capabilities.planningStages).toEqual([]);
    expect(capabilities.verificationScopes).toEqual([]);
    expect(outputBlockedReasons(capabilities)).toHaveLength(4);
    expect(outputBlockedReasons({ ...capabilities, mode: 'live', verificationScopes: ['continuous-stock'], exportFormats: ['linuxcnc'] })).toEqual(['No current, independently verified plan and checked output are loaded.']);
    expect(outputBlockedReasons(null)).not.toHaveLength(0);
    const { job } = await fixtureService.openExample();
    expect((await fixtureService.validateDraft(job, 18)).authoritative).toBe(false);
  });
  it('honors request cancellation', async () => {
    const controller = new AbortController(); controller.abort();
    await expect(fixtureService.openExample(controller.signal)).rejects.toThrow();
  });
});

describe('display boundary', () => {
  it('uses display-only placement with the documented Y-up transform', () => {
    const point = placePoint({ x: 12, y: 8 }, { origin_mm: { x: 2, y: 3 }, scale: 2, rotation_deg: 90 });
    expect(point.x).toBeCloseTo(-10);
    expect(point.y).toBeCloseTo(20);
  });
  it('renders inert normalized rings, holes, and hidden/inspected states without source markup', async () => {
    const { job, display } = await fixtureService.openExample();
    job.source.svg = '<svg onload="alert(1)"><script>bad()</script></svg>';
    const markup = renderToStaticMarkup(<Viewport job={job} display={display} inspected="letter-b::0" onInspect={() => {}} hidden={new Set(['disk::0'])} />);
    expect(markup).toContain('fill-rule="evenodd"');
    expect(markup).toContain('artwork-region included inspected');
    expect(markup).not.toContain('data-component="disk::0"');
    expect(markup).not.toContain('onload');
    expect(markup).not.toContain('<script');
    expect(markup).toContain('no planned cuts');
  });
});

describe('tab recovery', () => {
  it('allows a stateless session when browser storage is unavailable', () => {
    expect(readRecovery({ getItem: () => { throw new Error('Storage denied'); } })).toBeNull();
  });
  it('retains invalid field text separately from the portable job', async () => {
    const { job } = await fixtureService.openExample();
    const draft = newDraft(job); draft.text['stock.thickness_mm'] = '1e-';
    const result = readRecovery({ getItem: () => JSON.stringify({ version: 1, draft }) });
    expect(result).toEqual(draft);
    expect(result?.base.stock.thickness_mm).toBeNull();
  });
  it('rejects unknown text paths and malformed/future recovery without mutating storage', async () => {
    const { job } = await fixtureService.openExample();
    const draft = newDraft(job); draft.text['__proto__.polluted'] = '1';
    expect(() => readRecovery({ getItem: () => JSON.stringify({ version: 1, draft }) })).toThrow(/Invalid recovery field/);
    expect(() => readRecovery({ getItem: () => '{' })).toThrow();
    expect(() => readRecovery({ getItem: () => JSON.stringify({ version: 2, draft }) })).toThrow(/Unsupported recovery/);
  });
});
