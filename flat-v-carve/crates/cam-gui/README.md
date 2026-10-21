# Flat V-carve GUI

The production home of the shared egui/eframe and wgpu application. Native Windows and
desktop Chromium/WebGPU use the same Rust document, retained execution,
simulation and checked-output workflow.

From the workspace root:

```powershell
cargo run -p cam-gui --release --locked
./scripts/build-gui.ps1
./scripts/build-gui.ps1 -Target web
node crates/cam-gui/web/serve.mjs
```

The native artifact is `artifacts/gui/native/cam-gui.exe`. The browser artifact
is `artifacts/gui/browser/`; serve that directory over localhost or HTTPS and
open `/web/index.html`. The development server uses port 5182. Building the
browser requires wasm-pack and the wasm32-unknown-unknown Rust target.

## Known-carve regression

For Pocket, open `fixtures/pocket/two-pockets.job.json`. The operation editor
has **Geometry & depth**, **Cutting**, and **Entry & leads** tabs. See the
[Pocket guide](../../../docs/flat-v-carve/pocket.md) for selection semantics,
entry constraints and verification. The browser regression command is
`node crates/cam-gui/web/smoke.mjs --pocket` from the workspace root.

1. Open `fixtures/gui2/flower.job.json`, or choose **File → Flower fixture**.
2. In **Machine**, load `fixtures/gui2/machine.json`, or expand **Example machine**
   and choose **Apply flower machine profile**.
3. Change depth or cutting feed, then **Generate**.
4. Select **Simulate**. Inspect Top/Isometric, After endmill, After V-bit, and arbitrary backward
   stock scrubbing. Playback shows cumulative removal from the actual motions.
5. **Export…**, then **Save checked bytes** in Machine & Export. Preparation uses
   the retained execution; retry saves the same bytes after a failed write.
6. **Save job**, reopen it and verify the edited settings and applied machine.
7. Enter a partial number such as `-`, restart, and restore the recovery draft.

For a new carving, use **File → Import SVG** or **+ Import artwork**. Select
filled components in the operation's own **Cutting → Geometry to carve**, or
click them in the viewport (Shift-click adds or removes one); the Artwork panel
manages placement, hide/lock and sources only. Set stock/work zero in Setup, and
edit the two job tools and cutting assignments. New machining values are unset.
The inspector filter and its scroll position help navigate longer forms. See
[GUI architecture](../../../docs/flat-v-carve/gui-architecture.md) for editing,
recovery and worker contracts.

Admission is limited to schema-5 jobs with SVG or sketch artwork and up to twelve ordered
Flat V-carve, Face, Profile, Drag knife or Drill operations, plus schema-2 machine profiles.
Older documents are refused; there is no old-file migration in the application.
The fixture regeneration example is test-data preparation only. Profile supports
tabs, finishing and entries. Drill supports hole selection, depth references,
pecking and dwell; see the [drill fixtures](../../fixtures/drill/README.md).

## Sketch artwork

Choose **New sketch** in the artwork navigator. To reopen a saved sketch,
double-click its name under **ARTWORK**, right-click that row and choose
**Edit sketch**, or select it and use **Edit sketch** below its inspector heading.
Open the saved job JSON to retain editable geometry and dimensions; importing an
exported SVG creates SVG artwork with no sketch constraints.

The left **Tools** panel contains all supported drawing, constraint and dimension
commands. Hover a tool for its shortcut and required selection. Draw an outline with Rectangle or Polyline, then add
circles for holes. Polyline closes when you click its first vertex; Enter finishes
an open path. Arc takes start, a point along the arc, then end. Point creates a
reference point with no cutting geometry. Construction creates dashed geometry
excluded from machining.

Rectangle adds horizontal and vertical constraints automatically. Its connected
corners stay joined, leaving position, width and height free until dimensioned.
Dragging a corner preserves the rectangular shape. These constraints are created
and undone with the rectangle; existing sketches are not retroactively changed.

The origin and X/Y axes are fixed construction references. Click them on the
canvas or select **Origin**, **X axis**, or **Y axis** at the top of Geometry list.
For example, select a corner, Shift-select Origin, then choose Coincident to
anchor that corner at zero. When an endpoint overlaps the origin, Shift-click it
again to add Origin, or use the list. These references cannot be moved, deleted,
or converted to cutting geometry.

Snap shows its target beside the cursor. Snap and Grid snap can be switched off;
the grid spacing is editable in millimetres. Snapping does not add constraints.
Click selects an individual edge or point; Shift-click adds/removes items.
Double-click an edge, or choose **Select wire**, to select its complete wire.
Drag empty space left-to-right to select contained geometry, or right-to-left
to include crossed geometry. Drag selected geometry as far as constraints permit.
Delete selection and Toggle construction require complete wires and explain how
to select them; neither silently expands a partial selection. Remove related
constraints before deleting a wire.
Ctrl-Z/Ctrl-Y (or Cmd-Z/Cmd-Shift-Z) use sketch-local history. Escape cancels a
gesture and returns to Select. Wheel zooms, middle/right drag pans, and Fit sketch frames the geometry.

**Geometry list** and **Constraints & dimensions** independently show optional
lists on the right. Toggling either preserves selection and the active tool.
Geometry rows select the corresponding canvas entity; **Locate selection** centres
it. Constraint filters show All, Dimensions, or Problems. Panel visibility is
remembered when reopening sketches in the same app session; widths are resizable.
At small window sizes the tools and lists scroll.

FreeCAD 1.1.4 default sequences are pressed in order, not held together. After a prefix,
the footer shows available continuations. Escape cancels a sequence; an unfinished
sequence expires after two seconds. C waits 0.7 seconds for the C,C alternative.
Tool shortcuts do not run inside text fields. **Shortcuts** in the tools panel lists
every default command binding and its availability. See the complete
[compatibility catalogue](../../../docs/flat-v-carve/freecad-sketcher-shortcuts.md)
for missing tools and selection limitations.

| Command | Shortcut |
| --- | --- |
| Contextual dimension (length / diameter / radius / angle / distance) | D |
| Line / Polyline / Rectangle / Circle / Point | G,L / G,M / G,R / G,C / G,Y |
| Three-point arc | G,3,A |
| Construction (selected complete wires, or new geometry) | G,N |
| Auto horizontal/vertical / Horizontal / Vertical / Coincident | A / H / V / C (strict: C,C) |
| Parallel / Perpendicular / Equal length or radius / Tangent or collinear / Symmetry or midpoint | P / N / E / T / S |
| Point on line or circle / signed X distance / signed Y distance | O / L / I |
| Distance between points, or length of selected line | K,D |
| Radius / Diameter / Angle / Fix entity | K,R / K,O / K,A / K,B |
| Automatic circle diameter / arc radius | R or K,S |
| Lock point position with X/Y dimensions | K,L |
| Switch selected dimension driving/reference, or new dimension mode | K,X (leave the value field with Tab first) |
| Toggle origin / horizontal axis / vertical axis in selection | Z,O / Z,H / Z,V |
| Select geometry belonging to the current constraint / remove selected lines' H/V constraints | Z,E / Z,R |
| Select all authored geometry | Ctrl+A |

**Apply sketch** validates the output and makes one job undo transaction.
**Cancel sketch** leaves the committed artwork intact. Apply reports invalid
intersections, incomplete topology or unsatisfied saved constraints and leaves the
draft editable. Local recovery retains the draft separately from the job; after
restoring, local undo history starts afresh. Finish with Apply/Cancel before saving
or opening another job. Stock and artwork placement stay fixed throughout editing.

Selection works with the lists open or closed. Click an endpoint or centre for a point operand; click the middle of a curve for an
entity operand. Shift-click adds operands in the order shown as A, B and C.
Choose a visible geometric constraint tool to apply it to a valid selection;
otherwise follow its selection hint and use **Add constraint**. Dimension tools
open a value form. Double-click a canvas dimension label or click its list row to edit
or remove it; use **Update constraint** to commit form changes. **Clear operands**
discards the form edit. Remove related constraints before deleting a wire.

For rapid dimensioning, select geometry and press bare **D**. A line gets a length,
a circle a diameter, an arc a radius, two line bodies an angle, and point selections
a distance (including point-to-axis offsets). The value field opens with its current
value selected: just type the replacement. **Enter** or selecting the next geometry
saves the value to the local sketch and starts solving; press **D** again for the
next selection. An existing dimension of the same kind and operands reopens instead
of adding a duplicate. **Escape** in the value field discards the pending value.
Invalid input keeps the original selection and field open for correction. These
commits are local undo steps; **Apply sketch** still commits the artwork to the job.
The explicit dimension tools and **K,D** sequence remain available.

**Distance** accepts a selected line for its length, two points for their distance,
or a point and an axis for its perpendicular offset. Select a corner and Shift-click
an axis (either selection order works): the X axis gives a signed Y offset, and
the Y axis gives a signed X offset. Positive values mean above/right of the axis;
negative values mean below/left. A single selected point measures from the origin.
Horizontal/vertical distance also accepts one line, measuring end minus start.
These selections work through the tools, shortcuts and constraint dropdown,
whether the selection or command comes first. Leave a dimension value blank to
use the current measurement.

Available constraints include coincident, horizontal/vertical, parallel,
perpendicular, equal length/radius, tangent, point on line/circle, midpoint,
symmetry, fixed entity/point, distance, signed X/Y distance, length, radius, diameter and
signed angle. Dimensions use local millimetres or degrees. X/Y distances are
A minus B; angles run from line A to line B. Tangency uses the supporting circle
of an arc. Wire joins remain connected automatically.

Driving dimensions set the geometry. **Reference dimension** measures it without
removing degrees of freedom; conversion to driving starts at the current value.
Drag a selected endpoint to deform its wire, or select a complete wire to
move a group. Arael solves in a separate native process/browser Worker. The status
shows remaining DOF, failures and cursor limits. **Solve sketch** retries and
**Cancel solve** stops an active request. Failed solves keep the previous geometry
and the editable constraints. Diagnostic rows locate unsatisfied constraints;
hover a constraint for its residual. Duplicate constraints are flagged, but the
diagnostics do not promise a minimal conflict set or a complete redundancy analysis.

The current per-sketch limits are 500 entities, 2,000 constraints, 500 wires,
500 total wire edges, and 2 MB of model JSON. Geometry coordinates/parameters
must be finite and within the model's ±100,000 numeric range. Requests/replies
are limited to 2.5/4 MB; a solve exceeding ten seconds is terminated and can be
retried. Sketch undo retains up to 64 entries and 4 MB per history direction.
These limits bound resource use; a 500-entity sketch is not guaranteed to solve
interactively. Split a larger design into separate artwork items.

Constraint edits and each completed drag have one local undo step. Recovery also
retains unfinished dimension text; Apply requires adding/updating or discarding
that form first. Applied sketches use the existing operation geometry pickers
alongside SVGs. Constraint validity is separate from the topology check at Apply.

After Apply, the artwork inspector reports preserved selections/anchors and
anything needing repair. Dimension changes keep selections whose persistent
topology survives. Deleted or split outputs, changed region membership, and
manual starts/tabs on changed boundaries need explicit reassignment. **Repair
start**, **Repair tab**, and **Repair selection** open the affected operation.
Existing unresolved references stay unresolved. Apply is one job undo step;
undo/redo restores both the sketch and its operation references. Generate again
after geometry edits before preparing checked output. SVGs and sketches can
participate in the same operation and the same generated job.

**Export artwork SVG** saves a geometry snapshot without changing the editable
job or marking it saved. Sketches export their fixed page in millimeters,
excluding construction geometry, constraints, and setup placement. Closed
regions retain holes and islands; open wires remain strokes. Curves use CAM's
tessellation tolerance; isolated circles remain analytic SVG circles, while
circles in compound regions become paths (their analytic drill markers are not
retained in that SVG). SVG sources export their original bytes. Page coordinates
are relative to the sketch frame's minimum; when reimporting a sketch with a
nonzero frame minimum at its old setup position, subtract that minimum from the
old placement origin. Keep the job JSON for editable dimensions and original
selection identities.

Tests: `cargo test -p cam-gui --lib sketch` and, with browser assets built and
the development server running, `node crates/cam-gui/web/smoke.mjs --sketch`.
Use `--sketch-constraints` for the browser constraint/drag/recovery scenario.
Use `--sketch-hotkeys` for bare D, immediate value replacement, committing on next
canvas/list selection, Enter/Escape, invalid input, existing dimensions and save/reopen.
Use `--sketch-basics` for rectangle constraints, fixed origin/axes, contextual
distances and saved reference roundtrips through the real browser solver Worker.
Use `--sketch-ux` for independent selection/lists, shortcuts, reopening, dimension
labels and solver failure/retry. Use `--sketch-performance` for production WASM
worker measurements; `node scripts/qualify-sketch-native.mjs` measures the release
native solver process. Neither benchmark substitutes for the normal native UI run.
Use `--sketch-lifecycle` for mixed-source Apply, anchor repair, undo/redo, checked
output, SVG retry and save/reopen. The `ui-review` feature accepts `sketch`,
`sketch-constraints`, and `sketch-impact` (with `fixtures/v5/sketch-lifecycle-job.json`)
for native captures.

## Simulator detail

The simulator's **Display** control offers Coarse, Standard and Fine relative
grids, plus fixed **0.5 mm**, **0.2 mm** and **0.1 mm** cell spacing. For a
960 × 125 mm stock, Fine uses about 0.94 mm cells; 0.1 mm uses 9600 × 1250
cells. The control shows the actual spacing and grid dimensions. These are
display samples: a feature narrower than a cell can disappear, and seeing the
shape of a feature requires multiple cells across it.

Changing spacing replays the retained toolpath at the same playhead without
replanning. Physical presets keep up to 256 MiB of checkpoints in the worker
and send one frame to the viewport. They admit at most 64 MiB per padded frame;
larger grids ask for a coarser spacing rather than silently reducing detail.
Finer grids take more memory and may slow playback and rendering.

## Facing and ordered preparation

**File → New face job** starts a source-free Face job: set stock in Setup, then
the coverage, height, tool and cutting values in Cutting. Passes run at 0° or
90°, coverage is the whole stock or a rectangle with per-side margins, and the
entry/exit overrun is travel beyond the requested coverage. A newly created
operation leaves every machining value unset.

The Operations list is the only execution order: add, rename, enable/disable,
move and delete operations there. **Generate** plans the whole enabled list;
**Generate through here** (or **Generate through <operation>**) plans the
enabled prefix and is the scope that **Export…** later prepares — a prefix is
never widened by export. In a Face → carving sequence the carving's top can be
bound to the plane the face published (**Cutting → Top: <face> face result**);
reordering or disabling that face leaves a saveable unresolved reference with a
located issue until the order is repaired. **Simulate** reports one entry per
executed stage, so **After face** shows the stock before the carving cuts
anything, and hiding earlier paths never rewinds the removal.

`app` owns edits, Undo/Redo, freshness and save snapshots; `session` calls
cam-service's retained runtime; `worker`/`platform` provide process or browser
Worker isolation; `viewport`, rendering and simulation modules display results.
No displayed mesh or simulation result grants export authority.

```powershell
cargo fmt --all -- --check
cargo clippy -p cam-gui --all-targets --locked -- -D warnings
cargo test -p cam-gui --release --locked
# After the browser build, with the development server running:
node crates/cam-gui/web/smoke.mjs
node crates/cam-gui/web/smoke.mjs --authoring
node crates/cam-gui/web/smoke.mjs --gui7
```

`experiments/gui1` preserves the framework experiment; the former React UI has
been removed. New GUI work belongs here. The [documentation index](../../../docs/flat-v-carve/README.md)
links current contracts and remaining work.
