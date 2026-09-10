// Profile settings editor (D3 + E3): explicit contour selection with
// per-contour side, heights, stepdown, through allowance, cut direction, the
// milling assignment, radial finishing, tabs, the depth entry (plunge or
// ramp), tangent line/arc leads, and automatic or anchor starts. Submitted
// whole through the engine's strict UpdateSettings command inside the kind
// envelope — the canonical job is never edited in place, and the contour
// list comes from the engine's catalogue projection.
import { useEffect, useState } from 'react';
import type { ReactElement } from 'react';
import type { ContourInfo, SequenceDocument, SequenceService } from '../contracts/sequence';

type ProfileSettings = {
  contours: { contour_id: string; side: string; traversal?: string | null }[];
  assignment: Record<string, unknown>;
  top: unknown;
  bottom: unknown;
  stepdown_mm?: number | null;
  through_cut_allowance_mm?: number | null;
  direction?: string | null;
  order?: string;
  [key: string]: unknown;
};

type EntrySpec = { kind?: string; [key: string]: unknown };
type LeadSpec = { kind?: string; [key: string]: unknown };
type StartSpec = { kind?: string; [key: string]: unknown };
type TabsSpec = { height_mm?: number | null; width_mm?: number | null; [key: string]: unknown };
type FinishSpec = { enabled?: boolean; [key: string]: unknown };

function parseNumber(text: string): number | null {
  const trimmed = text.trim();
  if (!trimmed) return null;
  const value = Number(trimmed);
  return Number.isFinite(value) ? value : null;
}

function numberField(
  label: string,
  value: number | null | undefined,
  onChange: (text: string) => void,
): ReactElement {
  return <label className="sequence-field">
    <span>{label}</span>
    <input
      inputMode="decimal"
      value={value === null || value === undefined ? '' : String(value)}
      aria-label={label}
      onChange={event => onChange(event.target.value)}
    />
  </label>;
}

export function ProfileSettingsEditor({
  service,
  job,
  operationId,
  settings,
  faceOperations,
  busy,
  onApplied,
}: {
  service: SequenceService;
  job: unknown;
  operationId: string;
  settings: ProfileSettings;
  /** Preceding face operations selectable as the profile's top plane. */
  faceOperations: { id: string }[];
  busy: boolean;
  onApplied: (document: SequenceDocument, message: string) => void;
}): ReactElement {
  const [draft, setDraft] = useState(() => structuredClone(settings));
  const [contours, setContours] = useState<ContourInfo[] | null>(null);
  const [error, setError] = useState('');

  useEffect(() => {
    service.contours(job).then(
      result => setContours(result.contours),
      cause => setError(String(cause)),
    );
  }, [service, job]);

  const selected = new Map(draft.contours.map(entry => [entry.contour_id, entry]));
  const toggle = (contour: ContourInfo) => {
    const next = new Map(selected);
    if (next.has(contour.id)) {
      next.delete(contour.id);
    } else {
      next.set(contour.id, {
        contour_id: contour.id,
        side: contour.suggestedSide,
        traversal: contour.suggestedSide === 'on' ? 'forward' : null,
      });
    }
    setDraft({ ...draft, contours: [...next.values()] });
  };
  const setSide = (id: string, side: string) => {
    const next = draft.contours.map(entry =>
      entry.contour_id === id
        ? { ...entry, side, traversal: side === 'on' ? (entry.traversal ?? 'forward') : null }
        : entry);
    setDraft({ ...draft, contours: next });
  };

  const rawAssignment = draft.assignment as Record<string, number | string | null | undefined>;
  const assignmentNumber = (key: string, label: string) =>
    numberField(label, (rawAssignment[key] as number | null | undefined) ?? null, text => {
      setDraft({ ...draft, assignment: { ...draft.assignment, [key]: parseNumber(text) } });
    });

  // Top reference: the original stock top or a preceding face plane; bottom
  // is measured below the resolved top (plan section 6.3).
  const topRef = (draft.top as { reference?: { kind?: string; operation_id?: string } })?.reference;
  const topKind = topRef?.kind === 'face_result' ? 'face' : 'stock';
  const topFace = topRef?.operation_id ?? faceOperations[0]?.id ?? '';
  const bottomRef = (draft.bottom as { reference?: { kind?: string }; offset_mm?: number }) ?? {};
  const bottomDepth = -(bottomRef.offset_mm ?? 0);

  // E3 entry/lead/start/tabs/finish specs ride along in the draft; each
  // control replaces one spec object wholesale so the engine always parses
  // a complete, self-describing shape.
  const entry = (draft.entry ?? { kind: 'plunge' }) as EntrySpec;
  const leadIn = (draft.lead_in ?? { kind: 'none' }) as LeadSpec;
  const leadOut = (draft.lead_out ?? { kind: 'none' }) as LeadSpec;
  const start = (draft.start ?? { kind: 'automatic' }) as StartSpec;
  const tabs = (draft.tabs ?? null) as TabsSpec | null;
  const finish = (draft.finish ?? { enabled: false }) as FinishSpec;

  const leadEditor = (label: string, spec: LeadSpec, key: 'lead_in' | 'lead_out'): ReactElement => {
    const kind = spec.kind ?? 'none';
    return <div className="sequence-lead-editor">
      <label className="sequence-field">
        <span>{label}</span>
        <select aria-label={label} value={kind}
          onChange={event => {
            const next = event.target.value;
            setDraft({
              ...draft,
              [key]: next === 'none' ? { kind: 'none' }
                : next === 'tangent_line' ? { kind: 'tangent_line', length_mm: null, feed_mm_min: null }
                  : { kind: 'tangent_arc', radius_mm: null, sweep_deg: null, feed_mm_min: null },
            });
          }}>
          <option value="none">None</option>
          <option value="tangent_line">Tangent line</option>
          <option value="tangent_arc">Tangent arc</option>
        </select>
      </label>
      {kind === 'tangent_line' && <>
        {numberField('Lead length (mm)', spec.length_mm as number | null | undefined, text => {
          setDraft({ ...draft, [key]: { ...spec, length_mm: parseNumber(text) } });
        })}
        {numberField('Lead feed (mm/min)', spec.feed_mm_min as number | null | undefined, text => {
          setDraft({ ...draft, [key]: { ...spec, feed_mm_min: parseNumber(text) } });
        })}
      </>}
      {kind === 'tangent_arc' && <>
        {numberField('Arc radius (mm)', spec.radius_mm as number | null | undefined, text => {
          setDraft({ ...draft, [key]: { ...spec, radius_mm: parseNumber(text) } });
        })}
        {numberField('Arc sweep (deg)', spec.sweep_deg as number | null | undefined, text => {
          setDraft({ ...draft, [key]: { ...spec, sweep_deg: parseNumber(text) } });
        })}
        {numberField('Lead feed (mm/min)', spec.feed_mm_min as number | null | undefined, text => {
          setDraft({ ...draft, [key]: { ...spec, feed_mm_min: parseNumber(text) } });
        })}
      </>}
    </div>;
  };

  // Anchor starts bind to the source fingerprint of their contour; the
  // catalogue entry of the first selected contour supplies it.
  const anchorContourId = (start.contour_id as string | undefined) ?? draft.contours[0]?.contour_id ?? '';
  const anchorContour = contours?.find(contour => contour.id === anchorContourId) ?? null;
  const anchorFingerprint =
    (start.source_geometry_fingerprint as string | undefined) ?? anchorContour?.sourceFingerprint ?? '';

  async function apply() {
    setError('');
    try {
      const faceTop = faceOperations.some(face => face.id === topFace);
      const next = {
        ...draft,
        top: faceTop
          ? { reference: { kind: 'face_result', operation_id: topFace }, offset_mm: 0 }
          : { reference: { kind: 'stock_top' }, offset_mm: 0 },
        bottom: { reference: { kind: 'operation_top' }, offset_mm: -Math.max(bottomDepth, 0) },
      };
      const document = await service.updateSettings(job, operationId, {
        kind: 'profile',
        settings: next,
      });
      onApplied(document, `Profile settings for ${operationId} updated through the engine.`);
    } catch (cause) {
      setError(String(cause));
    }
  }

  return <div className="sequence-profile-editor">
    <h3>Profile settings · {operationId}</h3>
    {contours === null && !error && <p className="hint" role="status">loading contour catalogue…</p>}
    {contours !== null && <ul className="sequence-contour-list">
      {contours.map(contour => <li key={contour.id} className={selected.has(contour.id) ? 'selected' : ''}>
        <label>
          <input
            type="checkbox"
            checked={selected.has(contour.id)}
            aria-label={`Select contour ${contour.id}`}
            onChange={() => toggle(contour)}
          />
          <span>{contour.id}</span>
          <small> · {contour.role}{contour.parentContourId ? ` in ${contour.parentContourId}` : ''} · {contour.perimeterMm.toFixed(1)} mm</small>
        </label>
        {selected.has(contour.id) && <label className="sequence-field">
          <span>Side</span>
          <select
            aria-label={`Compensation side for ${contour.id}`}
            value={selected.get(contour.id)?.side ?? contour.suggestedSide}
            onChange={event => setSide(contour.id, event.target.value)}
          >
            <option value="outside">Outside</option>
            <option value="inside">Inside</option>
            <option value="on">On</option>
          </select>
        </label>}
      </li>)}
    </ul>}
    <div className="sequence-fields-grid">
      <label className="sequence-field">
        <span>Top</span>
        <select aria-label="Top height reference" value={topKind === 'face' ? `face:${topFace}` : 'stock'}
          onChange={event => {
            const value = event.target.value;
            setDraft({
              ...draft,
              top: value === 'stock'
                ? { reference: { kind: 'stock_top' }, offset_mm: 0 }
                : { reference: { kind: 'face_result', operation_id: value.slice(5) }, offset_mm: 0 },
            });
          }}>
          <option value="stock">Stock top</option>
          {faceOperations.map(face => <option key={face.id} value={`face:${face.id}`}>Face plane of {face.id}</option>)}
        </select>
      </label>
      {numberField('Depth below top (mm)', bottomDepth, text => {
        const depth = parseNumber(text) ?? 0;
        setDraft({ ...draft, bottom: { reference: { kind: 'operation_top' }, offset_mm: -Math.max(depth, 0) } });
      })}
      {numberField('Stepdown (mm)', draft.stepdown_mm ?? null, text => setDraft({ ...draft, stepdown_mm: parseNumber(text) }))}
      {numberField('Through allowance (mm)', draft.through_cut_allowance_mm ?? null, text => setDraft({ ...draft, through_cut_allowance_mm: parseNumber(text) }))}
      <label className="sequence-field">
        <span>Direction</span>
        <select aria-label="Cut direction" value={draft.direction ?? ''}
          onChange={event => setDraft({ ...draft, direction: event.target.value || null })}>
          <option value="">Unset</option>
          <option value="climb">Climb</option>
          <option value="conventional">Conventional</option>
        </select>
      </label>
      <label className="sequence-field">
        <span>Spindle direction</span>
        <select aria-label="Spindle rotation" value={String(rawAssignment.spindle_direction ?? '')}
          onChange={event => setDraft({
            ...draft,
            assignment: {
              ...draft.assignment,
              spindle_direction: event.target.value || null,
            },
          })}>
          <option value="">Unset</option>
          <option value="clockwise">Clockwise</option>
          <option value="counterclockwise">Counterclockwise</option>
        </select>
      </label>
      {assignmentNumber('spindle_rpm', 'Spindle RPM')}
      {assignmentNumber('cutting_feed_mm_min', 'Cutting feed (mm/min)')}
      {assignmentNumber('plunge_feed_mm_min', 'Plunge feed (mm/min)')}
      {assignmentNumber('max_stepdown_mm', 'Max stepdown (mm)')}
    </div>
    <div className="sequence-fields-grid">
      <label className="sequence-field">
        <span>Depth entry</span>
        <select aria-label="Depth entry" value={entry.kind ?? 'plunge'}
          onChange={event => setDraft({
            ...draft,
            entry: event.target.value === 'ramp'
              ? { kind: 'ramp', max_angle_deg: null, feed_mm_min: null }
              : { kind: 'plunge' },
          })}>
          <option value="plunge">Plunge</option>
          <option value="ramp">Contour ramp</option>
        </select>
      </label>
      {entry.kind === 'ramp' && <>
        {numberField('Ramp angle (deg)', entry.max_angle_deg as number | null | undefined, text => {
          setDraft({ ...draft, entry: { ...entry, max_angle_deg: parseNumber(text) } });
        })}
        {numberField('Ramp feed (mm/min)', entry.feed_mm_min as number | null | undefined, text => {
          setDraft({ ...draft, entry: { ...entry, feed_mm_min: parseNumber(text) } });
        })}
      </>}
      {leadEditor('Lead-in', leadIn, 'lead_in')}
      {leadEditor('Lead-out', leadOut, 'lead_out')}
    </div>
    <div className="sequence-fields-grid">
      <label className="sequence-field">
        <span>Start</span>
        <select aria-label="Start selection" value={start.kind ?? 'automatic'}
          onChange={event => setDraft({
            ...draft,
            start: event.target.value === 'anchor'
              ? {
                kind: 'anchor',
                contour_id: anchorContourId,
                source_geometry_fingerprint: anchorFingerprint,
                fraction_along_source_contour: 0,
              }
              : { kind: 'automatic' },
          })}>
          <option value="automatic">Automatic</option>
          <option value="anchor" disabled={!anchorContour}>Anchor</option>
        </select>
      </label>
      {start.kind === 'anchor' && <label className="sequence-field">
        <span>Anchor contour</span>
        <select aria-label="Anchor contour" value={anchorContourId}
          onChange={event => {
            const contour = contours?.find(entry => entry.id === event.target.value);
            setDraft({
              ...draft,
              start: {
                ...start,
                kind: 'anchor',
                contour_id: event.target.value,
                source_geometry_fingerprint: contour?.sourceFingerprint ?? null,
              },
            });
          }}>
          {draft.contours.map(entry => <option key={entry.contour_id} value={entry.contour_id}>{entry.contour_id}</option>)}
        </select>
      </label>}
      {start.kind === 'anchor' && numberField('Start fraction along contour', start.fraction_along_source_contour as number | null | undefined, text => {
        setDraft({ ...draft, start: { ...start, kind: 'anchor', fraction_along_source_contour: parseNumber(text) } });
      })}
    </div>
    <div className="sequence-fields-grid">
      <label className="sequence-field">
        <span>Tabs</span>
        <select aria-label="Tabs" value={tabs ? 'rectangular' : 'none'}
          onChange={event => setDraft({
            ...draft,
            tabs: event.target.value === 'rectangular'
              ? {
                height_mm: null,
                width_mm: null,
                shape: 'rectangular',
                placement: { kind: 'automatic', count: null, spacing_mm: null },
              }
              : null,
          })}>
          <option value="none">None</option>
          <option value="rectangular">Rectangular (automatic)</option>
        </select>
      </label>
      {tabs && <>
        {numberField('Tab height (mm)', tabs.height_mm ?? null, text => {
          setDraft({ ...draft, tabs: { ...tabs, height_mm: parseNumber(text) } });
        })}
        {numberField('Tab width (mm)', tabs.width_mm ?? null, text => {
          setDraft({ ...draft, tabs: { ...tabs, width_mm: parseNumber(text) } });
        })}
        {numberField('Tab count', (tabs.placement as { count?: number | null })?.count ?? null, text => {
          const count = parseNumber(text);
          setDraft({
            ...draft,
            tabs: {
              ...tabs,
              placement: {
                kind: 'automatic',
                count: count === null ? null : Math.max(0, Math.round(count)),
                spacing_mm: null,
              },
            },
          });
        })}
      </>}
    </div>
    <div className="sequence-fields-grid">
      <label className="sequence-field">
        <span>Finish</span>
        <select aria-label="Finishing" value={finish.enabled ? 'on' : 'off'}
          onChange={event => setDraft({
            ...draft,
            finish: event.target.value === 'on'
              ? { enabled: true, radial_allowance_mm: null, feed_mm_min: null }
              : { enabled: false },
          })}>
          <option value="off">Single rough pass</option>
          <option value="on">Rough + finish</option>
        </select>
      </label>
      {finish.enabled && <>
        {numberField('Radial allowance (mm)', finish.radial_allowance_mm as number | null | undefined, text => {
          setDraft({ ...draft, finish: { ...finish, enabled: true, radial_allowance_mm: parseNumber(text) } });
        })}
        {numberField('Finish feed (mm/min)', finish.feed_mm_min as number | null | undefined, text => {
          setDraft({ ...draft, finish: { ...finish, enabled: true, feed_mm_min: parseNumber(text) } });
        })}
      </>}
    </div>
    <div className="inline-actions">
      <button className="primary" disabled={busy} onClick={() => void apply()}>Apply profile settings</button>
      {error && <p role="alert" className="inline-warning">{error}</p>}
    </div>
    <p className="hint">Contours and sides are explicit per selection; entries, leads, tabs and finishing are parsed and validated by the engine.</p>
  </div>;
}
