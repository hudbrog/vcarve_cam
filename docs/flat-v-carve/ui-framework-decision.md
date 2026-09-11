# GUI1 framework decision — 2026-09-11

## Outcome: **Proceed with explicit scope limits**

GUI1 is **complete**, and the user **accepted the framework on 2026-09-11** inside
the envelope below. The accepted stack is egui/eframe/egui-wgpu **0.33.3** with
the shared wgpu **27.0.1** viewport, for Windows x86_64 native (Vulkan) and
desktop Chromium with WebGPU. GUI2a may begin within that envelope.

Bare *Proceed* would claim platforms this experiment never ran, so the
unexercised rows of the platform matrix stay **Unverified** rather than Passed,
and the plan's rule still holds: a row is not supported until it is exercised on
suitable hardware.

**Accepted scope (qualified and supported today)**

- Windows x86_64 native: window, Vulkan, RTX 3090, 125% DPI, named controls,
  typing while a calculation runs, cancellation, files and recovery, the GPU
  recovery drill and a captured wgpu validation error.
- Desktop Chromium (152.0.0.0) with WebGPU: the same workflow, plus a real-browser
  smoke test that loads both builtin references, takes canvas keyboard focus,
  runs `Ctrl+F` and delivers an IME commit, with zero console errors.
- Both targets: one Rust document/reducer/service authority, paged binary scene
  transport, DPI-aware display picking, bounded stock replay and export checks
  through the existing core.

**Explicitly outside the accepted scope (unverified, not waived)**

| Deferred item | Owner slice |
| --- | --- |
| Firefox desktop (a required candidate in section 2.1), Linux, Safari/macOS and the Windows DX12 runtime | GUI11 release qualification; evaluating Linux as the next desktop target stays a recorded plan item |
| Real OS IME tour on both targets; real OS drag gesture; real browser and native file-dialog confirm/cancel | The input and file work of GUI2a, re-checked per target |
| Real driver device loss (TDR / `device.lost`) | GUI2a reliability work; the recreation drill plus captured validation error are today's evidence |
| Sustained two-minute M playback frame-time percentiles, GPU transfer timing and GPU memory | The GUI2a performance gate, using the S/M/L harness already in place |
| Browser tab total memory (JS heap + WASM heap + GPU) and real storage eviction | GUI11; the counting allocator covers the Rust/WASM heap only |
| Native and browser screen-reader support | Excluded by explicit user decision on 2026-09-11 |

**One provisional budget was revised, with the reason recorded.** The plan's
"M scrub ≤ 250 ms" is met for checkpoint-bounded seeks (p95 40.7 ms on the 200k
probe) and is **not** met for a cold replay (p95 869 ms). The display therefore
keeps transported checkpoints, replays only a bounded suffix between them, and
reports the cold path as a loading cost instead of stalling silently. Improving
the cold path belongs to the retained-execution contract in the plan's section
3.3, not to the framework choice.

## Scope and provenance

Starting commit: `046a3340b0f10303514e2309d57c3ebe182c83d3`. The only pre-existing
untracked directory was `.zcode/`, left untouched. All implementation is under
`flat-v-carve/experiments/gui1`, with an independent Cargo workspace/lockfile.
No existing CAM crate, source fixture, incumbent UI, or backend checklist changed.
No production UI crate map was committed or selected.

The [GUI1 plan](2.5d-cam-ui-implementation-plan.md#2-gui1-framework-selection-and-risk-reduction)
requires keyboard/focus/input behavior on both targets. Framework tests still use
named controls; native and browser screen-reader support is not a release gate.
Required failures still trigger
the plan's stop condition, but the browser AccessKit gap no longer does.

## Reproduction and review

From `flat-v-carve/experiments/gui1`:

```powershell
./launch.ps1 -Target native
./launch.ps1 -Target web
```

The [experiment README](../../flat-v-carve/experiments/gui1/README.md) contains
the complete five-minute manual recipe, checks, and limits. The browser routes
are `/web/index.html` (candidate), `/web/dom-probe.html` (narrow alternative),
`/web/input-probe.html` (browser input integration probe) and
`/web/platform-probe.html` (storage/quota qualification), served on
`http://127.0.0.1:5181`. The native artifact is
`target/release/cam-gui1-desktop.exe`; the browser package is `pkg/`. Artifacts
are local build outputs, not committed installers. No deployment is required.

The review shell has three resizable panes, independent scroll regions, a
40-field optional/raw-text inspector, source and operation identity probes,
search, a timeline, top/isometric views, actual roughing/finishing motion batches,
and disposable compute adapters. Controls are explicitly experimental: numeric
fields do not mutate the reference job, the two source rows are synthetic, and
stock playback uses a labeled coarse grid and discrete motion checkpoints.
The [simulation continuation](gui1-simulation-evidence.md) records the tested
Rust port, full-cell comparisons, bounded GPU stock preview and remaining limits.
The [files/input continuation](gui1-files-input-evidence.md) records automatic
recovery, revision conflict protection, retained save retry, native modal focus,
browser offline restart, and the distinction between real and injected failures.

## Candidate actually tested

| Boundary | Exact experiment choice |
| --- | --- |
| Shell | egui/eframe/egui-wgpu **0.33.3**; egui_kittest **0.33.3** |
| eframe features | Defaults disabled; `accesskit`, `default_fonts`, `wgpu`, `x11`, `wayland` |
| wgpu | **27.0.1**; defaults disabled; `std`, `dx12`, `vulkan`, `wgsl`, `webgpu`; resolved wgpu-core/hal in Cargo.lock |
| Viewport | egui-wgpu callbacks, persistent motion/packed stock buffers and camera uniforms; lines, stock-cell triangles and box; Depth24Plus; one sample |
| Rust | **1.95.0**, MSVC x86_64; WASM target wasm32-unknown-unknown |
| Browser bootstrap | wasm-pack **0.15.0**, wasm-bindgen **0.2.128**, futures **0.4.78**, js-sys/web-sys **0.3.105**, Node **24.19.0** |
| Browser baseline | WebGPU only. Explicit unavailable message if absent; no WebGL2 fallback claim |
| Native files | rfd **0.16.0** dialogs/background file work; atomic sibling replacement; OS-locked session recovery |
| Browser files | HTML picker, direct-save handle/readback where available, Blob fallback; IndexedDB revision transactions and offline asset cache |
| Execution | Native child process with kill/wait; browser disposable module Worker; request generation gates results; no shared-memory threads |
| Protocol | Private `gui1-spike-3`; worker handshake checks version; same WASM module contains compute and UI entry points |
| CAM | Existing cam-core/cam-service **0.7.7**, legacy combined planning and retained-receipt checked export; no new machining API |

0.33.3 was a bounded, explicitly pinned API baseline, not a claim to be the latest
release. The latest eframe **0.36.2** was also downloaded and inspected specifically
for the discovered accessibility gap; upgrading does not remove it. It was not
substituted into the tested build. Lockfile captures all transitive versions.

License/distribution inventory is in the experiment's
[THIRD-PARTY.md](../../flat-v-carve/experiments/gui1/THIRD-PARTY.md) and `licenses/`.
There is no separate icon pack. Font notices are copied alongside the experiment;
an installable distribution still needs a target-specific dependency notice bundle.

## Browser screen-reader limitation and earlier comparison

1. The pinned eframe web app runner destructures `accesskit_update: _` with the
   comment `not currently implemented` ([0.33.3 source](https://docs.rs/eframe/0.33.3/src/eframe/web/app_runner.rs.html)).
   Current 0.36.2 does the same ([0.36.2 source](https://docs.rs/eframe/0.36.2/src/eframe/web/app_runner.rs.html)).
   Local registry source was inspected at lines 328 and 394 respectively.
2. On the running Chromium build, the accessibility tree was a web area, the
   named canvas image, and one generic text field. It had **no named CAM buttons,
   operation controls, or numeric fields**, while the screenshot showed them.
   This is an observed browser semantics failure, not a claim based only on docs.
3. On Windows native, UI Automation exposed named buttons, editable Maximum depth,
   stage selectors and sliders. This is useful integration evidence but not a full
   Narrator/NVDA qualification. UIA `set_value` automation hit a cache-property error;
   focus plus ordinary text input worked, with `-` visible while the child was busy.
4. A deliberately small DOM alternative exposes two separately labelled fields,
   reorder/generate/cancel buttons and a live status. In the same browser, raw `-`
   survived source reorder, `1.` stayed editable while busy, cancellation worked,
   and the same Rust WASM worker returned the small reference's complete summary.

The DOM probe contains no CAM or stock algorithms and is not a second production
UI. It is retained as evidence of the earlier comparison, not the next selected
implementation path. It does not qualify a native webview package or replacement
framework. The user's scope revision removes the reason to switch shells or add
an egui-to-DOM accessibility bridge for this limitation. Continue the existing
egui prototype and its required native/browser input and named-control tests.

## Reference and measured results

Immutable input hashes, byte counts and captures are in [gui1-evidence](gui1-evidence/measurements.json).
The original flower job, accompanying SVG and `real_data/machine-profile.json`
were used unchanged. The embedded SVG hash is also recorded separately by the
engine capture; a filename is not used as identity.

| Reference | Actual result |
| --- | --- |
| Flower | 15 selected rings; 1 operation, 2 tools; 7,048 roughing + 15,835 finishing motions; 65,650 contour/motion vertices; 1,838,200 vertex bytes |
| Small contact-line | 1 selected ring; 1 operation, 2 tools; 0 roughing + 37 finishing motions; 82 vertices; 2,296 vertex bytes. Empty roughing is the fixture's real geometry result |
| Canonical capture | Existing `ui-8` Open migration projection recorded in each reference JSON; schema-5 model implementation does not establish collection execution |
| Flower output | Native checked export **passed**, one `combined.ngc`, 22,883 motions, 2 tool changes; SHA256 `c190feced004bb42e67a5da97e1c108b4897a750132a8008551bc1ce05d3997e` |
| Small output | Native checked export **failed**, `M5_FLOOR_RIDGE`, zero programs. This negative fixture was not loosened to manufacture a pass |
| Native render smoke | Actual flower displayed in Vulkan, raw text editable while busy; cancellation retained previous display; one UI stop/reap report 2.23 ms after supervisor observation |
| Browser render smoke | Actual flower displayed in WebGPU, same 65,650 vertices; small reference fingerprint and motion fingerprint matched native in the visible worker summary |

[Flower planning capture](gui1-evidence/flower-reference.json),
[flower checked-output capture](gui1-evidence/flower-output.json),
[small planning/output capture](gui1-evidence/small-reference.json).
These are software results, not physical trial claims or general output qualification.

The initial headless capture had five small-plan times **5.0974, 4.1698, 3.5136, 3.7731,
3.5349 ms** (median 3.7731; max/nearest-rank p95 5.0974). A separate small checked
output run took 5.2604 ms. One flower plan took 2838.8529 ms and its separate native
plan+checked-output run 4660.7706 ms. These exclude interactive transport/rendering.

Five native kill-to-exit samples were **2.3600, 2.2291, 2.2357, 2.2041, 2.4302 ms**
(median 2.2357; max/nearest-rank p95 2.4302), after a 150 ms busy period. An earlier
run under concurrent builds was 2.43–4.06 ms. These are stopped-process samples,
not input-to-visible timing or proof about a browser's stopped CPU latency.

One interactive flower request, during other build/test work, took 12,990.9 ms
native and 11,028.9 ms browser including transport. One localhost browser shell
startup measured 429.6 ms. Neither is a five-repeat cold-start benchmark.
The initial optimized WASM was about 6.29 MB raw / 2.59 MB gzip; native executable
was about 16.67 MB. [Build sizes and hashes](gui1-evidence/build-manifest.json) record
the exact artifacts. Gzip size is an estimate of compression, not measured
network transfer; the local server serves uncompressed assets.

S/M/L generators use seed `0x5eed`, 20k/200k/1m line segments, exactly two vertices
per segment (28 bytes each). They are now paged: the transport, page residency,
revealed in the [paging/viewport continuation](gui1-paging-viewport-evidence.md)
and [perf-measure.json](gui1-evidence/perf-measure.json). Measured highlights:
metadata stays in the kilobyte range while the payload is a single binary
transfer (7.1 / 30.4 / 58.4 MB for S / M / L); reloading the same payload copies
no scene bytes; a 16 MiB page budget admits 37 of 123 L pages and reports the
rest as omitted; picking p95 is 0.004 / 0.066 / 0.42 ms with 0 index/brute-force
disagreements on 3,840 camera/DPI combinations; checkpoint-bounded stock scrubs
are p95 5.2 / 40.7 / n/a / 8.3 ms (S / M / L / flower) while the cold replay
path costs 62 / 869 / n/a / 253 ms. The counting global allocator reports
whole-Rust-heap peaks of 31.9 / 142.4 / 245.5 / 117.1 MB. A sustained two-minute
M playback, GPU transfer time, GPU memory, the JS heap and a browser tab total
remain unmeasured, so no sustained-performance or total-memory pass is claimed.

## Platform and test matrix

| Target | Evidence | Qualification |
| --- | --- | --- |
| Windows x86_64, OS build 10.0.26200 | Native window, NVIDIA RTX 3090, NVIDIA 595.79, Vulkan, 125% DPI; flower, named controls, typing while busy, Cancel | Native screen-reader support excluded by user; experimental review ready, input/files/renderer qualification incomplete |
| Windows DX12 | Feature compiled | Unverified runtime |
| Chromium 152.0.0.0, Windows (headless smoke) and the Codex in-app browser | WebGPU shell + flower, canvas-only AX tree; earlier DOM comparison labels/input/worker/cancel; 125% DPI; headless smoke loads both references, checks canvas focus/`Ctrl+F`/IME commit and reports 0 console errors | Browser screen-reader access excluded by user; experimental review ready, remaining required checks incomplete (real OS IME, real drag gesture, file-dialog confirm/cancel) |
| Browser GPU/driver | WebGPU adapter identity redacted by browser | Unknown; do not copy native GPU identity into this row |
| Firefox desktop | No actual run | Unverified required candidate |
| Linux, Safari/macOS | No actual run/package | Unverified evaluation targets |

Executed: `cargo fmt --all -- --check`, native release build, wasm-pack release
build, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`,
the two opt-in GPU image comparisons, and the opt-in real-browser smoke test
(`node web/smoke-browser.mjs`, which writes `gui1-evidence/browser-smoke.json`).
The current experiment has 48 passing
behavior/state/file/simulation/transport/picking/clock tests; two
layout goldens pass at 1280×800 and 1440×900. The UI harness finds controls by
accessible label and asserts draft identity after actual button actions. Its
negative assertion control is detected. Image baseline generation was followed
by a separate comparison run with update mode unset. Goldens cover shell layout;
custom viewport correctness was visually smoke-tested, not pixel-qualified.

## Deferred beyond the accepted scope, with the evidence collected so far

These are the items the accepted envelope excludes. Each has an owner slice in
the outcome table above; none of them is claimed as passed.

- Actual OS IME tours and browser keyboard-only dialog completion/cancellation.
  Native Ctrl+O/Escape focus restoration, harness IME/Tab/reorder focus, and a
  headless-browser smoke test that gives the web build canvas focus, sends a real
  `Ctrl+F` and delivers an IME composition/commit into the focused field all
  pass. The web build needed `tabindex` on the canvas before any of that worked,
  and per-frame timing needed a wasm-safe clock; both defects are fixed and
  covered by `web/smoke-browser.mjs`. A real OS IME tour and real file-dialog
  cancel/confirm remain manual.
  Native and browser screen-reader tours are out of scope.
- Implemented and measured since the previous revision: translucent selection
  fill, blade glyph, display picking with a physical-pixel tolerance at several
  DPI values (indexed versus brute-force oracle), paged motion and tiled stock
  transfer, arbitrary stock seeks between transported checkpoints, bounded page
  residency with reported omissions, and S/M/L transport/paging/picking/scrub/
  heap measurements. See the continuation evidence for the exact numbers, the
  M cold-seek limit and the 512-cell display preset.
- Actual device loss. A real GPU validation error is now captured and reported,
  and a resource-recreation drill re-uploads from retained CPU data; driver
  device loss (TDR / `device.lost`) is still not exercised.
- OS drag gestures, physical browser quota/eviction and destination outcomes,
  shared-library revision conflicts, and full browser checked-flower save/retry.
  Session revision conflicts, actual native failure/retry, automatic recovery,
  browser offline reopen and synthetic browser drop events now have bounded evidence.
- Whole-process allocation accounting for the mixed JS/WASM/GPU browser case,
  sustained frame-time performance, all required browser rows, installable
  packaging and full asset/license distribution checks.

The TypeScript heightfield now has a narrow Rust port preserving 256×256 lazy
Uint16 tiles, thickness/65535 quantization, tool ownership and analytic tool coverage.
Native full-grid and actual WASM preview comparisons pass; see the separate
simulation evidence for exact prefixes, timing and scope limits. No JS runtime
was added to the application for simulation.

## Accepted decision and GUI2a contract

The decision is resolved: **Proceed with explicit scope limits** for
egui/eframe with the shared wgpu viewport. The narrow heightfield port, bounded
preview and paged transport are established. The deferred items above are
feature-specific risks carried by their owner slices, not blockers for GUI2a
inside the accepted envelope. Keep one Rust
document/reducer/service authority. React/Three.js
remains the compatibility and simulation reference; a DOM/native-host evaluation
is no longer required for browser screen-reader support. Ratify the platform
matrix again (with real runs) before using any currently Unverified row.

GUI2a's first-slice checklist, conditional on a resolved framework decision:

1. Use the released schema-4 single-source/single-Flat-V-carve path unless a newer
   fully working open/generate/prepare/save path is verified. H1 alone is insufficient.
2. Open the unchanged flower job and explicitly apply/load the user's schema-1
   profile through the existing migration to the schema-2 session profile. Keep
   one machine authority; do not hide it in schema 4 or supply missing values.
3. Bind live feed/geometry settings through typed commands with stable raw drafts,
   Undo and freshness. Reject unsupported collection/multiple-operation input
   before replacing the current draft.
4. Add the plan section 3.3 narrow service-owned retained execution/check/output
   contract. Stateless `Motions`/`Export` replanning is not a scrubbing/retention API.
5. Connect actual combined endmill/V-bit heightfield playback, stage navigation and
   backward scrubbing; compare the established simulator with the same job/motions.
6. Prepare/check from the retained plan, keep exact bytes for save/retry, distinguish
   failed checks, stale results, cancellation and actual destination outcomes.
7. Deliver open → edit → generate → simulate → prepare/export → save/reopen on both
   qualified targets, with native/browser failure fixtures and manual review evidence.

Technical status: **GUI1 complete**; framework outcome **Proceed with explicit
scope limits**, accepted by the user on 2026-09-11. Review readiness: the
native/Chromium prototype is the accepted reference for GUI2a, still labeled
experimental inside the application. No H milestone status changed, and GUI2a is
the first dependent production slice.
