import { useMemo, useRef, useState } from 'react';
import type { ArtworkDisplay } from '../contracts/service';
import type { Job, Point } from '../contracts/job';

interface Props {
  display: ArtworkDisplay | null; job: Job; inspected: string | null;
  onInspect: (id: string) => void; hidden: Set<string>;
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
export function Viewport({ display, job, inspected, onInspect, hidden }: Props) {
  const [camera, setCamera] = useState({ x: 50, y: 30, span: 125 });
  const [grid, setGrid] = useState(true);
  const drag = useRef<{ x: number; y: number; camera: typeof camera } | null>(null);
  const components = useMemo(() => display?.components.map(component => ({ ...component,
    rings: component.rings.map(ring => ({ ...ring, points: ring.points.map(point => placePoint(point, job.import.placement)) })),
  })) ?? [], [display, job.import.placement]);
  const page = useMemo(() => display ? [{ x: 0, y: 0 }, { x: display.widthMm, y: 0 },
    { x: display.widthMm, y: display.heightMm }, { x: 0, y: display.heightMm }].map(point => placePoint(point, job.import.placement)) : [], [display, job.import.placement]);
  function fit(selection = false) {
    const points = selection ? components.filter(component => component.id === inspected).flatMap(component => component.rings.flatMap(ring => ring.points)) : page;
    if (!points.length) return;
    const xs = points.map(point => point.x), ys = points.map(point => point.y);
    const minX = Math.min(...xs), maxX = Math.max(...xs), minY = Math.min(...ys), maxY = Math.max(...ys);
    setCamera({ x: (minX + maxX) / 2, y: (minY + maxY) / 2, span: Math.max(maxX - minX, (maxY - minY) / .7, 1) * 1.2 });
  }
  function zoom(factor: number) { setCamera(current => ({ ...current, span: Math.min(100000, Math.max(.01, current.span * factor)) })); }
  return <section className="viewport" aria-label="Artwork viewport">
    <div className="viewport-toolbar">
      <span className="view-title"><span className="view-dot" /> Artwork <span className="subtle">/ Top view</span></span>
      <div className="button-group">
        <button onClick={() => fit()}>Fit job</button>
        <button onClick={() => fit(true)} disabled={!inspected}>Fit inspected</button>
        <button onClick={() => zoom(1.25)} aria-label="Zoom out">−</button>
        <button onClick={() => zoom(.8)} aria-label="Zoom in">+</button>
        <button aria-pressed={grid} onClick={() => setGrid(!grid)}>Grid</button>
      </div>
    </div>
    <div className="drawing-wrap">
      {display ? <>
        <div className="drawing-note">SOURCE GEOMETRY <span>·</span> no planned cuts</div>
        <svg className="drawing" role="group" aria-label="Top view of normalized artwork. Inspect shapes with the source list. Arrow keys pan; plus and minus zoom; Home fits the job."
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
          </defs>
          <rect x={camera.x - camera.span / 2} y={-camera.y - camera.span * .35} width={camera.span} height={camera.span * .7} className="drawing-background" />
          {grid && <rect x={camera.x - camera.span / 2} y={-camera.y - camera.span * .35} width={camera.span} height={camera.span * .7} fill="url(#grid)" />}
          <path d={pathData([{ points: page }])} className="page-outline" vectorEffect="non-scaling-stroke" />
          {components.filter(component => !hidden.has(component.id)).map(component => <path key={component.id}
            data-component={component.id} d={pathData(component.rings)} fillRule="evenodd"
            className={`artwork-region ${job.selected_region_ids.includes(component.id) ? 'included' : ''} ${inspected === component.id ? 'inspected' : ''}`}
            vectorEffect="non-scaling-stroke" onClick={() => onInspect(component.id)}>
            <title>{component.label} · {job.selected_region_ids.includes(component.id) ? 'Included for machining' : 'Excluded from machining'}</title>
          </path>)}
          <g className="workpiece-axes" strokeWidth="1.5" fill="none">
            <path d="M0 -8V0H8" vectorEffect="non-scaling-stroke" />
            <circle cx="0" cy="0" r=".55" vectorEffect="non-scaling-stroke" />
          </g>
          <g className="axis-labels" fontSize="2.2"><text x="9" y=".8">X</text><text x="-.8" y="-9">Y</text><text x="1.2" y="3">0, 0</text></g>
        </svg>
        <div className="drawing-legend"><span className="legend-swatch included" /> Included region <span className="legend-swatch hole" /> Preserved hole <span className="legend-swatch inspected" /> Inspected</div>
      </> : <div className="empty-viewport"><span className="empty-symbol">⌗</span><h2>Artwork needs inspection</h2><p>Connect the local Rust service to normalize this source or changed import tolerance. Your draft is retained.</p></div>}
    </div>
    <div className="viewport-status"><span>mm <span className="divider">|</span> Stock top Z = 0</span><span>{display ? `Grid 10 mm · captured tolerance ${display.geometryToleranceMm} mm` : 'No geometry available'}</span></div>
  </section>;
}
