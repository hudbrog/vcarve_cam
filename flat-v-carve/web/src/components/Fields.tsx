import type { Dispatch } from 'react';
import { fieldIsActive, fieldText, type Draft, type Field } from '../state/draft';
import type { WorkspaceAction } from '../state/workspace';
import { computationDefaults, computationHints } from '../state/computation';

export function Fields({ fields, draft, errors, dispatch }: {
  fields: Field[]; draft: Draft; errors: Record<string, string>; dispatch: Dispatch<WorkspaceAction>;
}) {
  return <div className="fields">{fields.filter(field => fieldIsActive(draft, field)).map(field => {
    const value = fieldText(draft, field);
    const error = errors[field.path];
    const defaultValue = computationDefaults[field.path];
    const hint = computationHints[field.path];
    const describedBy = [field.unit ? `${field.path}-unit` : '', error ? `${field.path}-error` : '', defaultValue !== undefined ? `${field.path}-default` : '', hint ? `${field.path}-hint` : ''].filter(Boolean).join(' ') || undefined;
    return <div className="field" key={field.path}>
      <label htmlFor={field.path}>{field.label}</label>
      <div className={`input-wrap ${error ? 'invalid' : ''}`}>
        {field.kind === 'boolean' || field.kind === 'choice' ? <select id={field.path} value={value} aria-invalid={!!error} aria-describedby={describedBy}
          onChange={event => { dispatch({ type: 'text', path: field.path, value: event.target.value }); dispatch({ type: 'commit' }); }}>
          <option value="">Not specified</option>{(field.kind === 'boolean' ? [{ value: 'true', label: 'Yes' }, { value: 'false', label: 'No' }] : field.options)?.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
        </select> : field.kind === 'multiline' ? <textarea id={field.path} value={value} placeholder="Not specified" rows={4} aria-invalid={!!error} aria-describedby={describedBy}
          onChange={event => dispatch({ type: 'text', path: field.path, value: event.target.value })} onBlur={() => dispatch({ type: 'commit' })} /> : <><input id={field.path} type="text" inputMode={field.kind === 'text' ? 'text' : field.integer ? 'numeric' : 'decimal'} value={value} placeholder={defaultValue === undefined ? 'Not specified' : String(defaultValue)} autoComplete="off"
          aria-invalid={!!error} aria-describedby={describedBy}
          onChange={event => dispatch({ type: 'text', path: field.path, value: event.target.value })}
          onBlur={() => dispatch({ type: 'commit' })} />
          {field.unit && <span className="unit" id={`${field.path}-unit`}>{field.unit}</span>}</>}
      </div>
      {defaultValue !== undefined && <span className="field-default" id={`${field.path}-default`}>{value.trim() ? `Clear to use default: ${defaultValue.toLocaleString('en-US')}` : `Using default: ${defaultValue.toLocaleString('en-US')}`}{field.unit ? ` ${field.unit}` : ''}</span>}
      {hint && <span className="hint" id={`${field.path}-hint`}>{hint}</span>}
      {error && <span className="field-error" id={`${field.path}-error`}>{error}</span>}
    </div>;
  })}</div>;
}
