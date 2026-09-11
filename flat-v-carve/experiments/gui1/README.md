# GUI1 candidate experiment

Historical framework evidence. Product development has moved to
[`crates/cam-gui`](../../crates/cam-gui/README.md), the production workspace
crate with native and browser entry points. Do not build new GUI features here.

**GUI1 is complete: the framework is accepted with explicit scope limits.** The
user accepted it on 2026-09-11 for Windows x86_64 native and desktop
Chromium/WebGPU; the unexercised platform rows and the deferred manual checks are
listed in the decision. The user also removed browser, then native,
screen-reader support as requirements on 2026-09-11, so screen readers are
outside scope on both targets. Keyboard/focus/IME behavior and named controls for
framework tests are covered by the native suite and the real-browser smoke test.
See the [framework decision](../../../docs/flat-v-carve/ui-framework-decision.md).
This is a runnable, deliberately incomplete risk experiment, not GUI2 or a new preferred application.

From PowerShell in this directory:

```powershell
./launch.ps1 -Target native
./launch.ps1 -Target web
```

The web command serves [egui](http://127.0.0.1:5181/web/index.html) and the
[two-field DOM comparison](http://127.0.0.1:5181/web/dom-probe.html) on localhost,
plus the [browser input probe](http://127.0.0.1:5181/web/input-probe.html) and
[platform qualification](http://127.0.0.1:5181/web/platform-probe.html) pages.
Run one server. Close the native test window before rebuilding its executable.
Prerequisites: Rust 1.95/MSVC, wasm32-unknown-unknown target, wasm-pack 0.15.0
(verify your local version), Node 24 or later. Dependency versions/features are
explicit in Cargo.toml and the independent Cargo.lock. Browser build inherits
the existing `flat-v-carve/.cargo/config.toml` getrandom WASM flag.

Already built artifacts live in `target/release/cam-gui1-desktop.exe` and `pkg/`.
To serve a previously built browser package without rebuilding: `node web/serve.mjs`.
WebGPU is required; WebGL2, Firefox, Linux, and Safari are not qualified.

## Five-minute review

1. Choose **Flower reference**. The exact embedded job is planned by Rust in a
   disposable process/Worker. Expect 7,048 roughing and 15,835 finishing motions.
   Cyan is endmill, amber is V-bit; both use actual recorded motion endpoints.
   The status line reports the plan time, motion count, page count, metadata and
   payload bytes. The scene arrives as one binary payload, not a JSON document.
2. Switch Top/Isometric, drag to rotate, zoom, choose Endmill/V-bit paths, and
   scrub **Stock motion** or jump with **Transported checkpoint**. Stock removal
   and the physical stock box are displayed at exact motion prefixes: a
   checkpoint is instant, anything between two checkpoints replays only the
   suffix and reports its seek time. The UI labels the preview cell size, the
   finer reference cell size and the checkpoint count. Uncheck **Stock preview**
   for the original motion-line timeline. Path filtering preserves cumulative
   stock. On the L workload the status line says that arbitrary seeks are
   unavailable and keeps checkpoint jumps.
3. Click in the viewport to pick a motion: the status reports the motion index,
   its distance in physical pixels and the current DPI scale, a translucent
   ribbon marks the pick and a tool marker (endmill disc or V-bit cone with its
   declared radius) sits at the playhead. The **Pick tolerance (px)** slider is
   in physical pixels, so the same setting stays the same physical target at any
   display scale. The status line also reports resident/omitted pages, copied
   tiles and the picking index build time.
4. Type `-`, `1.`, or invalid text into Maximum depth. Reverse artwork order;
   select operation 2 and return to 1. Text belongs to stable source/operation/field
   IDs. Inspector fields are explicitly input probes, not machining edits.
5. Start **Busy worker**, continue typing, then **Cancel**. Prior display remains.
   Native reports kill/reap time after supervisor observation. Browser reports
   Worker.terminate call time only; actual stopped CPU latency is unknown.
6. **Risk probes** offers S/M/L workloads (paged motion streams, replayable
   checkpoints, tiled stock and measured transport), a compute-child crash, a
   renderer visibility failure/resume, a **GPU recovery drill** that rebuilds
   pipelines and buffers and re-copies pages from retained CPU data, an
   **Inject GPU validation error** probe that submits a real invalid request and
   shows wgpu's reported error, GPU page-budget choices (16/64/256 MiB) and
   denied-write injection. The injected renderer failure is a visibility control,
   not device loss; driver device loss remains unverified.
   In the browser, click the workspace once before trying keyboard shortcuts:
   the canvas takes focus on click, and until it does egui discards key events.
7. Save/recover raw draft JSON, or wait for **Recovery saved** after editing.
   Restart/reload and choose **Restore session** to recover invalid text and the
   exact job, with derived results recalculated. Native uses `gui1-recovery/`
   beside the executable; browser uses IndexedDB and an offline asset cache.
   Enable denied-write injection, attempt Save, disable it, then use **Retry
   previous save** to retry the retained bytes. Ctrl/Cmd+O opens a job, Ctrl/Cmd+S
   saves the raw draft, and Ctrl/Cmd+F searches settings.
8. **Prepare checked small reference** exercises retained core plan/output checks.
   The unchanged contact-line fixture fails `M5_FLOOR_RIDGE`; expect **failed**,
   zero checked programs, and disabled Save checked bytes. No settings are relaxed
   to produce G-code. Save reference job preserves exact input bytes independently
   of the experimental raw fields. Native save success reports its byte hash;
   browser direct-save checks readback, while its download fallback reports only
   Download requested. **Prepare checked flower** uses
   the unchanged real job and `real_data/machine-profile.json`; native M6 checks
   passed and retained `combined.ngc` with 22,883 motions and two tool changes.
9. Compare the DOM page: two labelled controls, reorder, type partial text while
   the same Rust worker runs, and cancel. Browser accessibility exposes these
   controls; the egui canvas does not. This earlier comparison is optional and no
   longer a framework gate. Then open `/web/input-probe.html`: it
   starts the application, drops a real `File` through a real `DataTransfer`,
   sends `Ctrl+F` and IME composition events, and asserts the published state
   snapshot. A real OS IME tour and a real OS drag gesture remain manual; native
   and browser screen-reader tours are excluded.

## Checks and evidence capture

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo test --locked --test paging
cargo test --locked --test interaction dense_layout -- --ignored
node web/capture.mjs
node web/capture-build.mjs
node web/compare-simulation.mjs --preview-only
node web/compare-wasm-simulation.mjs
node web/smoke-browser.mjs      # real browser; start `node web/serve.mjs` first
target\release\cam-gui1-desktop.exe --measure docs\flat-v-carve\gui1-evidence\perf-measure.json flower
```

The ignored image tests are opt-in GPU tests, separately executed locally at
1280×800 and 1440×900. Goldens are checked in. Regenerate intentionally with
`$env:UPDATE_SNAPSHOTS='true'`, then remove that variable and rerun for a real
comparison. The behavior harness includes a negative assertion control. The
goldens cover dense shell layout, not custom viewport geometry correctness.
`capture.mjs` records five small planning runs, a separate checked-output run,
one flower run, input hashes and five native process cancellation samples.
Use `--capture <directory> flower export` on the executable to investigate the
flower output contract separately; its native report is captured in the evidence.
This establishes software checks, not a new physical machining trial.

`--measure <path> [flower]` writes the S/M/L/flower measurement report used by
the paging evidence: transport sizes, JSON-equivalent estimates, per-page
fingerprint and copy costs, page-budget admission, picking latency against a
brute-force oracle, checkpoint-bounded and cold scrub times, dirty-tile counts
and whole-Rust-heap counters. `--dump-scene <folder> [flower]` writes the exact
worker frame so `compare-wasm-simulation.mjs` can compare the native and browser
transports byte for byte. That comparison records a real finding: the f64
planner output is not bit-identical across targets (41 of 22,883 flower motions
differ by one ULP), while rendered vertices, section tables, tile versions,
every packed cell and the final field checksum agree.

`smoke-browser.mjs` is the only check that runs the real browser build: it
launches headless Chrome (or Edge with `--browser=edge`), loads both builtin
references, clicks the viewport, sends a real `Ctrl+F`, delivers an IME commit
to the focused field and fails on any console error or exception. It exists
because a wasm-only panic leaves the page as a static image while every native
test still passes; it needs `pkg/`, a running `web/serve.mjs` and a WebGPU-capable
browser, so it is opt-in like the golden layout tests.

Open `/web/platform-probe.html` for real IndexedDB conflict/abort tests, the
browser's own storage estimate and a bounded real quota attempt, plus the
qualification facts.
explicitly injected browser save-handle tests. See the
[files/input evidence](../../../docs/flat-v-carve/gui1-files-input-evidence.md)
for native destination failures, actual dialog/restart and browser offline/drop
checks. After a manual `wasm-pack build`, run `node web/write-offline-manifest.mjs`
before serving; `launch.ps1 -Target web` does this automatically. Reload again
after a new offline worker activates to use its current asset cache.

## Boundaries and remaining risks

- No domain crate or incumbent UI was changed. Schema-4 migration is captured
  through the existing service; the spike's open adapter supports legacy combined
  jobs only and rejects unsupported input without replacing its prior result.
- The narrow Rust heightfield port matches the original TS on full-cell native
  comparisons; the actual WASM module matches every preview checkpoint. See
  [simulation evidence](../../../docs/flat-v-carve/gui1-simulation-evidence.md).
  The 512-cell preview preset is explicitly coarser (flower 0.3609375 mm versus
  0.025 mm reference), has at most 18 checkpoints and a 20 MiB retained-cell cap.
  Arbitrary stock seeks, display picking, the blade glyph and marker, page
  residency and incremental tile transport now exist with the measurements in
  the [paging/viewport evidence](../../../docs/flat-v-carve/gui1-paging-viewport-evidence.md).
  What remains: a sustained two-minute M frame-time run, real GPU transfer
  timing, GPU memory, the JS heap and a browser tab total.
- Scene transport is paged and binary (metadata plus one sectioned payload).
  Draft serialization on save and the browser's own JSON/recovery paths are
  unchanged and still need their own budgets; the payload copy into WASM memory
  and the worker's transferable buffer are counted but not benchmarked as a
  network transfer.
- Source/operation lists are synthetic identity probes, not an artwork collection.
  No machine settings, schema fields, or output eligibility are invented by them.
- Session recovery and file-drop adapters are implemented and tested within the
  bounds documented above. OS drag gestures, physical browser quota/eviction,
  real browser destination outcomes, shared-library conflicts and OS IME remain
  unverified, although the browser probe now injects a real `DataTransfer` drop
  and real composition events and the platform probe records the browser's own
  storage estimate plus a bounded real quota attempt. Local recovery is best effort and
  can lose edits closed before its debounced write completes.
- Native and browser screen-reader support is outside the required scope. Other unfinished
  probes still gate qualification. Next work continues the egui prototype and
  simulator comparison described in the decision.

Fonts: epaint's bundled Hack, Ubuntu Light, Noto Emoji, and emoji-icon font.
Their license texts accompany the crate; packaged redistribution must carry the
applicable font notices as well as Rust dependency notices. No external icon pack
or generated visual asset was added. See `THIRD-PARTY.md`.
