// Stock/timeline display for sequence plans (D3 + E3, plan sections 15.2,
// 15.3): pages the complete ordered motion stream, tracks per-operation
// ownership in the heightfield, and renders a top-down stock view with
// operation checkpoints and scrubbing. Inspection: resolved tab bridges are
// drawn as an exact overlay (never implied by the display grid) and a depth
// probe reads remaining material under the cursor with its resolution.
import { useEffect, useMemo, useRef, useState } from 'react';
import type { ReactElement } from 'react';
import type { MotionPageResult, PlanSummary, SequenceService } from '../contracts/sequence';
import {
  MAX_DISPLAY_MOTIONS,
  applyRange,
  buildSequenceStore,
  captureCheckpoint,
  createSequenceField,
  operationColors,
  pageAllMotions,
  probeDepth,
  restoreCheckpoint,
  sequenceTools,
  stockViewColor,
  type SequenceField,
  type StockCheckpoint,
} from '../sim/sequenceSim';
import type { StockRect } from '../sim/engine';

export interface TimelineOperationInfo {
  id: string;
  name: string;
  stages: string[];
}

/** Resolved profile tab bridges (exact geometry, setup millimetres). */
interface TabBridge {
  contourId: string;
  topZMm: number;
  footprintMm: [number, number][];
}

export function SequenceTimeline({
  service,
  job,
  planSummary,
  stock,
  workZero,
}: {
  service: SequenceService;
  job: unknown;
  planSummary: PlanSummary;
  stock: StockRect;
  /** Work-zero point in setup coordinates (marker only; never moves stock). */
  workZero?: { x: number; y: number } | null;
}): ReactElement {
  const [status, setStatus] = useState('loading motions…');
  const [error, setError] = useState('');
  const [loaded, setLoaded] = useState<{ field: SequenceField; checkpoints: StockCheckpoint[]; operationIds: string[] } | null>(null);
  const [position, setPosition] = useState(0);
  const [operationMode, setOperationMode] = useState(false);
  const [showTabs, setShowTabs] = useState(true);
  const [probe, setProbe] = useState<{ x: number; y: number; depthMm: number; cellMm: number } | null>(null);
  const canvas = useRef<HTMLCanvasElement | null>(null);

  const operations = useMemo<TimelineOperationInfo[]>(() => {
    const names = new Map<string, TimelineOperationInfo>();
    for (const stage of planSummary.stages) {
      const entry = names.get(stage.operationId) ?? { id: stage.operationId, name: stage.operationId, stages: [] };
      entry.stages.push(stage.role);
      names.set(stage.operationId, entry);
    }
    return [...names.values()];
  }, [planSummary]);

  const tabBridges = useMemo<TabBridge[]>(() => {
    const bridges: TabBridge[] = [];
    for (const operation of planSummary.operations) {
      for (const output of operation.namedOutputs) {
        if (output.kind !== 'profile_tabs') continue;
        for (const placement of output.tabPlacements) {
          if (placement.footprintMm.length === 4) {
            bridges.push({
              contourId: placement.contourId,
              topZMm: placement.topZMm,
              footprintMm: placement.footprintMm,
            });
          }
        }
      }
    }
    return bridges;
  }, [planSummary]);

  // Page the complete ordered stream, then build the field and capture a
  // checkpoint after every operation (plan 15.2: bounded pages are not a
  // complete simulation; show loading until everything arrived).
  useEffect(() => {
    const controller = new AbortController();
    let cancelled = false;
    const operationIds = operations.map(op => op.id);
    (async () => {
      setError('');
      setLoaded(null);
      setStatus('loading motions…');
      try {
        const { motions, total } = await pageAllMotions(offset =>
          service.planMotions(job, { kind: 'allEnabled' }, offset, controller.signal).then((page: MotionPageResult) => ({
            motions: { count: page.motions.count, total: page.motions.total, motions: page.motions.motions },
          })));
        if (cancelled) return;
        if (motions.length < total) {
          setStatus(`incomplete: ${motions.length} of ${total} motions loaded`);
          return;
        }
        if (total > MAX_DISPLAY_MOTIONS) {
          setStatus(`incomplete: ${total} motions exceed the ${MAX_DISPLAY_MOTIONS}-motion display cap`);
          return;
        }
        const tools = sequenceTools(job);
        const store = buildSequenceStore(motions, tools, operationIds);
        const field = createSequenceField(stock, store, tools);
        const checkpoints: StockCheckpoint[] = [captureCheckpoint(field, 0)];
        let from = 0;
        for (let index = 0; index < operationIds.length; index++) {
          let to = from;
          while (to < store.count && store.opIndex[to] === index) to++;
          applyRange(field, from, to);
          checkpoints.push(captureCheckpoint(field, to));
          from = to;
        }
        setLoaded({ field, checkpoints, operationIds });
        setPosition(store.count);
        setStatus(`${store.count} motions · ${field.field.cols}×${field.field.rows} cells${field.resolution.cappedByTexels || field.resolution.cappedByBudget ? ` · capped at ${field.resolution.cellMm.toFixed(2)} mm/cell` : ''}`);
      } catch (cause) {
        if (!cancelled && !controller.signal.aborted) setError(String(cause));
      }
    })();
    return () => { cancelled = true; controller.abort(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [service, planSummary.executionFingerprint, stock.x0, stock.y0, stock.thicknessMm]);

  // Scrubbing: restore the nearest checkpoint at or before the target, then
  // apply the remaining motions.
  useEffect(() => {
    if (!loaded) return;
    const { field, checkpoints } = loaded;
    const target = Math.min(Math.max(position, 0), field.store.count);
    let base = 0;
    for (let index = 0; index < checkpoints.length; index++) {
      if (checkpoints[index].motionIndex <= target) base = index;
    }
    restoreCheckpoint(field, checkpoints[base]);
    applyRange(field, checkpoints[base].motionIndex, target);
    const canvasElement = canvas.current;
    if (!canvasElement) return;
    const context = canvasElement.getContext('2d');
    if (!context) return;
    const { field: stock, opOwner } = field;
    const colors = { operation: operationColors(loaded.operationIds.length) };
    // One canvas pixel per cell; CSS scales the element (pixelated).
    if (canvasElement.width !== stock.cols || canvasElement.height !== stock.rows) {
      canvasElement.width = stock.cols;
      canvasElement.height = stock.rows;
    }
    const image = context.createImageData(stock.cols, stock.rows);
    for (let row = 0; row < stock.rows; row++) {
      for (let col = 0; col < stock.cols; col++) {
        const cell = row * stock.cols + col;
        const level = stock.heights[Math.floor(col / 256) + Math.floor(row / 256) * stock.tilesX]?.[(col % 256) + (row % 256) * 256] ?? 0;
        const depthFraction = level / 65535;
        const [r, g, b] = stockViewColor(depthFraction, depthFraction >= 1, opOwner[cell], colors, operationMode);
        image.data[cell * 4] = r;
        image.data[cell * 4 + 1] = g;
        image.data[cell * 4 + 2] = b;
        image.data[cell * 4 + 3] = 255;
      }
    }
    context.putImageData(image, 0, 0);
    // Exact tab-bridge overlay: resolved plan geometry, so a bridge is never
    // hidden by display-grid resolution (plan section 15.3).
    if (showTabs && tabBridges.length > 0) {
      context.lineWidth = Math.max(1, stock.cellMm * 0.15);
      context.strokeStyle = 'rgba(255,176,32,0.95)';
      for (const bridge of tabBridges) {
        context.beginPath();
        bridge.footprintMm.forEach(([x, y], index) => {
          const col = (x - stock.x0Mm) / stock.cellMm;
          const row = (y - stock.y0Mm) / stock.cellMm;
          if (index === 0) context.moveTo(col, row);
          else context.lineTo(col, row);
        });
        context.closePath();
        context.stroke();
      }
    }
  }, [loaded, position, operationMode, showTabs, tabBridges]);

  // Depth probe: remaining material under the cursor, with the display
  // resolution reported so tab heights are inspected, not guessed from the
  // color ramp.
  function probeAt(event: React.MouseEvent<HTMLCanvasElement>) {
    if (!loaded) return;
    const element = canvas.current;
    if (!element) return;
    const bounds = element.getBoundingClientRect();
    const xMm = stock.x0 + (event.clientX - bounds.left) / bounds.width * (stock.x1 - stock.x0);
    const yMm = stock.y0 + (event.clientY - bounds.top) / bounds.height * (stock.y1 - stock.y0);
    const result = probeDepth(loaded.field, xMm, yMm);
    setProbe(result ? { x: xMm, y: yMm, ...result } : null);
  }

  const currentOperation = loaded && position > 0
    ? loaded.operationIds[loaded.field.store.opIndex[Math.min(position, loaded.field.store.count) - 1]]
    : null;
  const volumes = loaded?.checkpoints.at(-1)?.removedByOperationMm3 ?? [];

  return <div className="sequence-timeline">
    <h3>Stock & timeline</h3>
    {!loaded && <p className="hint" role="status">{error || status}</p>}
    {error && <p className="inline-warning" role="alert">{error}</p>}
    {tabBridges.length > 0 && <p className="hint">Tab inspection: {tabBridges.length} resolved bridge{tabBridges.length === 1 ? '' : 's'} on {new Set(tabBridges.map(bridge => bridge.contourId)).size} contour(s), drawn as exact-plan overlays with a depth probe — never implied by the display grid.</p>}
    {loaded && <>
      <div className="sequence-timeline-canvas-row">
        <canvas
          ref={canvas}
          className="sequence-stock-canvas"
          aria-label="Top-down stock view after the selected motions"
          onMouseMove={probeAt}
          onMouseLeave={() => setProbe(null)}
        />
        {workZero && <small className="hint">Work zero marker at setup ({workZero.x.toFixed(1)}, {workZero.y.toFixed(1)}) mm; the stock rectangle never moves with it.</small>}
      </div>
      {probe
        ? <p className="hint" role="status">Depth probe at ({probe.x.toFixed(2)}, {probe.y.toFixed(2)}) mm: {probe.depthMm.toFixed(2)} mm of material remaining · cell {probe.cellMm.toFixed(2)} mm</p>
        : <p className="hint">Hover the stock view to probe the remaining material depth.</p>}
      <label className="sequence-field sequence-scrub">
        <span>Motion {position} of {loaded.field.store.count}</span>
        <input
          type="range"
          min={0}
          max={loaded.field.store.count}
          value={position}
          aria-label="Simulation position in motions"
          onChange={event => setPosition(Number(event.target.value))}
        />
      </label>
      <div className="inline-actions">
        <label className="sequence-field">
          <span>Coloring</span>
          <select aria-label="Stock coloring" value={operationMode ? 'operation' : 'depth'} onChange={event => setOperationMode(event.target.value === 'operation')}>
            <option value="depth">Depth</option>
            <option value="operation">Operation</option>
          </select>
        </label>
        {tabBridges.length > 0 && <label className="sequence-field sequence-tab-toggle">
          <input
            type="checkbox"
            checked={showTabs}
            aria-label="Show resolved tab bridges"
            onChange={event => setShowTabs(event.target.checked)}
          />
          <span>{tabBridges.length} tab bridge{tabBridges.length === 1 ? '' : 's'} (exact geometry at Z {tabBridges[0].topZMm.toFixed(1)})</span>
        </label>}
        {loaded.checkpoints.map((checkpoint, index) => index === 0 ? null : (
          <button key={index} onClick={() => setPosition(checkpoint.motionIndex)}>
            After {loaded.operationIds[index - 1]}
          </button>
        ))}
      </div>
    </>}
    <ul className="sequence-timeline-ops">
      {operations.map((operation, index) => <li key={operation.id}>
        <strong>{operation.name}</strong> · {operation.stages.join(' → ')}
        {volumes[index] !== undefined && ` · ${volumes[index].toFixed(1)} mm³ removed`}
      </li>)}
    </ul>
    {loaded && <p className="hint">{status}{currentOperation && ` · at motion ${position}: operation ${currentOperation}`}</p>}
  </div>;
}
