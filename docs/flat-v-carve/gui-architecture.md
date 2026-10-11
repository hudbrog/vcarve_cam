# GUI architecture

`cam-gui` is the production egui/eframe application with a persistent wgpu
viewport. Native and browser builds share document, editing and simulation
code. The framework experiment is retained under `experiments/gui1`; its
completed review logs and measurements are in Git history.

## State and editing

The portable schema-5 document, raw editing drafts, reusable-resource drafts,
retained execution and presentation state have separate ownership. `app` owns
commands, Undo/Redo, freshness and save snapshots; `session` adapts the retained
service; `state`, `recovery`, `resources` and platform adapters own their
respective local state.

Numeric fields preserve incomplete input such as `-` or `1.` without changing
committed geometry. Stable field/entity IDs bind drafts, focus and diagnostics;
array positions and display names are not identity. High-level operations must
resolve pending edits explicitly. Undo transactions restore the document and
relevant drafts together. Recovery can preserve partial text that does not
belong in a portable job.

The Prepare/Simulate routes share the ordered navigator and selected entities.
Library, Machine library and Job tools use resource workspaces. Opening a
resource workspace pauses playback and preserves the playhead; it does not
discard execution or automatically resume playback on return. Shared semantic
theme/widgets and procedural diagrams are described in the
[asset manifest](ui-asset-manifest.md).

## Sketch artwork workspace

**New sketch** in the artwork navigator and **Edit sketch** below the artwork
inspector heading open the shared orthographic XY editor (`sketch`, `app::sketch_ui`).
Double-clicking a sketch navigator row or its Edit context action reopens it too.
The canvas works in local millimetres with Y up. Lines, polylines, rectangles,
circles, three-point arcs and reference points create persistent model entities;
coincident open endpoints join explicit wires. Closing a polyline makes a closed
wire. Construction wires render dashed and do not publish machining geometry.
Visible endpoint, centre, midpoint, quadrant, origin and grid snaps copy exact
coordinates without creating constraints. Geometry snaps take priority over grid.

Selection always uses ordered individual point/body operands; Shift-click adds/removes,
double-click selects a complete wire, and an empty-space drag box-selects geometry.
The geometry list shares that selection. Panel visibility never changes selection
or the active tool. Delete and Toggle construction require complete wires and reject
partial selections without expanding them. Related constraints must be removed before
deletion. The optional constraint panel edits all supported constraints,
including fixed points and driving/reference dimensions. Its form is a local edit
until Add/Update; Apply requires committing or discarding changed form text.
Canvas dimension labels remain visible with the panel closed and double-clicking
one opens its form. `sketch::tools_ui` provides visible drawing/constraint/dimension
tools and FreeCAD-compatible key sequences with a two-second prefix timeout and
text/IME focus protection. Geometry and constraint list visibility persists in egui
session data between sketch sessions; their resizable container uses egui layout memory.

`sketch::solve` sends bounded, revision-tagged requests to `sketch::worker`: a
dedicated native `--sketch-worker` child or browser `sketch-worker.js`. The worker
is independent of CAM computation. Structural edits cancel prior requests; drag
targets coalesce to one latest queued request. Obsolete IDs/revisions and superseded
cursor replies cannot install geometry or diagnostics. Every accepted candidate
is independently checked for identical authored identities/definitions, numerical
residuals and connected wire joins before installation. Failures retain the last
accepted geometry, leaving conflicting authored constraints editable. Worker failure
and a ten-second deadline terminate the worker and permit retry.

Arael treats wire joins as endpoint equalities and cursor positions as soft goals.
The last solve stage projects onto driving constraints with cursor penalties off.
An unreachable cursor is reported as a drag limit, not authority to relax a driving
dimension. DOF is measured without cursor penalties. Constraint rows show measured
references, residuals and exact duplicate warnings; diagnostics do not claim a
minimal conflict set or general redundant-constraint classification.

The editor owns its model, gesture state, selection and local undo/redo. One
completed drawing action or drag is one local transaction; history is bounded to
64 entries and 4 MB per stack. Escape cancels the current gesture and returns to
selection. Enter finishes an open polyline. Wheel zoom and middle/right drag pan
the canvas; Fit sketch frames its geometry. The fixed source envelope remains
unchanged. Placement, stock and work zero are never inferred from draft bounds.

Apply validates structure, numerical constraints and materialized topology before
calling the atomic add/apply command. Invalid output keeps the draft open with a
diagnostic. A changed Apply is one document undo step; an unchanged Apply closes
without invalidating the job. Cancel discards the draft. Job save/open, generation
and document history are unavailable while editing. Native window close requires
Apply/Cancel; browser tab close/reload uses a before-unload prompt.

`commands::apply_sketch_with_report` commits model and reference revisions together
and returns a transient explanation of output and binding changes. Stable wire
membership, entity types, region holes and ancestry provide continuity evidence.
Changed anchor boundaries, removed/split outputs, and already unresolved bindings
require explicit repair. The inspector report links to the existing operation
selection/start/tab controls. Reference inspection remains authoritative after
save/reopen or undo/redo; reports are cleared on subsequent document edits.
Apply invalidates prepared output and old plan freshness. Retained preparation
independently checks machining identity, including mixed SVG/sketch operations.

`artwork::export_svg` preserves SVG source bytes or delegates to
`sketch::export::to_svg` for verified geometry in the fixed millimeter frame.
Even-odd compound paths preserve holes; open wires remain strokes. Curve
tessellation is shared with CAM. Isolated circles retain analytic elements;
compound-region circles become paths. Placement, constraints and construction
data remain in the job. Export uses the existing native atomic-write/browser
save/download ports with an exact-byte retry; its save completion cannot mark
the portable job saved.

Recovery stores the current model, source identity, target and name separately
from the committed job. It retains incomplete accepted drawing geometry, including
open wires, and the constraint form including incomplete dimension text, but not an
unfinished click gesture, camera, worker results or local undo history. Restoring a
draft starts fresh local history and solver analysis; document undo history is
still recovered. Recovery writes remain asynchronous and use the existing size
limit and visible failure status. Apply refuses a draft whose original source
identity has changed. Entity/wire IDs use random 128-bit identities so deleted
geometry cannot acquire a former ID after reopening.

## Workers and persistence

Native computation runs in a disposable process; browser computation runs in a
Web Worker with the WASM engine. Request identity, document revision and
machining/execution identity reject stale replies. Cancellation and worker
failure must leave the editor responsive and preserve editable work. The
loopback HTTP server only hosts static files.

Save snapshots and prepared output retain their exact contents across failures.
Native file access and browser direct-save/download paths report their actual
outcome. A browser download request does not establish that bytes reached disk.
Portable jobs, recovery envelopes and reusable library/catalog files have
independent formats and revision rules.

## Viewport and stock

The viewport consumes paged actual-motion data and bounded stock checkpoints.
Stable identities allow resident pages to be reused; tile versions limit stock
uploads. Camera changes must not rebuild machining or re-upload an unchanged
scene. Picking combines a spatial candidate search with projected geometry and
uses the same coordinate frame as rendering.

The current display admits at most 250,000 motions and 32 stage groups
(`session.rs`). GPU residency is paged, but the initial execution still travels
as one payload; this is not end-to-end streaming. Display limits refuse oversized
jobs rather than silently truncating them. Stock resolution/checkpoint bounds
are distinct from machining tolerances.

Stock display is a 2.5D heightfield derived from recorded cutter sweeps. Cells
retain removal depth and stage/tool identity. Appearance can be plain, by
operation, by tool or by depth. Interior/perimeter walls are derived from the
displayed field with explicit geometry budgets. X-ray uses a translucent pass;
front/back/left/right views use axis-aligned section/silhouette geometry.
Display quality, omitted geometry and resolution remain visible limitations.

Playback uses modeled motion duration and a `(prefix, fraction)` position.
Feed moves, rapid rate, arcs and dwell contribute through the shared clock;
fractional cutting updates the displayed stock and cutter together. Backward
seeks replay from retained checkpoints. Hiding a path never restores material.
Tool/holder display comes from declared cutter geometry, optional shaft and
stickout, and the selected machine holder.

`sim_checks.rs` estimates assembly-below-surface and rapid-through-material
warnings on a stated raster. The inspection panel and timeline expose those
warnings. They are display estimates, not authoritative core collision checks
or an export gate. Acceleration, controller junction dynamics, unknown M6
motion and detached parts are not simulated.

## Maintenance and qualification

Use `crates/cam-gui/tests`, module tests, browser scenarios under
`crates/cam-gui/web`, and `scripts/native-smoke.mjs` for reproducible behavior.
Keep fixture READMEs beside their inputs. `scripts/ui-field-inventory.mjs`
generates a numeric-field destination inventory under `artifacts/`.

Windows native and desktop Chromium/WebGPU are the established review envelope.
Screen-reader support on native and browser targets is an accepted scope
exclusion; keyboard, focus, IME behavior and named test controls remain relevant.
Release qualification must distinguish synthetic input and renderer-restoration
drills from real OS dialogs, IME/device failures and physical GPU memory use.
See the [backlog](backlog.md) for remaining qualification rather than treating
old milestone completion as evidence for every platform.
