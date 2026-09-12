# Flat V-carve GUI

The production home of the shared egui/eframe and wgpu application. GUI2 is an
implementation milestone, not an experimental runtime. Native Windows and
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
The inspector filter and its scroll position help navigate longer forms. See the full
[authoring and recovery recipe](../../../docs/flat-v-carve/gui2bc-progress.md).

Admission is limited to schema-5 jobs with SVG artwork and up to twelve ordered
Flat V-carve, Face or Drag knife operations, plus schema-2 machine profiles.
Older documents are refused; there is no old-file migration in the application.
The fixture regeneration example is test-data preparation only. Profiles, tabs
and entries are later slices.

## Facing and ordered preparation (GUI7)

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

`experiments/gui1` is frozen framework qualification evidence. The incumbent
`web` UI and `cam-app` remain available until replacement workflows are accepted;
new GUI work belongs here. See the [GUI2a report](../../../docs/flat-v-carve/gui2a-progress.md)
and [GUI2b/c report](../../../docs/flat-v-carve/gui2bc-progress.md).
