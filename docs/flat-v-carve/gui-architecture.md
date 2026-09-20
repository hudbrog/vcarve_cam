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
