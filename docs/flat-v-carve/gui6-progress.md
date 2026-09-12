# GUI6 passive knife workflow

Status: GUI6a/b implemented and ready for manual review; user review pending.
Starting commit: `21902923ca1a8a12d84f586ebf1c1404b88ccef9`.
Ending source: the uncommitted worktree on that commit. The plan reorder and
GUI5 review-reference edits were already present when GUI6 implementation began.

The standalone workflow now imports stroked open/closed SVG chains, assigns
qualified source references explicitly, configures a knife without a milling
tool, generates through the existing core/service, displays unchanged-stock
playback, prepares exact checked output, exports and reopens portable jobs.
The previous Flat V-carve workflow remains available.

User review found missing operation add/delete controls. The Operations list
now offers Delete operation and Add operation (Flat V-carve or drag knife).
The workspace accepts zero or one operation, including empty-job save/reopen
and recovery. Replacement preserves artwork bytes/placement, stock, tools,
machine settings and shared raw drafts; Undo/Redo restores the previous state.
Adding a knife starts its cutting values and selection unset. It enables
centerline import interpretation without rewriting filled artwork as strokes.

Further review requested knife-derived defaults and usable geometry selection.
Cutting now exposes Geometry to cut, with per-source checkboxes, plus viewport
click/Shift-click assignment. Create knife outlines makes an explicit stroked
copy of core-imported filled boundaries (including holes) at the current import
tolerance and placement. Original artwork stays intact and the copy is not
automatically assigned. Source coordinate conversion respects SVG's downward Y
axis and setup placement; the rotated/scaled outer-and-hole regression passes.

Suggested operation values derive pass depth from the knife capacity and
profile limit. Swivel depth (10% of effective pass) and corner angle (20°) are
clearly labeled editable starting suggestions. Applying fills blank fields only,
preserves explicit/partial values, and supports Undo. Feeds, target cut depth and
physical heading remain explicit. No new material-specific cutting claim is made.

A later review follow-up moved the Flat V-carve geometry selection out of the
artwork panel: each operation now owns its geometry exactly as the knife does,
with the selection list, Select all/Clear, viewport click/Shift-click assignment
and explicit unresolved-reference repair living in the operation. See
[operation geometry selection](operation-geometry-selection.md) for the design,
the command contract and the verification record.

## Contracts and implementation

F3 compensation, passive-blade replay and contact checks remain in core. H4/H5
retained execution and checked-output transport remain authoritative. The
retained service attaches bounded F3 evidence to the same prepared OneProgram
bundle, with its actual program SHA-256 and execution fingerprint. It does not
replan just to produce a display report. Output precision/work-zero changes
rebind through preparation; machining changes require new generation.

`knife.rs`, `knife_ui.rs`, `knife_library.rs` and `viewport_knife.rs` extend the
production GUI, with changes to shared session, raw editing, resources,
inspection, simulator and viewport adapters. The existing schema-5 job and
typed knife resource contracts are unchanged. The internal GUI command protocol
adds knife import, selection and start commands. New numeric fields retain raw
partial text, copied profile reset and portable recovery semantics.

Knife simulation uses an explicit non-removing tool. Planned headings/tips are
projections of core motions; emitted samples come from core replay of the actual
saved bytes. The renderer does not implement a blade solver or G-code parser.
Knife seam choices bind the core catalogue's qualified reference and source
fingerprint, and core still validates their machining eligibility.

## Evidence and acceptance audit

| GUI6 requirement | Evidence |
| --- | --- |
| Open and closed sources; explicit selection and unset inputs | `tests/knife.rs`, real browser import tour |
| Multiple source ownership; no implicit assignment; replacement stays unresolved | Knife integration test using Add/Replace and source revision checks |
| Knife-only geometry, stock, raw depth/heading edits and portable reopen | Integration, native worker, all-panel egui and browser tests |
| Typed knife library/profile application and copied-baseline reset | GUI6 browser library tour using actual saved revisions |
| Start, depth, corner, swivel, alignment and closure controls | Source-anchor test, browser start selection, native entry navigation and rendered inspector |
| Intended/pivot/actual-byte replay, heading readout and bounded sample disclosure | Core F3 report attachment, overlay visibility/staleness test and browser screenshots |
| Unchanged stock during forward/backward knife playback | Frame checksums, removed-volume assertions, native/browser playback |
| Precision/work-zero freshness and exact export | Integration reprepare without extra planning, browser saved SHA check, native file SHA check |
| Synthetic and chosen real source | `chains.svg` (96 motions), explicit flower centerline derivative (813 motions) |
| Previous milling workflow preserved | GUI suite plus GUI3/GUI4/GUI5 and canonical flower browser regression results below |

## Checks

- Defaults/geometry follow-up: all 53 GUI library tests, five knife integration
  tests and the operation lifecycle integration test pass (59 total), log
  `artifacts/gui/gui6-authoring-tests.txt`. Clippy with warnings denied passes.

- Full GUI suite passed 89 tests after the operation lifecycle changes:
  `cargo test -p cam-gui --locked --lib --tests`, log
  `artifacts/gui/gui6-tests.txt`. Focused anchor, panel and overlay checks are
  also recorded in `artifacts/gui/gui6-final-tests.txt`. The final edit-group
  reset passes both lifecycle UI tests in `artifacts/gui/gui6-lifecycle-tests.txt`.
- Retained service: `cargo test -p cam-service --locked --test retained`, five
  passing tests, `artifacts/gui/gui6-service-tests.txt`.
- Clippy with warnings denied: `cargo clippy -p cam-gui -p cam-service
  --all-targets --locked -- -D warnings`, `artifacts/gui/gui6-clippy.txt`.
- Actual native Windows window tour: opened `knife.job.json`, generated 96
  motions, sought to initial stock and an entry, prepared and saved exact output.
  `artifacts/gui6/native/native-knife.ngc` is 3,400 bytes, SHA-256
  `5d070daa8efb38d28b04640167519d7d6c9734a4821bc4987112fdb0163c6d73`, matching
  the displayed checked-output hash. The user stopped further Computer Use with
  Escape after this passed. No physical machine was controlled.

Browser tours use real local Chromium/WebGPU and the WASM Worker, with isolated
test profiles. The final tours identify Chrome 152.0.7977.83 on Windows x86_64
and an NVIDIA Ampere WebGPU adapter (Chrome does not expose the device name).
Native uses Windows x86_64 eframe/wgpu plus the worker executable.
The final source adds start controls, knife Move gestures and operation lifecycle
controls after the native window tour; those additions are covered by shared
egui/core tests and browser tours. Final native window interaction is left for
user review.

Successful browser evidence, all with zero recorded console errors:

| Tour / command after `node crates/cam-gui/web/smoke.mjs` | Evidence directory under `artifacts/gui/browser-smoke` |
| --- | --- |
| Final defaults/geometry: `--knife-authoring --port=9341` | `2026-09-12T13-46-18.778Z` |
| GUI6 after defaults/geometry: `--gui6 --port=9342` | `2026-09-12T13-43-23.442Z` |
| Lifecycle and full GUI6: `--lifecycle --port=9341` | `2026-09-12T13-23-31.973Z` |
| Operation lifecycle and full GUI6: `--lifecycle --port=9341` | `2026-09-12T13-18-47.501Z` |
| Milling after lifecycle changes: `--port=9345` | `2026-09-12T13-20-57.677Z` |
| GUI6: `--gui6 --trace-io --port=9341` | `2026-09-12T12-57-11.936Z` |
| GUI3 artwork: `--gui3 --port=9343` | `2026-09-12T12-56-47.498Z` |
| GUI4 collection/setup: `--gui4 --port=9344` | `2026-09-12T12-37-47.761Z` |
| GUI5 resources: `--gui5 --port=9342` | `2026-09-12T12-46-50.343Z` |
| Canonical flower milling: `--port=9345` | `2026-09-12T12-51-05.211Z` |

Each directory contains `evidence.json` and rendered screenshots. The lifecycle
tour uses the updated add/delete workflow, then completes the GUI6 authoring,
generation, playback, exact export, library and real-source tour. Earlier tours
predate the operation lifecycle changes; the shared Rust suite also passes.
The final defaults/geometry tour matches the packaged offline build below and
exits successfully, including temporary-browser cleanup. It verifies viewport
assignment, Shift-click, Undo, defaults, custom values, filled-outline selection,
generation, checked output, save and reopen. The full GUI6 regression passed
before the final geometry-list width adjustment. Both record zero browser errors.
The knife screenshots were visually inspected: both synthetic
chains and the selected flower chain show actual-byte replay over intact stock.

The four core knife suites (`knife_geometry`, `knife_import`, `knife_output`,
`knife_stage`) also pass using `cargo test -p cam-core --locked` with those four
`--test` arguments. Log: `artifacts/gui/gui6-core-tests.txt`. They cover source
semantics, compensation, rejection cases, heading/effect metadata and output
process-state/readback constraints.

Browser regression corrections preserve the tested behavior: drag tolerance is
derived from one actual viewport pixel, clicks include a pointer move and
separate input frames, blank edits use Backspace, and missing-field navigation
asserts its required typed issue rather than a historical total issue count.
The file chooser sets files immediately on the CDP event, and CDP errors now
fail the harness explicitly instead of being discarded.
The milling follow-up completed all workflow checks with zero browser errors;
its process then reported a Windows lock while deleting its temporary Chrome
profile. Cleanup now retries the same temporary directory after browser shutdown.
Knife source movement also exposed and fixed a production assumption that Move
required a filled region. Knife mode now allows dragging the chosen source.

Final native SHA-256:
`70b86012645a5a1298b068157b6b219fd5605274b02dafbe8a3bfa36899e0914`.
Final browser offline build:
`a14eaf9575ebb1714512370c3e954c48f33a06a6b210386e65722b10ccba896b`.
`artifacts/gui6/review/manifest.json` records the source input hashes and both
package identities. This source tree is uncommitted; the manifest identifies
the actual files rather than attributing them to HEAD alone.

## Review handoff

Agent inspection feedback:

| Area | Finding and disposition |
| --- | --- |
| Trace visibility | Intact stock hid cutting-depth lines; fixed with a labeled top projection and actual-Z readout. |
| Terminology | Replaced raw JSON numbers/offsets with formatted measurements; clarified stroked SVG input and rejected filled-only input. |
| Navigation/input | Added entry/corner jumps and source-start choices; fixed knife Move's filled-region prerequisite; partial text and Undo verified. |
| Density | Long knife forms scroll within the inspector. Bounded replay ranges and full program identity wrap visibly. Further user layout feedback is pending. |
| Camera/scrub | Top and isometric controls retained; start/end/backward seek preserve intact knife stock. Source drag conversion is checked in viewport units. |
| Errors | Missing required values use typed diagnostics; replaced-source references do not silently bind; stale emitted replay is withheld. |
| Enhancements | Arbitrary anchor picking, larger display-detail budgets and mixed-operation navigation can be evaluated in later slices; no physical tracking claim is made. |

User feedback about missing add/delete controls is addressed above. The physical
Escape input ended Computer Use; it is not treated as GUI6 acceptance. No further
native window interaction was performed for this follow-up.

The defaults/geometry review executable is packaged separately at
`artifacts/gui6/review/knife-authoring-native/cam-gui.exe`, because the previous
review executable is open. Launch this new executable for the current changes;
the running previous version is preserved.

Use [the review recipe](gui6-review.md) for launch instructions, exact fixture
settings, supported SVG interpretation and evidence limits. Review builds live
in `flat-v-carve/artifacts/gui6/review`, with previous GUI5 builds preserved in
`artifacts/gui5-before-gui6`. Generated builds, logs and screenshots are local
ignored artifacts; source, fixtures and these reports remain in the worktree.

GUI7 facing/ordered preparation is the next dependent slice. Mixed knife
sequences remain GUI7d and require operation-prefix/stock-context integration.
GUI6 does not establish physical cutting confidence or user acceptance.
