import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { Job } from '../contracts/job';
import type { Motion } from '../contracts/planning';
import type { SliceInfo } from '../contracts/stock';
import { buildSimSetup } from '../sim/setup';
import type { MotionStore, Timing } from '../sim/store';
import { buildStore, buildTiming, indexForTime, poseAt, timeOfIndex } from '../sim/store';
import type { SessionStats } from '../sim/session';
import type { SimRenderer } from '../sim/renderer';

interface Props {
  motions: Motion[];
  job: Job;
  slices: SliceInfo[];
  onExit: () => void;
}

const RAPID_FEED = 3000;
const SPEEDS = [0.25, 1, 5, 15, 30, 60, 120, 200];
const LOOKAHEAD_SECONDS = 0.5;
// Long jumps run as a ladder of intermediate seeks so the surface and stats
// keep updating; the chunk size adapts toward ~100 ms of engine work.
const SEEK_CHUNK_START = 512;
const SEEK_CHUNK_MIN = 64;
const SEEK_CHUNK_MAX = 8192;
const SEEK_CHUNK_TARGET_MS = 100;

type Phase = 'loading' | 'ready' | 'error';

export function SimViewport({ motions, job, slices, onExit }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rendererRef = useRef<SimRenderer | null>(null);
  const workerRef = useRef<Worker | null>(null);
  const storeRef = useRef<MotionStore | null>(null);
  const timingRef = useRef<Timing | null>(null);
  const appliedRef = useRef(0);
  const inFlightRef = useRef(false);
  const modelTimeRef = useRef(0);
  const playingRef = useRef(false);
  const speedRef = useRef(15);
  const sentRef = useRef({ index: -1, fraction: -1 });
  const seekTargetRef = useRef<{ index: number; fraction: number } | null>(null);
  const chunkRef = useRef(SEEK_CHUNK_START);
  const sentAtRef = useRef(0);

  const [phase, setPhase] = useState<Phase>('loading');
  const [error, setError] = useState('');
  const [stats, setStats] = useState<SessionStats | null>(null);
  const [playing, setPlaying] = useState(false);
  const [speed, setSpeed] = useState(15);
  const [modelTime, setModelTime] = useState(0);
  const [totalSeconds, setTotalSeconds] = useState(0);
  const [zScale, setZScale] = useState(3);
  const [trackTool, setTrackTool] = useState(false);
  const [stageColors, setStageColors] = useState(true);
  const [clamped, setClamped] = useState(false);
  const [assumedFeeds, setAssumedFeeds] = useState(0);
  const [position, setPosition] = useState(0);
  const setup = useMemo(() => buildSimSetup(job, motions, slices), [job, motions, slices]);

  useEffect(() => { playingRef.current = playing; }, [playing]);
  useEffect(() => { speedRef.current = speed; }, [speed]);

  const postSeek = useCallback((index: number, fraction: number) => {
    const worker = workerRef.current;
    if (!worker || inFlightRef.current) return;
    sentRef.current = { index, fraction };
    sentAtRef.current = performance.now();
    inFlightRef.current = true;
    worker.postMessage({ type: 'seek', index, fraction });
  }, []);

  /** Route every seek through an adaptive ladder: far jumps advance in
   * chunks so the surface and percentages keep updating, and a newer target
   * redirects the jump mid-flight. */
  const dispatchSeek = useCallback((target: { index: number; fraction: number }) => {
    seekTargetRef.current = target;
    if (inFlightRef.current) return;
    const goal = target.index + target.fraction;
    const current = appliedRef.current;
    if (Math.abs(goal - current) <= SEEK_CHUNK_MIN) {
      postSeek(target.index, target.fraction);
      return;
    }
    const chunk = chunkRef.current;
    const forward = goal > current;
    const nextIndex = forward
      ? Math.min(Math.ceil(current) + chunk, target.index)
      : Math.max(Math.floor(current) - chunk, target.index);
    postSeek(nextIndex, nextIndex === target.index ? target.fraction : 0);
  }, [postSeek]);

  // Renderer lifecycle (three.js loads in its own chunk).
  useEffect(() => {
    let disposed = false;
    let observer: ResizeObserver | undefined;
    void (async () => {
      const { createSimRenderer } = await import('../sim/renderer');
      if (disposed || !canvasRef.current || !containerRef.current) return;
      const cols = Math.ceil((setup.stock.x1 - setup.stock.x0) / setup.resolution.cellMm);
      const rows = Math.ceil((setup.stock.y1 - setup.stock.y0) / setup.resolution.cellMm);
      const renderer = createSimRenderer(canvasRef.current, {
        stock: setup.stock,
        cols,
        rows,
        cellMm: setup.resolution.cellMm,
        tools: setup.tools,
      });
      rendererRef.current = renderer;
      renderer.setZScale(zScale);
      renderer.setStageColors(stageColors);
      renderer.setTrackTool(trackTool);
      const container = containerRef.current;
      observer = new ResizeObserver(() => renderer.resize(container.clientWidth, container.clientHeight));
      observer.observe(container);
      renderer.resize(container.clientWidth, container.clientHeight);
    })();
    return () => {
      disposed = true;
      observer?.disconnect();
      rendererRef.current?.dispose();
      rendererRef.current = null;
    };
    // Display-only options are applied through separate effects.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [setup]);

  useEffect(() => { rendererRef.current?.setZScale(zScale); rendererRef.current?.rebuildSkirts(); }, [zScale]);
  useEffect(() => { rendererRef.current?.setStageColors(stageColors); }, [stageColors]);
  useEffect(() => { rendererRef.current?.setTrackTool(trackTool); }, [trackTool]);

  // Worker lifecycle: build the compact store here once (fast typed-array
  // pass), transfer copies to the worker, and start from the pristine stock.
  // Transferring avoids cloning motion objects, which froze the main thread
  // for seconds on large plans.
  useEffect(() => {
    const worker = new Worker(new URL('../sim/simWorker.ts', import.meta.url), { type: 'module' });
    workerRef.current = worker;
    worker.onmessage = (event: MessageEvent) => {
      const data = event.data;
      if (data?.type === 'ready') {
        setPhase('ready');
        dispatchSeek({ index: 0, fraction: 0 });
        setPlaying(true);
      } else if (data?.type === 'seekDone') {
        inFlightRef.current = false;
        const elapsed = performance.now() - sentAtRef.current;
        const factor = Math.min(Math.max(SEEK_CHUNK_TARGET_MS / Math.max(elapsed, 5), 0.25), 4);
        chunkRef.current = Math.min(SEEK_CHUNK_MAX, Math.max(SEEK_CHUNK_MIN, Math.round(chunkRef.current * factor)));
        appliedRef.current = data.applied + data.fraction;
        setPosition(data.applied + data.fraction);
        setStats(data.stats);
        if (rendererRef.current) {
          rendererRef.current.applyTileDeltas(data.tiles);
          const goal = seekTargetRef.current;
          const settled = !goal || Math.abs(goal.index + goal.fraction - appliedRef.current) < 0.51;
          if (!playingRef.current && settled) rendererRef.current.rebuildSkirts();
        }
        // Continue an in-progress jump toward the newest target.
        const goal = seekTargetRef.current;
        if (goal && Math.abs(goal.index + goal.fraction - appliedRef.current) >= 0.51) {
          dispatchSeek(goal);
        }
      } else if (data?.type === 'error') {
        setPhase('error');
        setError(data.message);
      }
    };
    try {
      const store = buildStore(motions, { toolIndex: setup.toolIndex, assumedFeedMmMin: 1000 });
      storeRef.current = store;
      const timing = buildTiming(store, RAPID_FEED, false);
      timingRef.current = timing;
      setAssumedFeeds(store.assumedFeedCount);
      setTotalSeconds(timing.totalSeconds);
      const copy = {
        count: store.count,
        x0: store.x0.slice(), y0: store.y0.slice(), z0: store.z0.slice(),
        x1: store.x1.slice(), y1: store.y1.slice(), z1: store.z1.slice(),
        kind: store.kind.slice(), tool: store.tool.slice(), layer: store.layer.slice(),
        lengthMm: store.lengthMm.slice(), feedMmMin: store.feedMmMin.slice(),
        assumedFeedCount: store.assumedFeedCount,
      };
      worker.postMessage({
        type: 'init',
        stock: setup.stock,
        tools: setup.tools,
        toolIds: setup.toolIds,
        resolution: setup.resolution,
        store: copy,
      }, [
        copy.x0.buffer, copy.y0.buffer, copy.z0.buffer,
        copy.x1.buffer, copy.y1.buffer, copy.z1.buffer,
        copy.kind.buffer, copy.tool.buffer, copy.layer.buffer,
        copy.lengthMm.buffer, copy.feedMmMin.buffer,
      ]);
    } catch (error) {
      setPhase('error');
      setError(error instanceof Error ? error.message : String(error));
    }
    return () => {
      worker.terminate();
      workerRef.current = null;
    };
  }, [motions, setup, dispatchSeek]);

  // Playback clock: the main thread owns time; the worker never blocks it.
  useEffect(() => {
    if (phase !== 'ready') return;
    let raf = 0;
    let previous = performance.now();
    let lastPose = { index: -1, fraction: -1 };
    const tick = (now: number) => {
      raf = requestAnimationFrame(tick);
      const timing = timingRef.current;
      const store = storeRef.current;
      const renderer = rendererRef.current;
      if (!timing || !store || !renderer) return;
      const dt = Math.min((now - previous) / 1000, 0.25);
      previous = now;
      if (playingRef.current) {
        const ceiling = timeOfIndex(timing, Math.ceil(appliedRef.current)) + LOOKAHEAD_SECONDS;
        const next = Math.min(modelTimeRef.current + dt * speedRef.current, Math.min(ceiling, timing.totalSeconds));
        setClamped(next < modelTimeRef.current + dt * speedRef.current - 1e-9);
        modelTimeRef.current = next;
        setModelTime(next);
      }
      const target = indexForTime(timing, modelTimeRef.current);
      if (target.index !== lastPose.index || Math.abs(target.fraction - lastPose.fraction) > 1e-4) {
        lastPose = target;
        const pose = poseAt(store, target.index, target.fraction);
        renderer.setToolPose(pose.tool, pose.x, pose.y, pose.z);
      }
      if (target.index !== sentRef.current.index || Math.abs(target.fraction - sentRef.current.fraction) > 0.01) {
        dispatchSeek(target);
      }
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [phase, dispatchSeek]);

  const scrub = (value: number) => {
    const timing = timingRef.current;
    if (!timing) return;
    modelTimeRef.current = Math.min(Math.max(value, 0), timing.totalSeconds);
    setModelTime(modelTimeRef.current);
    setPlaying(false);
    const target = indexForTime(timing, modelTimeRef.current);
    dispatchSeek(target);
  };
  const step = (direction: 1 | -1) => {
    const timing = timingRef.current;
    if (!timing) return;
    setPlaying(false);
    const target = indexForTime(timing, modelTimeRef.current);
    const index = Math.min(Math.max(target.index + direction, 0), Math.max(motions.length - 1, 0));
    const time = timeOfIndex(timing, index);
    modelTimeRef.current = time;
    setModelTime(time);
    dispatchSeek({ index, fraction: 0 });
  };

  const totalText = totalSeconds > 0 ? formatSeconds(totalSeconds) : '0:00';
  return <div className="sim-viewport" ref={containerRef}>
    <canvas ref={canvasRef} />
    <div className="sim-badge">VISUAL PREVIEW · {(setup.resolution.cellMm).toFixed(3)} mm/cell{setup.resolution.cappedByTexels ? ' (texel-capped)' : ''}{setup.resolution.cappedByBudget ? ' (budget-capped)' : ''} · verification remains authoritative</div>
    <div className="sim-transport">
      <button onClick={() => { setPlaying(previous => !previous); rendererRef.current?.rebuildSkirts(); }} disabled={phase !== 'ready'}>{playing ? '❚❚ Pause' : '▶ Play'}</button>
      <button onClick={() => step(-1)} disabled={phase !== 'ready'} title="Previous motion">⏮ Motion</button>
      <button onClick={() => step(1)} disabled={phase !== 'ready'} title="Next motion">Motion ⏭</button>
      <label className="sim-scrub">
        <span className="sr-only">Playback position</span>
        <input type="range" min={0} max={Math.max(totalSeconds, 0.001)} step={Math.max(totalSeconds / 1000, 0.001)} value={modelTime}
          disabled={phase !== 'ready'} onChange={event => scrub(Number(event.target.value))} />
      </label>
      <span className="sim-time">{formatSeconds(modelTime)} / {totalText}</span>
      <label className="sim-speed">Speed
        <select value={speed} onChange={event => setSpeed(Number(event.target.value))} disabled={phase !== 'ready'}>
          {SPEEDS.map(option => <option key={option} value={option}>{option}×</option>)}
        </select>
      </label>
      {clamped && playing && <span className="sim-clamped" role="status">engine-limited</span>}
      <button onClick={() => scrub(0)} disabled={phase !== 'ready'}>Start</button>
      <button onClick={() => { const timing = timingRef.current; if (timing) scrub(timing.totalSeconds); }} disabled={phase !== 'ready'}>End</button>
      <button onClick={() => rendererRef.current?.fit()}>Fit</button>
      <label className="sim-option">Z ×<select value={zScale} onChange={event => setZScale(Number(event.target.value))}>{[1, 2, 3, 5, 8, 10].map(option => <option key={option} value={option}>{option}</option>)}</select></label>
      <label className="sim-option"><input type="checkbox" checked={trackTool} onChange={event => setTrackTool(event.target.checked)} /> Track tool</label>
      <label className="sim-option"><input type="checkbox" checked={stageColors} onChange={event => setStageColors(event.target.checked)} /> Stage colors</label>
      <button className="sim-exit" onClick={onExit}>Back to 2D</button>
    </div>
    <div className="sim-status">
      {phase === 'loading' && <span role="status">Preparing simulation · {motions.length.toLocaleString()} motions…</span>}
      {phase === 'error' && <span role="alert">Simulation error: {error}</span>}
      {phase === 'ready' && stats && <>
        <span>{(position / Math.max(motions.length, 1) * 100).toFixed(0)}% applied · {motions.length.toLocaleString()} motions</span>
        <span>removed {stats.removedVolumeMm3 >= 1000 ? `${(stats.removedVolumeMm3 / 1000).toFixed(1)} cm³` : `${stats.removedVolumeMm3.toFixed(0)} mm³`}</span>
        <span style={{ color: '#4a80d4' }}>endmill {(stats.stageRemovedMm3[0] / 1000).toFixed(1)} cm³</span>
        <span style={{ color: '#3fb8af' }}>v-bit {(stats.stageRemovedMm3[1] / 1000).toFixed(1)} cm³</span>
        {assumedFeeds > 0 && <span title="Cutting motions without a recorded feed use an assumed 1000 mm/min">{assumedFeeds.toLocaleString()} motions at assumed feed</span>}
        <span title="Tool-change dwell, spindle spin-up, and acceleration are not modeled">timing model: feeds + {RAPID_FEED} mm/min rapids</span>
      </>}
    </div>
  </div>;
}

function formatSeconds(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const rest = total % 60;
  const text = hours > 0 ? `${hours}:${String(minutes).padStart(2, '0')}:${String(rest).padStart(2, '0')}` : `${minutes}:${String(rest).padStart(2, '0')}`;
  return text;
}
