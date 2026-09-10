// Face settings editor: numeric controls for the coverage rectangle,
// margins, depths and raster, submitted through the engine's strict
// UpdateSettings command — the canonical job is never edited in place.
import { useState } from 'react';
import type { ReactElement } from 'react';
import type { SequenceDocument, SequenceService } from '../contracts/sequence';

type FaceSettings = {
  area: { kind: string; rect?: Record<string, number> };
  margins: Record<string, number | null | undefined>;
  entry_overrun_mm?: number | null;
  exit_overrun_mm?: number | null;
  stepdown_mm?: number | null;
  stepover_mm?: number | null;
  pass_angle_deg?: number | null;
  pattern?: string;
  [key: string]: unknown;
};

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

function parseNumber(text: string): number | null {
  const trimmed = text.trim();
  if (!trimmed) return null;
  const value = Number(trimmed);
  return Number.isFinite(value) ? value : null;
}

export function FaceSettingsEditor({
  service,
  job,
  operationId,
  settings,
  busy,
  onApplied,
}: {
  service: SequenceService;
  job: unknown;
  operationId: string;
  settings: FaceSettings;
  busy: boolean;
  onApplied: (document: SequenceDocument, message: string) => void;
}) {
  const [draft, setDraft] = useState(() => ({ ...settings, margins: { ...settings.margins } }));
  const [error, setError] = useState('');
  const rect = (draft.area.rect ?? { min_x_mm: 0, min_y_mm: 0, width_mm: 10, length_mm: 10 }) as Record<string, number>;
  const rectField = (key: string, label: string) =>
    numberField(label, rect[key], text => {
      const next = { ...rect };
      const value = parseNumber(text);
      if (value !== null) next[key] = value;
      setDraft({ ...draft, area: { kind: 'rectangle', rect: next } });
    });
  const marginField = (key: string, label: string) =>
    numberField(label, draft.margins?.[key] ?? null, text => {
      const margins = { ...draft.margins, [key]: parseNumber(text) };
      setDraft({ ...draft, margins });
    });
  const simpleField = (key: string, label: string) =>
    numberField(label, (draft as unknown as Record<string, number | null | undefined>)[key] ?? null, text => {
      setDraft({ ...draft, [key]: parseNumber(text) });
    });

  async function apply() {
    setError('');
    try {
      // OperationSettings is adjacently tagged on the wire: the engine
      // accepts the whole face settings only inside a kind envelope.
      const document = await service.updateSettings(job, operationId, {
        kind: 'face',
        settings: {
          ...draft,
          area: draft.area.kind === 'entire_stock'
            ? { kind: 'entire_stock' }
            : { kind: 'rectangle', rect: draft.area.rect },
          pattern: draft.pattern === 'one_way' ? 'one_way' : 'zig_zag',
        },
      });
      onApplied(document, `Face settings for ${operationId} updated through the engine.`);
    } catch (cause) {
      setError(String(cause));
    }
  }

  return <div className="sequence-face-editor">
    <h3>Face settings · {operationId}</h3>
    <div className="sequence-fields-grid">
      {rectField('min_x_mm', 'Rectangle min X (mm)')}
      {rectField('min_y_mm', 'Rectangle min Y (mm)')}
      {rectField('width_mm', 'Rectangle width (mm)')}
      {rectField('length_mm', 'Rectangle length (mm)')}
      {marginField('min_x_mm', 'Margin min X (mm)')}
      {marginField('max_x_mm', 'Margin max X (mm)')}
      {marginField('min_y_mm', 'Margin min Y (mm)')}
      {marginField('max_y_mm', 'Margin max Y (mm)')}
      {simpleField('stepdown_mm', 'Stepdown (mm)')}
      {simpleField('stepover_mm', 'Stepover (mm)')}
      {simpleField('entry_overrun_mm', 'Entry overrun (mm)')}
      {simpleField('exit_overrun_mm', 'Exit overrun (mm)')}
      {numberField('Pass angle (0 or 90)', draft.pass_angle_deg ?? null, text => {
        setDraft({ ...draft, pass_angle_deg: parseNumber(text) });
      })}
      <label className="sequence-field">
        <span>Pattern</span>
        <select
          aria-label="Pattern"
          value={draft.pattern === 'one_way' ? 'one_way' : 'zig_zag'}
          onChange={event => setDraft({ ...draft, pattern: event.target.value })}
        >
          <option value="zig_zag">Zigzag</option>
          <option value="one_way">One way</option>
        </select>
      </label>
    </div>
    <div className="inline-actions">
      <button className="primary" disabled={busy} onClick={() => void apply()}>Apply face settings</button>
      {error && <p role="alert" className="inline-warning">{error}</p>}
    </div>
    <p className="hint">Values are parsed and validated by the engine; the document updates only when they are accepted.</p>
  </div>;
}
