import { useEffect, useMemo, useRef, useState } from 'react';
import type { CamService } from '../contracts/service';
import type { Job } from '../contracts/job';
import type { PlanResult, SliceResponse } from '../contracts/planning';
import type { DisplayBounds, StockLayer } from '../contracts/stock';

export type InspectionView = 'motions' | 'endmill' | 'combined';
export type ToolFilter = 'all' | 'endmill' | 'vbit';
export function filterMotions(result: PlanResult, job: Job, tool: ToolFilter, layer: string) {
  const toolId = tool === 'endmill' ? job.operation.endmill_id : tool === 'vbit' ? job.operation.vbit_id : null;
  return result.motions.filter(m => (!toolId || m.tool_id === toolId) && (layer === 'all' || m.layer === Number(layer)));
}
export function useInspection(service: CamService, result: PlanResult | null, current: boolean, job: Job, refresh: number) {
  const [chosenView, setChosenView] = useState<InspectionView>('motions');
  const [choice, setChoice] = useState('');
  const [tool, setTool] = useState<ToolFilter>('all');
  const [layer, setLayer] = useState('all');
  const [visible, setVisible] = useState<Set<StockLayer>>(() => new Set(['nominalTarget', 'removedLower', 'remainingTarget']));
  const [loaded, setLoaded] = useState<{ key: string; response?: SliceResponse; error?: string } | null>(null);
  const [focus, setFocus] = useState<{ key: string; bounds: DisplayBounds; serial: number } | null>(null);
  const [retry, setRetry] = useState(0);
  const views: InspectionView[] = ['motions'];
  if (result?.stockSlices.some(s => s.stage === 'endmill')) views.push('endmill');
  if (result?.stockSlices.some(s => s.stage === 'combined')) views.push('combined');
  const view = views.includes(chosenView) ? chosenView : 'motions';
  const slices = result?.stockSlices.filter(s => s.stage === view) ?? [];
  const selected = slices.find(s => s.id === choice) ?? slices.at(-1);
  const key = current && result && selected ? `${result.task.instanceId}/${result.task.taskId}/${selected.id}/${refresh}/${retry}` : null;
  const request = useRef({ task: result?.task, selected }); request.current = { task: result?.task, selected };
  useEffect(() => {
    if (!key || !service.stockSlice) return;
    const controller = new AbortController();
    const { task, selected } = request.current;
    setLoaded({ key });
    service.stockSlice(task!, selected!, controller.signal).then(response => {
      if (!controller.signal.aborted) setLoaded({ key, response });
    }).catch(error => { if (!controller.signal.aborted) setLoaded({ key, error: String(error) }); });
    return () => controller.abort();
  }, [service, key]);
  // Rendering checks the current request key synchronously, before cleanup effects.
  const response = key && loaded?.key === key ? loaded.response : undefined;
  const error = key && !service.stockSlice ? 'This service does not support stock inspection.' : key && loaded?.key === key ? loaded.error : undefined;
  const pending = !!key && !response && !error;
  const effectiveTool = view === 'endmill' ? 'endmill' : tool;
  const candidates = useMemo(() => result ? filterMotions(result, job, effectiveTool, 'all') : [], [result, job, effectiveTool]);
  const layers = useMemo(() => Array.from(new Set(candidates.map(m => m.layer))).sort((a, b) => a - b), [candidates]);
  const effectiveLayer = layer === 'all' || layers.includes(Number(layer)) ? layer : 'all';
  const motions = useMemo(() => current && result ? candidates.filter(m => effectiveLayer === 'all' || m.layer === Number(effectiveLayer)) : undefined, [current, result, candidates, effectiveLayer]);
  const geometry = response?.slice.geometry?.filter(r => visible.has(r.key)) ?? [];
  function setView(value: InspectionView) { setChosenView(value); setChoice(''); setLayer('all'); setFocus(null); }
  function inspectRegion(region: StockLayer) {
    const bounds = response?.slice.info.regions.find(r => r.key === region)?.bounds;
    if (!key || !bounds || !response?.slice.geometry) return;
    setVisible(previous => new Set([...previous, region]));
    setFocus(previous => ({ key, bounds, serial: (previous?.serial ?? 0) + 1 }));
  }
  return { current, view, views, setView, selected, slices, setSlice: setChoice,
    tool: effectiveTool, setTool: (value: ToolFilter) => { setTool(value); setLayer('all'); },
    layer: effectiveLayer, layers, setLayer, motions, geometry, response, error, pending,
    visible, toggleLayer: (value: StockLayer) => setVisible(previous => {
      const next = new Set(previous); if (next.has(value)) next.delete(value); else next.add(value); return next;
    }), inspectRegion, focus: key && focus?.key === key ? focus : null, retry: () => setRetry(value => value + 1),
    previewMotionCount: result?.motions.length ?? 0, omittedMotionCount: result?.task.summary?.omittedMotionCount ?? 0,
  };
}
