import { useEffect, useRef } from 'react';
import type { LibraryTool, ToolSlot } from '../contracts/library';
import type { DraftLibraryReview } from '../state/library';

export function LibraryApplyPanel({ tool, slot, presetId, onPreset, review, current, jobAvailable, busy, onReview, onApply }: {
  tool:LibraryTool; slot:ToolSlot; presetId:string|null|undefined; onPreset:(id:string|null|undefined)=>void;
  review:DraftLibraryReview|null; current:boolean; jobAvailable:boolean; busy:boolean;
  onReview:()=>void; onApply:()=>void;
}) {
  const reviewPanel = useRef<HTMLElement>(null);
  useEffect(() => { if (review) { reviewPanel.current?.focus(); reviewPanel.current?.scrollIntoView({block:'nearest'}); } },[review]);
  const changes = review?.changes ?? [];
  const slotName = slot === 'endmill' ? 'endmill' : 'V-bit';
  return <section className="library-apply" aria-label="Apply library selection">
    <h3>Load into this job’s {slotName}</h3>
    <p><strong>1. Tool selected: {tool.name}</strong></p><p className="hint">Selecting a record only opens it for review. Your job changes after the final Apply button.</p>
    <label className="field"><span>2. Cutting preset to apply</span><select value={presetId === undefined ? '' : presetId === null ? 'geometry' : `preset:${presetId}`} disabled={busy}
      onChange={e => onPreset(e.target.value === '' ? undefined : e.target.value === 'geometry' ? null : e.target.value.slice(7))}>
      <option value="">Choose a cutting preset or geometry only…</option>
      {tool.cutting_presets.map(p => <option key={p.id} value={`preset:${p.id}`}>{p.name}</option>)}
      <option value="geometry">Geometry only — clear all cutting settings</option>
    </select></label>
    <p className="hint">{presetId === undefined ? 'Choose a preset to load spindle speed, feeds, stepdown and stepover with the cutter.'
      : presetId === null ? 'Geometry only clears spindle speed, cutting feed, plunge feed, stepdown and stepover. Those values will need setup before planning.'
      : 'This preset replaces all five cutting values, including any blanks.'}</p>
    {tool.geometry.kind !== slot && <p className="inline-warning">This cutter does not match the {slotName} slot. Choose a matching cutter or change the job tool slot.</p>}
    {!jobAvailable && <p className="hint">Open a job containing this tool slot before reviewing a selection.</p>}
    {jobAvailable && <p className="hint">Tools can be loaded while setup is unfinished. Applying replaces this tool’s fields; other unfinished settings are kept and checked before planning.</p>}
    <button className="primary" disabled={busy || !jobAvailable || tool.geometry.kind !== slot || presetId === undefined} onClick={onReview}>3. Review changes to {slotName}</button>
    {review && <section ref={reviewPanel} tabIndex={-1} aria-label="Review job changes"><h3>Review before applying</h3>
      <p>{review.toolName} · {review.presetName ?? 'Geometry only'}</p>
      {changes.length > 0 && <table className="stock-metrics library-changes"><thead><tr><th>Setting</th><th>Current job</th><th>After applying</th></tr></thead><tbody>{changes.map(c => <tr key={c.label}><th>{c.label}</th><td>{c.before}</td><td>{c.after}</td></tr>)}</tbody></table>}
      <p className="hint">Job tool IDs and machine mapping stay the same. Applying changes can be undone and requires a new plan.</p>
      {!changes.length && <p>These settings already match your job. Confirm to record this library selection.</p>}
      {!current && <p role="alert">The job or library changed. Review the selection again.</p>}
      <button className="primary wide" disabled={busy || !current} onClick={onApply}>{changes.length ? `Apply to job’s ${slotName}` : `Use matching settings for ${slotName}`}</button>
    </section>}
  </section>;
}
