# New UI implementation progress

## GUI1 — framework experiment (in progress)

Starting commit: `046a3340b0f10303514e2309d57c3ebe182c83d3`.
Pre-existing untracked work: `.zcode/`, left untouched. No AGENTS.md was found.

User outcome: launch an isolated native/browser candidate, compare the familiar
combined carving, exercise dense editing and failures, and make an evidence-based
framework decision before GUI2. The incumbent application remains the reference.

Manual recipe to qualify: open the flower reference; compare roughing/finishing;
type `-`, `1.` and invalid text; reorder duplicate-named sources and return to the
same operation; resize, change DPI/camera, scrub backward; start/cancel a busy
worker; recover raw text after a failed write; retry saving; test keyboard, IME
on both targets, plus a real screen reader on native targets. Browser screen-reader
access was removed from the required scope by the user on 2026-09-11.

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
remains pending, with keyboard/focus/input behavior and native accessibility in scope.

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
real native cancellation/typing smoke and five process-stop samples. Portable
draft/save and injected failures implemented but not comprehensively qualified.
Browser screen-reader access is an accepted exclusion; IME/native assistive technology, Firefox, device loss,
actual persistence failure/retry, arbitrary interactive stock seeks and sustained
S/M/L tests remain open. Full-reference native stock and native/WASM preview
comparisons pass; a 20 MiB checkpoint cap bounds the interactive display preset.

GUI1d: decision updated for the user-approved scope; earlier DOM comparison,
per-target evidence matrix and conditional GUI2a service/workflow checklist retained.
Continue the egui simulation/input/runtime qualification within GUI1 before
promoting the shell into production.

Technical checks: native and WASM release builds, fmt, warning-free clippy,
nine passing state/behavior/simulation tests and two separately executed layout golden
comparisons at 1280×800 and 1440×900. Precise commands and limits are in the
[experiment README](../../flat-v-carve/experiments/gui1/README.md).

Review readiness: **Windows/Vulkan and Chromium/WebGPU experimental prototype
only**. Complete GUI1 technical qualification: **not achieved**. User review:
**pending** for prototype acceptance; the browser screen-reader scope exclusion
is explicit user feedback and is applied above. The initial experiment was committed
and pushed as `81a9fed`; the simulation continuation is subsequent workspace work.
No backend H status was changed.
