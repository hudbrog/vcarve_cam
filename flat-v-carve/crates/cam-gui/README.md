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

## First vertical slice

1. Open `fixtures/gui2/flower.job.json`, or choose **Flower fixture**.
2. Apply `fixtures/gui2/machine.json`, or choose **Apply flower machine profile**.
3. Change depth or cutting feed, then **Generate**.
4. Inspect Top/Isometric, After endmill, After V-bit, and arbitrary backward
   stock scrubbing. Playback shows cumulative removal from the actual motions.
5. **Prepare checked output**, then **Save checked bytes**. Preparation uses
   the retained execution; retry saves the same bytes after a failed write.
6. **Save job**, reopen it and verify the edited settings and applied machine.
7. Enter a partial number such as `-`, restart, and restore the recovery draft.

Admission is intentionally limited to schema-5 jobs with one SVG and one
Flat V-carve operation, plus schema-2 machine profiles. There is no old-file
migration in the application. The fixture regeneration example is test-data
preparation only. Multi-operation authoring and resource editors are later slices.

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
```

`experiments/gui1` is frozen framework qualification evidence. The incumbent
`web` UI and `cam-app` remain available until replacement workflows are accepted;
new GUI work belongs here. See the [slice report](../../../docs/flat-v-carve/gui2a-progress.md).
