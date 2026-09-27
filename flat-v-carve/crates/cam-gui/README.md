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

Admission is limited to schema-5 jobs with SVG artwork and up to twelve ordered
Flat V-carve, Face, Profile, Drag knife or Drill operations, plus schema-2 machine profiles.
Older documents are refused; there is no old-file migration in the application.
The fixture regeneration example is test-data preparation only. Profile supports
tabs, finishing and entries. Drill supports hole selection, depth references,
pecking and dwell; see the [drill fixtures](../../fixtures/drill/README.md).

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
