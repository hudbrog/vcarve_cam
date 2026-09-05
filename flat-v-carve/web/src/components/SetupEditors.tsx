import type { Dispatch, ReactNode } from 'react';
import { accuracyFields, endmillLimitFields, endmillPlanningFields, machineProfileFields, placementFields,
  shapeFields, stockFields, strategyFields, toolFields, travelFields, vbitPlanningFields, type Draft, type Field } from '../state/draft';
import type { WorkspaceAction } from '../state/workspace';

interface Props { draft: Draft; fields: (items: Field[]) => ReactNode; dispatch: Dispatch<WorkspaceAction> }
function Group({ title, children }: { title: string; children: ReactNode }) {
  return <section className="inspector-group"><h2>{title}</h2>{children}</section>;
}
function ClearBlock({ fields, dispatch, label }: { fields: Field[]; dispatch: Props['dispatch']; label: string }) {
  return <button className="text-button" onClick={() => dispatch({ type: 'clear-fields', paths: fields.map(field => field.path) })}>{label}</button>;
}
export function StockSetup({ fields }: Props) {
  return <>
    <Group title="Stock">{fields(stockFields)}<p className="hint">Stock top is Z = 0. Depth is positive downward. Stock footprint and clamps are not modeled.</p></Group>
    <Group title="Artwork placement">{fields(placementFields)}<p className="hint">Origin uses source-page coordinates, with Y upward. Scale changes artwork only; cutters, depths, feeds, and tolerances keep their values.</p></Group>
    <Group title="Travel">{fields(travelFields)}<p className="hint">Starting XY uses workpiece coordinates. Planning clearance is a positive Z above the stock top and applies to both tools. Entry, strategy, and resource limits are under Carve & tools.</p></Group>
  </>;
}
export function ToolsSetup({ draft, fields, dispatch }: Props) {
  return <>
    <Group title="Desired shape">{fields(shapeFields)}<p className="hint">Endmill wall allowance defaults to 0 mm. Increase it to leave stock for V-bit cleanup. Physical finish allowances are separate from numerical accuracy.</p></Group>
    {(['endmill', 'vbit'] as const).map((kind, index) => <details className="tool-section" key={kind} open>
      <summary><span className={`tool-index ${kind}`}>{index + 1}</span>{kind === 'endmill' ? 'Endmill clearing' : 'V-bit rest & finish'}</summary>
      {fields(toolFields(draft.base, kind))}
      <ClearBlock fields={toolFields(draft.base, kind).filter(field => field.path.includes('.geometry.'))} dispatch={dispatch} label={`Clear ${kind === 'endmill' ? 'endmill' : 'V-bit'} geometry`} />
      {kind === 'vbit' && <p className="hint">Included angle is the full tip angle. Enter 0 only for an actually pointed tip. V-bit planning uses direct plunge with an explicit capability and plunge feed.</p>}
    </details>)}
    <Group title="Endmill strategy & entry">
      {fields(strategyFields)}
      <p className="hint">Depth-dependent clearing follows the available area at each depth. Deepest-region clearing uses the deepest region on every layer.</p>
      <p className="hint">Direct plunge uses the plunge feed. Ramp entry also needs explicit ramp capability, angle, and feed. Switching entry modes retains unfinished ramp values in this tab; only the active entry is downloaded.</p>
    </Group>
    <details className="tool-section"><summary>Endmill computation limits</summary>
      {fields(endmillLimitFields)}<p className="hint">Resource limits bound computation. Supported ranges and feasibility are checked by Rust.</p>
    </details>
    <Group title="Endmill planning block"><p className="hint">Leave travel, entry, strategy, and limits all blank to keep endmill planning unset. Complete the block when configuring a job, or clear it to download an incomplete job.</p>
      <ClearBlock fields={endmillPlanningFields} dispatch={dispatch} label="Clear endmill planning" />
    </Group>
    <details className="tool-section"><summary>V-bit computation & sampling</summary>
      {fields(vbitPlanningFields)}<p className="hint">These settings control planning resources and the slice/sample preview. Sample spacing does not establish a global verification bound. Leave all blank to keep the block unset.</p>
      <ClearBlock fields={vbitPlanningFields} dispatch={dispatch} label="Clear V-bit planning" />
    </details>
    <details className="tool-section"><summary>Advanced accuracy</summary>{fields(accuracyFields)}</details>
  </>;
}
export function MachineSetup({ fields, dispatch }: Props) {
  return <Group title="Machine profile snapshot">
    {fields(machineProfileFields)}
    <p className="hint">An optional profile needs an ID when any profile value is entered. Work offset and tool numbers must match the actual LinuxCNC setup.</p>
    <p className="hint">M6 is an editable description only; it does not define or validate macro behavior. Tool-length compensation and exact-output checks require the M6 service contract.</p>
    <ClearBlock fields={machineProfileFields} dispatch={dispatch} label="Clear machine profile" />
  </Group>;
}
