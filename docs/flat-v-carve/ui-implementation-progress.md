# New UI implementation progress

## GUI1 — framework experiment (**complete**, framework accepted with explicit scope limits)

Starting commit: `046a3340b0f10303514e2309d57c3ebe182c83d3`.
Pre-existing untracked work: `.zcode/`, left untouched. No AGENTS.md was found.

User outcome: launch an isolated native/browser candidate, compare the familiar
combined carving, exercise dense editing and failures, and make an evidence-based
framework decision before GUI2. The incumbent application remains the reference.

Manual recipe to qualify: open the flower reference; compare roughing/finishing;
type `-`, `1.` and invalid text; reorder duplicate-named sources and return to the
same operation; resize, change DPI/camera, scrub backward; start/cancel a busy
worker; recover raw text after a failed write; retry saving; test keyboard, IME
on both targets. The user removed browser screen-reader access, then native
screen-reader support, from the required scope on 2026-09-11. Keyboard, focus,
IME and named controls for framework tests remain required.

Implementation checklist:

- Capture immutable small/flower inputs, profile and reference engine results.
- Isolated `flat-v-carve/experiments/gui1` crate and shared egui workspace.
- Stable raw-field identity, explicit experimental data, persistent wgpu callback.
- Disposable native/browser computation, cancellation and failure reporting.
- Recovery/file probes, behavioral and image harness, measured fixture manifest.
- Record platform evidence, unresolved risks, framework decision and GUI2a gate.

Prerequisites: current checklist records H1 done, H2–H6 unimplemented. The released
`ui-8` sequence path still consumes schema 4, while schema-5 model work alone does
not establish collection execution. GUI1 uses existing legacy/core contracts;
no H milestone or production architecture is implemented by this experiment.

## GUI1 result — egui evaluation continues; experiment ready for review

Current direction: **continue egui/eframe evaluation**, recorded in [ui-framework-decision.md](ui-framework-decision.md).
The browser eframe integration does not publish AccessKit controls; both pinned
0.33.3 and current 0.36.2 source show the gap, and the running Chromium accessibility
tree confirms it. A narrow DOM comparison exposes labelled fields/buttons and runs
the same Rust WASM calculation. The earlier Reconsider-the-stack outcome is
superseded by the user's explicit removal of browser screen-reader access as a
requirement (2026-09-11). Retain that observed limitation and comparison evidence;
no DOM switch or accessibility bridge is required for it. Final GUI1 qualification
is complete inside the accepted scope, with keyboard/focus/input behavior and
named controls for tests covered on both targets. The user subsequently extended the screen-reader exclusion to native
targets as well; neither native nor browser screen-reader tours are required.

GUI1a: input hashes, unchanged flower/small references, schema-4 service migration,
actual motion stages and native checked-output reports captured. Flower output
passed; the small contact-line negative fixture failed M5_FLOOR_RIDGE and yielded
no program. The simulator now has a narrow Rust port with native full-cell and
WASM preview comparisons against the original TypeScript. See
[simulation continuation](gui1-simulation-evidence.md) for evidence and limits.

GUI1b: isolated native/browser three-pane prototype, 40 raw input probes, stable IDs,
virtualized operation list, custom persistent wgpu motion callback, top/isometric
views, stages and motion timeline. The actual flower and a stock box now render
on both targets, with bounded stock removal checkpoints and explicit display
resolution. No claim of machining edits, picking, or artwork collection support.

GUI1c: same core calculation in disposable native process and browser Worker;
real native cancellation/typing smoke and five process-stop samples. Automatic
session recovery, revision conflicts, native atomic save/retry, browser direct-save
and download adapters, and file-drop paths now have bounded
[files/input evidence](gui1-files-input-evidence.md). Native real dialog cancellation
restores field focus; browser offline reload restores raw text. Native and browser
screen-reader support is an accepted exclusion; OS IME,
Firefox, renderer recovery, real browser quota/destination outcomes,
arbitrary interactive stock seeks and sustained S/M/L tests remain open.
Full-reference native stock and native/WASM preview
comparisons pass; a 20 MiB checkpoint cap bounds the interactive display preset.

GUI1d: decision updated for the user-approved scope; earlier DOM comparison,
per-target evidence matrix and conditional GUI2a service/workflow checklist retained.
Continue the egui simulation/input/runtime qualification within GUI1 before
promoting the shell into production.

GUI1e (this continuation): paged transport replaces the single JSON scene
document. The worker now returns a small metadata document plus one sectioned
binary payload (`u32 metadata length | metadata JSON | payload`), the browser
hands the payload over as a transferred `ArrayBuffer`, motion geometry is
resident page by page with fingerprint-based skipping and budget-limited
admission, and stock checkpoints upload only the tiles whose version changed.
The display gained translucent selection fill, an endmill/V-bit marker and
DPI-aware picking (index plus exact projected distance, cross-checked against a
brute-force oracle), a bounded replay that makes arbitrary stock seeks possible
between transported checkpoints, a real resource-recreation drill, a captured
wgpu validation error, a counting global allocator and an S/M/L/flower
measurement mode. `web/input-probe.html` drives real browser drop and
composition events into the running application and asserts the published state
snapshot; `web/platform-probe.html` adds real storage-estimate and quota-abort
checks. `web/compare-wasm-simulation.mjs` now compares a native worker frame
with the browser package byte for byte and records the cross-target planner
float finding (41 flower motions differ by one ULP; rendered geometry, tile
versions, every packed cell and the final checksum agree). Full evidence and the
remaining manual checks are in
[paging/viewport evidence](gui1-paging-viewport-evidence.md) and
[perf-measure.json](gui1-evidence/perf-measure.json).

Driving a real browser then exposed two browser-only defects that the native
build could not show: per-frame timing used `std::time::Instant`, which panics on
wasm32 and trapped the whole page on the first loaded scene, and the canvas had
no `tabindex`, so the browser build never received keyboard input. Both are fixed
(`src/clock.rs`, `tabindex` on the canvas, published focus state) and covered by
`web/smoke-browser.mjs`, which loads both builtin references in headless Chrome,
checks canvas focus, sends a real `Ctrl+F` and delivers an IME commit. Still
open: the real OS IME tour, real OS drag gesture, real file-dialog
cancel/confirm, real device loss, a sustained two-minute M frame-time run and the
non-Chromium browser rows.

GUI1f (acceptance): on 2026-09-11 the user accepted the framework after reviewing
both builds ("native build — everything works reasonably"), and the two
browser-only defects found during that review were fixed and covered by a
real-browser smoke test. The recorded outcome is
[**Proceed with explicit scope limits**](ui-framework-decision.md#outcome-proceed-with-explicit-scope-limits):
Windows x86_64 native and desktop Chromium/WebGPU are the qualified envelope,
and the unexercised rows (Firefox, Linux, Safari/macOS, DX12 runtime), the
remaining manual input/file tours, real device loss, sustained frame-time and
browser-total memory are deferred to the slices named in that table.

Technical checks: native and WASM release builds, fmt, warning-free clippy,
48 passing state/behavior/file/simulation/transport/picking/clock tests, two separately executed layout golden
comparisons at 1280×800 and 1440×900, and an opt-in real-browser smoke test
(`node web/smoke-browser.mjs`). Precise commands and limits are in the
[experiment README](../../flat-v-carve/experiments/gui1/README.md).

Review readiness: **Windows/Vulkan and Chromium/WebGPU prototype accepted inside
the recorded scope**; the application itself still labels itself experimental.
Complete GUI1 technical qualification: **achieved** (with the deferred items
listed above owned by later slices). User review: **accepted 2026-09-11**; the
native/browser screen-reader scope exclusion is explicit user feedback and is applied above. The initial experiment was committed
and pushed as `81a9fed`; the simulation continuation was pushed as `11bce97`;
the files/input continuation was pushed as `b527f7a`; the paging/viewport
continuation, the two fixes it needed (native worker message framing, then the
wasm clock and canvas-focus defects) and this acceptance record are on branch
`codex/gui1-viewport-perf` (not yet pushed).
No backend H status was changed.
