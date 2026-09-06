import type { Dispatch, ReactNode } from 'react';
import { accuracyFields, endmillLimitFields, endmillPlanningFields, fieldText, machineProfileFields, placementFields,
  shapeFields, stockFields, strategyFields, toolFields, travelFields, vbitPlanningFields, type Draft, type Field } from '../state/draft';
import type { WorkspaceAction } from '../state/workspace';
import type { ToolSlot } from '../contracts/library';
import type { LibraryAssignment } from '../service/useLibraryAssignments';
import { draftToolChanges } from '../state/library';
import { computationDefaults } from '../state/computation';

interface Props { draft: Draft; fields: (items: Field[]) => ReactNode; dispatch: Dispatch<WorkspaceAction> }
function Group({ title, children }: { title: string; children: ReactNode }) {
  return <section className="inspector-group"><h2>{title}</h2>{children}</section>;
}
function ClearBlock({ fields, dispatch, label }: { fields: Field[]; dispatch: Props['dispatch']; label: string }) {
  return <button className="text-button" onClick={() => dispatch({ type: 'clear-fields', paths: fields.map(field => field.path) })}>{label}</button>;
}
function DefaultBlock({ fields, draft, dispatch, label }: Omit<Props, 'fields'> & { fields: Field[]; label: string }) {
  return <button disabled={fields.every(field => !fieldText(draft, field).trim())} onClick={() => dispatch({type:'clear-fields',paths:fields.map(field => field.path)})}>{label}</button>;
}
export function StockSetup({ fields }: Props) {
  return <>
    <Group title="Stock">{fields(stockFields)}<p className="hint">Stock top is Z = 0. Depth is positive downward. Stock footprint and clamps are not modeled.</p></Group>
    <Group title="Artwork placement">{fields(placementFields)}<p className="hint">Origin uses source-page coordinates, with Y upward. Scale changes artwork only; cutters, depths, feeds, and tolerances keep their values.</p></Group>
    <Group title="Travel">{fields(travelFields)}<p className="hint">Starting XY uses workpiece coordinates. Planning clearance is a positive Z above the stock top and applies to both tools. Entry, strategy, and resource limits are under Carve & tools.</p></Group>
  </>;
}
export function ToolsSetup({ draft, fields, dispatch, openLibrary, assignments = {} }: Props & {
  openLibrary?: (slot?: ToolSlot) => void; assignments?: Partial<Record<ToolSlot, LibraryAssignment>>;
}) {
  return <>
    <Group title="Reusable tools"><button disabled={!openLibrary} onClick={() => openLibrary?.()}>Manage tool library</button><p className="hint">Save cutter geometry and cutting presets, then review a selection before applying it to this job.</p></Group>
    <Group title="Computation & accuracy"><p className="hint">Planning budgets and numerical tolerances have automatic defaults. Existing values in your job are kept; advanced overrides are below. Travel, cutter settings and requested surface finish still need your setup.</p>
      <DefaultBlock fields={[...endmillLimitFields, ...vbitPlanningFields.filter(field => !['max_cleanup_iterations','quality_sample_spacing_mm','stock_slices'].some(key => field.path.endsWith(`.${key}`)))]} draft={draft} dispatch={dispatch} label="Use default planning budgets" />
      <p className="hint">Replaces budget overrides with the automatic ceilings. Sampling, cleanup, tolerances and cutting settings stay as entered. Undo restores the overrides.</p>
    </Group>
    <Group title="Desired shape">{fields(shapeFields)}<p className="hint">Endmill wall allowance defaults to 0 mm. Increase it to leave stock for V-bit cleanup. Physical finish allowances are separate from numerical accuracy.</p></Group>
    {(['endmill', 'vbit'] as const).map((kind, index) => <details className="tool-section" key={kind} open>
      <summary><span className={`tool-index ${kind}`}>{index + 1}</span>{kind === 'endmill' ? 'Endmill clearing' : 'V-bit rest & finish'}</summary>
      <button disabled={!openLibrary} onClick={() => openLibrary?.(kind)}>Choose {kind === 'endmill' ? 'endmill' : 'V-bit'} from library</button>
      {assignments[kind] ? <p className="tool-assignment" role="status"><strong>{draftToolChanges(draft,assignments[kind]!.tool,kind).length === 0 ? 'Applied from library' : 'Edited since library selection'}: {assignments[kind]!.toolName}</strong><span>{assignments[kind]!.presetName ? `Cutting preset: ${assignments[kind]!.presetName}` : 'Geometry only · cutting settings cleared at application'}</span></p>
        : <p className="hint">Current job values are shown below. In the library, choose a tool and cutting preset, then review and apply them here.</p>}
      {fields(toolFields(draft.base, kind))}
      <ClearBlock fields={toolFields(draft.base, kind).filter(field => field.path.includes('.geometry.'))} dispatch={dispatch} label={`Clear ${kind === 'endmill' ? 'endmill' : 'V-bit'} geometry`} />
      {kind === 'vbit' && <p className="hint">Included angle is the full tip angle. Enter 0 only for an actually pointed tip. V-bit planning uses direct plunge with an explicit capability and plunge feed.</p>}
    </details>)}
    <Group title="Endmill strategy & entry">
      {fields(strategyFields)}
      <p className="hint">Depth-dependent clearing follows the available area at each depth. Deepest-region clearing uses the deepest region on every layer.</p>
      <p className="hint">Direct plunge uses the plunge feed. Ramp entry also needs explicit ramp capability, angle, and feed. Switching entry modes retains unfinished ramp values in this tab; only the active entry is downloaded.</p>
    </Group>
    <details className="tool-section" id="endmill_budgets" tabIndex={-1}><summary>Advanced endmill computation</summary>
      <p className="hint">Default ceilings are 256 layers, 1,024 loops per layer and 100,000 motions. Work stops when the job is done; these values do not request extra cuts.</p>
      <DefaultBlock fields={endmillLimitFields} draft={draft} dispatch={dispatch} label="Use default endmill budgets" />
      {fields(endmillLimitFields)}
    </details>
    <Group title="Endmill planning block"><p className="hint">Set travel, strategy and entry for the job. Blank computation limits use defaults. Clearing the block leaves travel and entry unset and can be undone.</p>
      <ClearBlock fields={endmillPlanningFields} dispatch={dispatch} label="Clear endmill planning" />
    </Group>
    <details className="tool-section" id="vbit_planning" tabIndex={-1}><summary>Advanced V-bit computation</summary>
      <p className="hint">Blank fields use defaults automatically. Work ceilings use the engine’s supported maximums; they protect memory and bound difficult calculations. Increasing a ceiling does not guarantee that geometry or finish requirements can be satisfied.</p>
      <p className="hint">Sampling and cleanup also affect planning quality and generated cleanup cuts. Defaults use 1 mm sample spacing, up to 2 cleanup iterations and 8 stock slices. Verification has its own controls.</p>
      <DefaultBlock fields={vbitPlanningFields} draft={draft} dispatch={dispatch} label="Use default V-bit settings" />
      {fields(vbitPlanningFields)}
    </details>
    <details className="tool-section" id="numerical_tolerances" tabIndex={-1}><summary>Advanced accuracy</summary><p className="hint">Defaults: 0.01 mm motion tolerance and 0.05 mm verification tolerance. These stay fixed as the job changes. If the geometry is too coarse for them, refine its import precision or choose explicit tolerances; planning never loosens them automatically.</p><DefaultBlock fields={accuracyFields.filter(field => computationDefaults[field.path] !== undefined)} draft={draft} dispatch={dispatch} label="Use default numerical tolerances" />{fields(accuracyFields)}</details>
  </>;
}
export function MachineSetup({ fields, dispatch }: Props) {
  return <Group title="Job machine constraints">
    {fields(machineProfileFields)}
    <p className="hint">An optional profile needs an ID when any profile value is entered. Work offset and tool numbers must match the actual LinuxCNC setup.</p>
    <p className="hint">These optional job constraints must agree with the separate LinuxCNC export profile above. The legacy M6 description does not populate or approve that profile’s contract. Changing this job block requires a new plan.</p>
    <ClearBlock fields={machineProfileFields} dispatch={dispatch} label="Clear machine profile" />
  </Group>;
}
