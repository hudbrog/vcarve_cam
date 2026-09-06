import { useEffect, useMemo, useRef, useState } from 'react';
import type { ArtworkDisplay } from '../contracts/service';
import type { Job, Point } from '../contracts/job';
import type { Motion } from '../contracts/planning';
import { stockLayers, type DisplayBounds, type SliceInfo, type StockRegion } from '../contracts/stock';
import type { Finding } from '../contracts/verification';

interface Props {
  display: ArtworkDisplay | null; job: Job; inspected: string | null;
  onInspect: (id: string) => void; hidden: Set<string>;
  motions?: Motion[];
  stockInfo?: SliceInfo;
  stockGeometry?: StockRegion[];
  stockPending?: boolean;
  stockError?: string;
  focus?: { bounds: DisplayBounds; serial: number } | null;
  verificationFinding?: Finding;
  verificationScope?: string;
}
export function placePoint(point: Point, placement: Job['import']['placement']): Point {
  const radians = placement.rotation_deg * Math.PI / 180;
  const x = point.x - placement.origin_mm.x;
  const y = point.y - placement.origin_mm.y;
  return { x: placement.scale * (Math.cos(radians) * x - Math.sin(radians) * y),
    y: placement.scale * (Math.sin(radians) * x + Math.cos(radians) * y) };
}
function pathData(rings: { points: Point[] }[]) {
  return rings.map(ring => ring.points.map((point, index) => `${index ? 'L' : 'M'}${point.x},${-point.y}`).join(' ') + ' Z').join(' ');
}
export function motionPath(motions: Motion[]): string {
  // Engine motions already use workpiece coordinates. Only flip Y for SVG.
  // Join only genuinely connected recorded segments; retain every endpoint.
  let previous: Motion | undefined;
  return motions.map(motion => {
    const connected = previous && previous.tool_id === motion.tool_id && previous.layer === motion.layer
      && previous.end.x === motion.start.x && previous.end.y === motion.start.y && previous.end.z === motion.start.z;
    previous = motion;
    return `${connected ? '' : `M${motion.start.x},${-motion.start.y}`}L${motion.end.x},${-motion.end.y}`;
  }).join(' ');
}
export function motionBounds(motions: Motion[]): DisplayBounds | null {
  if (!motions.length) return null;
  const min = { x: Infinity, y: Infinity }, max = { x: -Infinity, y: -Infinity };
  for (const motion of motions) for (const point of [motion.start, motion.end]) {
    min.x = Math.min(min.x, point.x); min.y = Math.min(min.y, point.y);
    max.x = Math.max(max.x, point.x); max.y = Math.max(max.y, point.y);
  }
  return { min, max };
}
export function stockPath(region: StockRegion): string { return pathData(region.rings); }
export function Viewport({ display, job, inspected, onInspect, hidden, motions, stockInfo, stockGeometry, stockPending, stockError, focus, verificationFinding, verificationScope }: Props) {
  const [camera, setCamera] = useState({ x: 50, y: 30, span: 125 });
  const [grid, setGrid] = useState(true);
  const [showMotions, setShowMotions] = useState(true);
  const [showTravel, setShowTravel] = useState(false);
  const stockPaths = useMemo(() => {
    const order = ['removedUpper', 'removedLower', 'accessibleFloor', 'requestedCenters', 'remainingTarget', 'missingFloor', 'possibleOvercut', 'nominalTarget'];
    return [...(stockGeometry ?? [])].sort((a, b) => order.indexOf(a.key) - order.indexOf(b.key)).map(region => ({ key: region.key, path: stockPath(region) }));
  }, [stockGeometry]);
  function fitBounds(bounds: DisplayBounds) {
    const { min, max } = bounds;
    setCamera({ x: (min.x + max.x) / 2, y: (min.y + max.y) / 2, span: Math.max(max.x - min.x, (max.y - min.y) / .7, .01) * 1.2 });
  }
  useEffect(() => { if (focus) fitBounds(focus.bounds); }, [focus]);
  const stockBounds = stockInfo?.regions.find(r => r.key === 'nominalTarget')?.bounds;
  const hatchSize = camera.span / 80;
  const motionGroups = useMemo(() => {
    const groups = new Map<string, Motion[]>();
    for (const motion of motions ?? []) {
      const travel = ['rapid_x_y', 'rapid_retract', 'approach'].includes(motion.kind);
      const key = `${motion.tool_id}:${travel ? 'travel' : 'cutting'}`;
      const group = groups.get(key) ?? []; group.push(motion); groups.set(key, group);
    }
    return Array.from(groups, ([key, group]) => {
      const chunks = [];
      for (let start = 0; start < group.length; start += 4096) chunks.push({ key, chunk: start, tool: group[0].tool_id,
        path: motionPath(group.slice(start, start + 4096)), travel: key.endsWith(':travel') });
      return chunks;
    }).flat();
  }, [motions]);
  const drag = useRef<{ x: number; y: number; camera: typeof camera } | null>(null);
  const components = useMemo(() => display?.components.map(component => ({ ...component,
    rings: component.rings.map(ring => ({ ...ring, points: ring.points.map(point => placePoint(point, job.import.placement)) })),
  })) ?? [], [display, job.import.placement]);
  const page = useMemo(() => display ? [{ x: 0, y: 0 }, { x: display.widthMm, y: 0 },
    { x: display.widthMm, y: display.heightMm }, { x: 0, y: display.heightMm }].map(point => placePoint(point, job.import.placement)) : [], [display, job.import.placement]);
  function fit(selection = false, plan = false) {
    if (plan) { const bounds = motionBounds(motions ?? []); if (bounds) fitBounds(bounds); return; }
    const points = selection ? components.filter(component => component.id === inspected).flatMap(component => component.rings.flatMap(ring => ring.points)) : page;
    if (!points.length) return;
    let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
    for (const point of points) { minX = Math.min(minX, point.x); maxX = Math.max(maxX, point.x); minY = Math.min(minY, point.y); maxY = Math.max(maxY, point.y); }
    setCamera({ x: (minX + maxX) / 2, y: (minY + maxY) / 2, span: Math.max(maxX - minX, (maxY - minY) / .7, 1) * 1.2 });
  }
  function zoom(factor: number) { setCamera(current => ({ ...current, span: Math.min(100000, Math.max(.01, current.span * factor)) })); }
  return <section className="viewport" aria-label="Artwork viewport">
    <div className="viewport-toolbar">
      <span className="view-title"><span className="view-dot" /> Artwork <span className="subtle">/ Top view</span></span>
      <div className="button-group">
        <button onClick={() => fit()}>Fit job</button>
        <button onClick={() => fit(true)} disabled={!inspected}>Fit inspected</button>
        {motions && <button onClick={() => fit(false, true)} disabled={!motions.length}>Fit plan</button>}
        {stockInfo && <button onClick={() => stockBounds && fitBounds(stockBounds)} disabled={!stockBounds || !stockGeometry?.length}>Fit slice</button>}
        <button onClick={() => zoom(1.25)} aria-label="Zoom out">−</button>
        <button onClick={() => zoom(.8)} aria-label="Zoom in">+</button>
        <button aria-pressed={grid} onClick={() => setGrid(!grid)}>Grid</button>
        {motions && <><button aria-pressed={showMotions} onClick={() => setShowMotions(!showMotions)}>Motions</button><button aria-pressed={showTravel} onClick={() => setShowTravel(!showTravel)}>Travel</button></>}
      </div>
    </div>
    <div className="drawing-wrap">
      {display ? <>
        <div className="drawing-note">{verificationFinding ? `VERIFICATION FINDING · ${verificationScope ?? "original coordinates"} · ${verificationFinding.code}` : stockInfo ? `STOCK SLICE · ${stockInfo.stage === 'endmill' ? 'after endmill' : 'after both tools'} · ${stockInfo.depthMm} mm deep${stockPending ? ' · loading…' : stockError || stockInfo.unavailableReason ? ' · geometry unavailable' : ''}` : motions ? motions.length === 0 ? 'NO RECORDED MOTIONS · see plan outcome' : showMotions ? 'RECORDED MOTIONS · top projection' : 'SOURCE GEOMETRY · motion overlay hidden' : 'SOURCE GEOMETRY · no planned cuts'}</div>
        <svg className="drawing" role="group" aria-label={`${verificationFinding ? `Verification finding ${verificationFinding.code} in ${verificationScope ?? "original coordinates"}. Path overlays show original recorded motions.` : stockInfo ? `Stock slice at ${stockInfo.depthMm} mm below stock top. Use the stock overlay controls and area table to inspect regions.` : 'Top view of normalized artwork. Inspect shapes with the source list.'} Arrow keys pan; plus and minus zoom; Home fits the job.`}
          tabIndex={0} viewBox={`${camera.x - camera.span / 2} ${-camera.y - camera.span * .35} ${camera.span} ${camera.span * .7}`}
          onKeyDown={event => {
            const step = camera.span * .08;
            if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', '+', '=', '-', 'Home'].includes(event.key)) event.preventDefault();
            if (event.key === 'Home') fit();
            if (event.key === '+' || event.key === '=') zoom(.8);
            if (event.key === '-') zoom(1.25);
            if (event.key.startsWith('Arrow')) setCamera(current => ({ ...current,
              x: current.x + (event.key === 'ArrowRight' ? step : event.key === 'ArrowLeft' ? -step : 0),
              y: current.y + (event.key === 'ArrowUp' ? step : event.key === 'ArrowDown' ? -step : 0) }));
          }}
          onPointerDown={event => {
            if (event.button !== 0 || (event.target as Element).closest('[data-component]')) return;
            event.currentTarget.setPointerCapture(event.pointerId);
            drag.current = { x: event.clientX, y: event.clientY, camera };
          }}
          onPointerMove={event => {
            if (!drag.current) return;
            const bounds = event.currentTarget.getBoundingClientRect();
            const pixelsPerMm = Math.min(bounds.width / camera.span, bounds.height / (camera.span * .7));
            setCamera({ ...drag.current.camera,
              x: drag.current.camera.x - (event.clientX - drag.current.x) / pixelsPerMm,
              y: drag.current.camera.y + (event.clientY - drag.current.y) / pixelsPerMm });
          }}
          onPointerUp={() => { drag.current = null; }} onPointerCancel={() => { drag.current = null; }}>
          <defs>
            <pattern id="grid" width="10" height="10" patternUnits="userSpaceOnUse">
              <path d="M10 0H0V10" fill="none" className="grid-line" strokeWidth="0.08" />
            </pattern>
            <pattern id="stock-remaining" width={hatchSize} height={hatchSize} patternUnits="userSpaceOnUse"><path d={`M0 ${hatchSize}L${hatchSize} 0`} stroke="#e686a4" strokeWidth={hatchSize / 7} /></pattern>
            <pattern id="stock-missing" width={hatchSize} height={hatchSize} patternUnits="userSpaceOnUse"><path d={`M0 ${hatchSize / 2}H${hatchSize}`} stroke="#f3588b" strokeWidth={hatchSize / 4} /></pattern>
            <pattern id="stock-overcut" width={hatchSize} height={hatchSize} patternUnits="userSpaceOnUse"><path d={`M0 0L${hatchSize} ${hatchSize}M0 ${hatchSize}L${hatchSize} 0`} stroke="#b384eb" strokeWidth={hatchSize / 7} /></pattern>
          </defs>
          <rect x={camera.x - camera.span / 2} y={-camera.y - camera.span * .35} width={camera.span} height={camera.span * .7} className="drawing-background" />
          {grid && <rect x={camera.x - camera.span / 2} y={-camera.y - camera.span * .35} width={camera.span} height={camera.span * .7} fill="url(#grid)" />}
          <path d={pathData([{ points: page }])} className="page-outline" vectorEffect="non-scaling-stroke" />
          {components.filter(component => !hidden.has(component.id)).map(component => <path key={component.id}
            data-component={component.id} d={pathData(component.rings)} fillRule="evenodd"
            className={`artwork-region ${job.selected_region_ids.includes(component.id) ? 'included' : ''} ${inspected === component.id ? 'inspected' : ''}`}
            vectorEffect="non-scaling-stroke" onClick={() => onInspect(component.id)}>
            <title>{`${component.label} · ${job.selected_region_ids.includes(component.id) ? 'Included for machining' : 'Excluded from machining'}`}</title>
          </path>)}
          {stockPaths.map(region => <path key={region.key} data-stock-region={region.key} d={region.path} fillRule="evenodd" className={`stock-region ${region.key}`} vectorEffect="non-scaling-stroke"><title>{`${stockLayers[region.key].label} · engine-calculated slice`}</title></path>)}
          {showMotions && motionGroups.filter(group => showTravel || !group.travel).map(group => <path key={`${group.key}:${group.chunk}`} data-motion-group={group.key}
            d={group.path} className={`plan-motion ${group.travel ? 'travel' : group.tool === job.operation.vbit_id ? 'vbit cutting' : 'endmill cutting'}`} vectorEffect="non-scaling-stroke"><title>{`${group.key} · recorded XYZ motions projected to XY`}</title></path>)}
          <g className="workpiece-axes" strokeWidth="1.5" fill="none">
            <path d="M0 -8V0H8" vectorEffect="non-scaling-stroke" />
            <circle cx="0" cy="0" r=".55" vectorEffect="non-scaling-stroke" />
          </g>
          {verificationFinding && <g data-verification-finding={verificationFinding.code} className={`verification-location ${verificationFinding.status}`}>
            <title>{verificationFinding.code} · {verificationFinding.message}</title>
            {verificationFinding.cell && <rect x={verificationFinding.cell.min.x} y={-verificationFinding.cell.max.y} width={verificationFinding.cell.max.x - verificationFinding.cell.min.x} height={verificationFinding.cell.max.y - verificationFinding.cell.min.y} vectorEffect="non-scaling-stroke" />}
            <circle cx={verificationFinding.location.x} cy={-verificationFinding.location.y} r={camera.span / 90} vectorEffect="non-scaling-stroke" />
          </g>}
          <g className="axis-labels" fontSize="2.2"><text x="9" y=".8">X</text><text x="-.8" y="-9">Y</text><text x="1.2" y="3">0, 0</text></g>
        </svg>
        <div className="drawing-legend">{stockInfo ? stockPaths.map(region => <span className="stock-legend-item" key={region.key}><span className={`stock-swatch ${region.key}`} />{stockLayers[region.key].label}</span>) : <><span className="legend-swatch included" /> Included region <span className="legend-swatch hole" /> Preserved hole <span className="legend-swatch inspected" /> Inspected</>}{showMotions && <>
          {motionGroups.some(g => !g.travel && g.tool === job.operation.endmill_id) && <><span className="legend-swatch endmill-motion" /> Endmill</>}
          {motionGroups.some(g => !g.travel && g.tool === job.operation.vbit_id) && <><span className="legend-swatch vbit-motion" /> V-bit</>}
          {showTravel && motionGroups.some(g => g.travel) && <><span className="legend-swatch travel-motion" /> Travel</>}
        </>}</div>
      </> : <div className="empty-viewport"><span className="empty-symbol">⌗</span><h2>Artwork needs inspection</h2><p>Connect the local Rust service to normalize this source or changed import tolerance. Your draft is retained.</p></div>}
    </div>
    <div className="viewport-status"><span>mm <span className="divider">|</span> Stock top Z = 0</span><span>{display ? `Grid 10 mm · import tolerance ${display.geometryToleranceMm} mm` : 'No geometry available'}</span></div>
  </section>;
}
