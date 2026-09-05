import type { InspectionView, ToolFilter, useInspection } from '../service/useInspection';
import { stockLayers, type StockLayer } from '../contracts/stock';

export type Inspection = ReturnType<typeof useInspection>;
const viewLabels = { motions: 'Recorded motions', endmill: 'After endmill', combined: 'After both tools' };
const number = (value: number) => new Intl.NumberFormat(undefined, { maximumSignificantDigits: 6 }).format(value);
export function InspectionToolbar({ inspection: i }: { inspection: Inspection }) {
  return <div className="inspection-toolbar" aria-label="Plan inspection controls">
    <label>View<select aria-label="Inspection view" disabled={!i.current} value={i.view} onChange={e => i.setView(e.target.value as InspectionView)}>{i.views.map(view => <option key={view} value={view}>{viewLabels[view]}</option>)}</select></label>
    {i.view !== 'motions' && <label>Depth below stock top<select aria-label="Stock slice depth" disabled={!i.current} value={i.selected?.id ?? ''} onChange={e => i.setSlice(e.target.value)}>{i.slices.map(s => <option key={s.id} value={s.id}>{number(s.depthMm)} mm · Z {number(-s.depthMm)}</option>)}</select></label>}
    <label>Toolpaths<select aria-label="Toolpath filter" disabled={!i.current || i.view === 'endmill'} value={i.tool} onChange={e => i.setTool(e.target.value as ToolFilter)}><option value="all">Both tools</option><option value="endmill">Endmill</option><option value="vbit">V-bit</option></select></label>
    <label>Path layer<select aria-label="Toolpath layer" disabled={!i.current} value={i.layer} onChange={e => i.setLayer(e.target.value)}><option value="all">All layers</option>{i.layers.map(layer => <option key={layer} value={layer}>Layer {layer + 1}</option>)}</select></label>
  </div>;
}
export function StockInspector({ inspection: i }: { inspection: Inspection }) {
  if (!i.selected || i.view === 'motions') return null;
  const slice = i.response?.slice;
  return <section className="inspector-group stock-inspector"><h2>Stock slice · {number(i.selected.depthMm)} mm</h2>
    <p>{viewLabels[i.view]} at stock-top Z {number(-i.selected.depthMm)} mm.</p>
    <p className="hint">Fixed-depth bounds from recorded motions. These slices do not establish continuous-volume verification. Toolpath filters affect the overlay only.</p>
    {!i.current ? <p className="inline-warning">The plan is stale. Stock geometry is hidden until a current plan is generated.</p> : <>
      {i.pending && <p role="status">Loading stock slice…</p>}
      {i.error && <><p role="alert" className="inline-warning">{i.error}</p><button onClick={i.retry}>Retry stock slice</button></>}
      {i.selected.unavailableReason && <p role="status" className="inline-warning">{i.selected.unavailableReason}</p>}
      <fieldset className="stock-overlays"><legend>Stock overlays</legend>{i.selected.regions.map(region => <label key={region.key} title={stockLayers[region.key].description}><input type="checkbox" checked={i.visible.has(region.key)} disabled={!slice?.geometry} onChange={() => i.toggleLayer(region.key)} /><span className={`stock-swatch ${region.key}`} />{stockLayers[region.key].label}</label>)}</fieldset>
      <table className="stock-metrics"><caption>Slice areas · mm²</caption><thead><tr><th scope="col">Region</th><th scope="col">Area</th></tr></thead><tbody>{i.selected.regions.map(region => <tr key={region.key}><th scope="row">{stockLayers[region.key].label}</th><td><button aria-label={`Inspect ${stockLayers[region.key].label}`} disabled={!slice?.geometry || !region.bounds} onClick={() => i.inspectRegion(region.key)}>{number(region.areaMm2)}</button></td></tr>)}</tbody></table>
      {i.selected.diagnostics.map((d, index) => {
        const target: StockLayer | undefined = d.code === 'INCOMPLETE_FLOOR_COVERAGE' ? 'missingFloor' : d.code === 'SWEEP_OVERCUT_UNCERTAINTY' ? 'possibleOvercut' : undefined;
        return <div className="slice-finding" key={index}><p><strong>{d.code}</strong> · {d.message}</p>{target && <button disabled={!slice?.geometry || !i.selected?.regions.find(r => r.key === target)?.bounds} onClick={() => i.inspectRegion(target)}>Locate {stockLayers[target].label.toLowerCase()}</button>}</div>;
      })}
      {i.selected.omittedDiagnostics > 0 && <p className="hint">{i.selected.omittedDiagnostics} more slice diagnostics are omitted from this summary.</p>}
      <details><summary>Bounds and interpretation</summary><dl><dt>Contributing motions</dt><dd>{i.selected.contributingMotionCount}</dd><dt>Radial approximation error</dt><dd>{number(i.selected.capsuleRadialErrorMm)} mm</dd><dt>Slice outcome</dt><dd>{i.selected.status ?? 'No separate per-slice verdict'}</dd></dl>{i.selected.regions.map(region => <p className="hint" key={region.key}><strong>{stockLayers[region.key].label}:</strong> {stockLayers[region.key].description} Geometry tolerance: {number(region.geometryToleranceMm)} mm.</p>)}</details>
    </>}
  </section>;
}
