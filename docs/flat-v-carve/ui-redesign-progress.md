# UI redesign implementation

The scope and acceptance criteria are in the [visual redesign plan](ui-visual-redesign-plan.md). Each V milestone is committed independently. Optional conveniences listed in the plan remain separate from this work.

| Milestone | State | Evidence |
|---|---|---|
| V0 baseline | Complete | Fresh native/browser captures; 111-field inventory; baseline tests |
| V1 visual foundation | Complete | Shared tokens/icons, bounded forms, Stock sections and live datum diagram; 167 tests |
| V2 workspace shell | Complete | Compact header, one ordered row per operation, anchored job tools, scope/status, adaptive panels; 171 tests |
| V3 resources | Complete | Full-width Library/Machines/Job tools, parameter-driven diagrams, contextual staged picker; native and browser resource acceptance |
| V4 authoring | Complete | Four operation editors, Artwork/Machine/Job settings, procedural diagrams, stable operation views, job-wide T/H drafts; native and browser acceptance |
| V5 inspection | Complete | Bounded view/transport controls, time track, warning markers, Summary/Section/Warnings, seek regressions; native/browser acceptance |
| V6 completion | Complete | Exact-scope export review, owner-aware issue routing, startup/recovery/failure actions; native/browser/DPI acceptance and 19-area audit |

## V0 — reproducible baseline

Application source baseline: `8236ae4`. No styling changes in this milestone. The feature-gated native review harness and browser scenario render the existing application with `fixtures/gui4/lettering.job.json`; the resource view uses `fixtures/gui5/library.json`. Native review disables recovery and seeds a private in-memory catalog. Browser review runs in a new temporary Chrome profile. Neither uses the user's live document or catalog.

Representative checked-in evidence:

- [Native stock, 1280×800 at 1×](ui-redesign-evidence/v0/native-stock.png).
- [Browser stock, 1280×800 at 1×](ui-redesign-evidence/v0/browser-stock.png).
- [Browser simulation, 1440×900 at 1×](ui-redesign-evidence/v0/browser-simulation.png).
- [Browser library, 1440×900 at 1×](ui-redesign-evidence/v0/browser-library.png).

Full local evidence: `flat-v-carve/artifacts/gui/visual-v0/` (native PNG and JSON pairs) and `flat-v-carve/artifacts/gui/browser-smoke/2026-09-18T06-49-39.052Z/` (22 browser captures plus state/performance metadata). Browser build ID: `75300797693e`. Both use the same pinned egui default fonts and top camera; simulation is paused at final stock. Native review is a debug build and browser is optimized WASM without wasm-opt, so their timings are not cross-target performance comparisons. Frame metrics in the JSON establish observations for matched subsequent runs, not an FPS guarantee.

Browser coverage: Artwork, Stock, Cutting, Machine at 1280×800, 1440×900, 1920×1080, and 1280×800 at device scales 1.5 and 2; final-stock simulation and imported Library at 1440×900. Native coverage: artwork, stock, operation, machine, library, job tools, simulation at 1280×800, plus Stock at the other sizes/scales. Final audit must also exercise reduced logical width at 200% OS scaling; device-pixel scaling alone does not prove narrow-layout support.

Baseline findings: the large blank area above Stock numeric inputs is reproducible on both targets; horizontal `with_layout` centers rows in all remaining vertical space. The default inspector can grow to accommodate its contents, so matching outer dimensions does not guarantee identical panel width. Operations and assigned tools overflow the navigator vertically. View toolbar controls overflow on narrow center panels. These are defects to fix, not golden behavior to preserve.

Validation: `cargo test -p cam-gui --lib --locked` passed 166/166; `cargo clippy -p cam-gui --features ui-review --locked -- -D warnings` passed; the browser `--visual-review` scenario passed with no console errors. The first capture-script attempt clicked before resize had settled and used a V-bit jump for an Endmill-only fixture; the script was corrected and rerun. The passing run above is the authoritative evidence.

Reproduction, from `flat-v-carve`:

```powershell
cargo build -p cam-gui --features ui-review --locked
./target/debug/cam-gui.exe --ui-review fixtures/gui4/lettering.job.json artifacts/gui/review/native-stock.png stock 1280 800 1
./scripts/build-gui.ps1 -Target web -NoOpt
# Separate terminal:
node crates/cam-gui/web/serve.mjs
node crates/cam-gui/web/smoke.mjs --visual-review
node scripts/ui-field-inventory.mjs
```

The native review arguments are JOB, OUTPUT.png, PANEL, logical WIDTH, logical HEIGHT, SCALE. Panels: artwork, stock, machine, settings, operation, library, tools, simulation. It uses the actual egui/wgpu frame, writes observed control rectangles and renderer statistics, and exits. Its image encoder is optional and absent from the normal build.

## V1 — visual foundation and Stock

Shared presentation now lives in `ui_theme.rs`, `ui_icons.rs` and `ui_widgets.rs`: semantic colors, 14/12-point text, 28-point bounded numeric rows, flat section headings, scope badges and a table-header primitive. The [asset manifest](ui-asset-manifest.md) records the original vector icon family and procedural stock schematic. New inspector sessions start at 344 points; saved widths are retained.

The numeric renderer keeps existing field identities, raw text, units, help, diagnostic focus and undo/recovery paths. It replaces the unbounded horizontal layout that centered each field in all remaining vertical space. A regression test checks input height, row spacing and the first field's position across 800/900/1080-point window heights.

Stock is grouped into Dimensions, Position, Work zero, and Clearance & start. Existing page/bounds capture, resize anchors, unset commands and custom XY remain available. There is one documented Z0 selector. The schematic labels actual committed thickness, clearance above stock, top and bottom coordinates for the selected datum. Help sits beside the section heading. The panel scrolls to its lower sections; compact shell/toolbar and other authoring-panel layout changes remain V2/V4/V5 work.

Evidence:

- [Native Stock at 1280×800](ui-redesign-evidence/v1/native-stock.png) and [Work zero at 1440×900](ui-redesign-evidence/v1/native-work-zero.png).
- [Browser Stock at 1280×800](ui-redesign-evidence/v1/browser-stock.png) and [bottom-datum Work zero](ui-redesign-evidence/v1/browser-work-zero.png).
- Full native PNG/metadata: `flat-v-carve/artifacts/gui/visual-v1/`, including 2× DPI. The review panel `stock-zero` applies a lower scroll position after initial window layout.
- Full browser evidence: `flat-v-carve/artifacts/gui/browser-smoke/2026-09-18T07-13-09.176Z/`; build `e42ae3d3eb21`. This repeats V0's sizes/scales and adds real-input assertions for partial thickness, navigation, two-step Undo, and both Z datum choices, followed by generation, final-stock simulation and library import. It passed with no console errors.

Validation: 167/167 `cam-gui` library tests; clippy with `ui-review` and warnings denied; native/WASM builds; formatting and diff checks; all 111 field IDs still mapped. The final help-heading adjustment was also checked with both setup-tab tests and the complete browser capture scenario. Native review remains debug, so cross-target timings are not a performance comparison.

One older `--authoring` scenario was attempted and stopped at its initial import assertion: it expects absent machining probe properties to equal null on an artwork-only import, although current imports correctly contain zero operations. It did not reach any modified field. The current-schema checks above are the V1 acceptance evidence. A capture-script attempt also clicked a stale field-filter rectangle; waiting for the real layout to settle fixed the script without changing application behavior.

## V2 — workspace shell

The graphite header now combines File, compact Save/Undo/Redo, an elided document name with a separately visible document state, Prepare/Simulate, Generate all, an attached scope menu and Export. The scope menu can generate all enabled operations or through the selected operation; the work-area strip shows the active generation scope or the retained result's scope. Prefix export stays visibly named. The operation footer uses neutral actions and reserves enough space above the status bar, including at 640×400 logical points.

The 248-point resizable navigator has fixed Setup controls, a bounded Artwork/Operations scroll, and anchored job-wide tools with Manage and a Global library link. Every operation appears once, with enable checkbox, ordinal/type icon, name, cutter summary, located-issue text and an actions menu. Rename, move earlier/later, generate-through and delete keep their existing commands. Artwork has independent eye/lock targets with explicit display/picking semantics. Inspect result is reached through Simulate. The selected operation's legacy geometry inspectors remain available through the explicitly labeled Selected operation tools menu.

Panels have persistent widths/collapse state with backward-compatible recovery defaults. Below 960 logical points, opening one side panel closes the other; both can be closed to expand the canvas. Short windows get Setup pages and a compact job-tool summary. The normal status row is 24 points; recovery offers/failures and save retry still expand into visible actions. This follows revision 2's hierarchy while retaining the actual job settings, operation enablement and execution-scope controls absent from the image.

The zero-operation acceptance check found an old resource guard that forcibly closed Job tools and replaced the library with an add-operation message. Job-wide geometry and the global library are now reachable without an operation. Artwork and Machine also open their real editors instead of the prior no-operation placeholder. Assignment-only actions require a selected operation; assignment inspection/application addresses that selected operation rather than indexing the first one. No machining defaults or operation are created by browsing resources.

Evidence:

- [Native shell at 1280×800](ui-redesign-evidence/v2/native-stock.png), [twelve duplicate-name operations](ui-redesign-evidence/v2/native-twelve.png), and [640×400 logical points at 2×](ui-redesign-evidence/v2/native-compact.png).
- [Browser last operation and anchored tools](ui-redesign-evidence/v2/browser-twelve.png), [empty operation list](ui-redesign-evidence/v2/browser-empty.png), and [current prefix result](ui-redesign-evidence/v2/browser-prefix.png).
- Native PNG/metadata and generated review jobs: `flat-v-carve/artifacts/gui/visual-v2/`.
- Browser shell acceptance: `flat-v-carve/artifacts/gui/browser-smoke/2026-09-18T07-58-07.085Z/`. Browser size/DPI and editing review: `flat-v-carve/artifacts/gui/browser-smoke/2026-09-18T07-58-10.213Z/`. Both use build `8d097a052c5c`, passed and reported no console errors.

Validation: 171/171 library tests; clippy with warnings denied; native/WASM builds; fmt and diff checks; all 111 numeric IDs remain mapped. Focused tests cover long header names, separate hit regions for twelve rows, anchored resources, 640×400 through 1920×1080 layouts, footer bounds, same-operation ramp-draft/scroll preservation and recovery compatibility. The browser shell scenario exercises rename/move/delete/Undo, independent enable/eye/lock controls, collapsed-panel reopening, empty-job resource access and creation, and prefix generation followed by checked preparation without scope widening. The visual scenario repeats baseline sizes/scales plus partial fields/navigation/Undo, datum choices, simulation and library import. Native and browser renders were compared with revision 2; remaining inspector/card density and the overflowing view toolbar belong to V4/V5, so this milestone does not claim completion of those panels or the complete narrow-width application.

Reproduction: build as in V0, then run `node crates/cam-gui/web/smoke.mjs --shell` and `node crates/cam-gui/web/smoke.mjs --visual-review`. The smoke driver now opens actual row/scope/tool/setup menus and scrolls by the distance needed to reveal a control; it never treats an offscreen row as a fixed header control.

## V3 — resource workspaces

Library, Machines and Job tools now use a single explicit resource route and occupy the center+right work area while retaining the navigator. Entering a resource page pauses playback; leaving retains the exact playhead without resuming. The selected operation, artwork, canvas mode, operation scroll and partial job/library/copied-tool drafts remain separate from the visible route. Back to job, the inspector toggle, Setup and operation navigation return to the authoring workspace. Worker polling, retained results and checked output continue through the existing lifecycle.

The Library follows revision 2's composition: searchable compact cutter rows, one geometry/capability section with a parameter-driven endmill/V-bit/knife schematic, and a profile list beside the selected cutting fields. A fixed footer names the target operation/stage and separates global Save from application to the job. At 1280×800 the endmill's geometry and all selected profile values are visible together. Compact widths use item/profile choosers with independently scrolling details and an anchored footer. Optional milling material/machine metadata remains editable through Profile actions → Material & machine context. Existing tool/profile/machine create, duplicate, capture, import/export, revision comparison and save/reload commands remain reachable. The asset manifest describes the diagram geometry and its limits; the concept's decorative flutes and undeclared holder bodies are deliberately not inferred.

Job tools presents physical geometry, usage and actual T/H mappings, followed by the selected cutter's assignment table with operation ordinal/stage, profile, feed, spindle and status. Mapping links open Machine. Selecting an assignment shows its copied cutting values and Open operation / Reapply reviewed profile actions. Geometry edits list every affected assignment and preserve declared shaft/stickout on Apply. Unused and never-configured cutters remain explicit. Global catalog edits do not silently update job copies.

The contextual modal names its operation and stage, filters compatible cutters, shows geometry/profile summaries and distinguishes tool-only from tool+profile application. Job-tool choices are staged until Apply; Cancel leaves the assignment intact, and the unchanged current tool cannot accidentally clear its own cutting values. Milling and knife presets use the saved catalog revision, with an explicit notice when unsaved library edits are excluded. Clear, baseline reset, reviewed reapply and capture remain explicit commands. Resource role/capture resolution now addresses the selected operation, including mixed Face/Profile/knife jobs.

Reviewed evidence:

- [Native Library, 1280×800](ui-redesign-evidence/v3/native-library-1280.png), [V-bit and profile, 1440×900](ui-redesign-evidence/v3/native-vbit-1440.png), [Machines](ui-redesign-evidence/v3/native-machines-1440.png), [Job tools at 1920×1080](ui-redesign-evidence/v3/native-job-tools-1920.png), [640×400 logical points at 2×](ui-redesign-evidence/v3/native-library-640.png), and [contextual picker](ui-redesign-evidence/v3/native-picker-1280.png).
- [Browser shared job tools](ui-redesign-evidence/v3/resources-job-tools.png), [knife geometry/profile](ui-redesign-evidence/v3/resources-knife-library.png), [staged assignment](ui-redesign-evidence/v3/resources-picker.png), and [library retained after failed import](ui-redesign-evidence/v3/resources-library-1280.png).
- Full native PNG/metadata: `flat-v-carve/artifacts/gui/visual-v3/`, including the 150% DPI Library, 1920×1080 Job tools and native knife diagram. Compared with revision-2 concepts 02/03; normal and compact pages were visually inspected.
- Final `--resources-review`: `flat-v-carve/artifacts/gui/browser-smoke/2026-09-18T11-45-06.372Z/`; seven acceptance records. Final `--shell`: `2026-09-18T11-45-20.774Z/`; four records. Both use build `2c51fac8b794` and report no console errors.
- Comprehensive `--gui5`: `flat-v-carve/artifacts/gui/browser-smoke/2026-09-18T11-41-00.582Z/`, build `cbe662410aef`; six records, no console errors. It covers create/duplicate/save, shared copies, global revision conflicts, compare/overwrite, machine settings, portable offline reset, simulation and exact downloaded output. The only later application change disables applying an unchanged job-tool choice; final V3 acceptance explicitly exercises that safeguard.

Validation: 174/174 library tests, 10/10 resource integration tests, native/WASM builds, clippy with warnings denied, fmt/diff checks and all 111 numeric field IDs still mapped. After the final unchanged-tool guard, the three picker tests and V3/browser shell checks were rerun. Focused tests cover draft/route preservation, Library footer bounds from 640×400 through 1440×900, and selected-operation capture in a mixed job. Browser V3 also checks partial library text through page navigation, failed-import retention, shared geometry and assembly edits, unchanged/cancelled/explicit job-tool choices, sibling assignment isolation, exact playback pause/return, high DPI and knife profile application. The initial short-window footer overflow and clipped final profile row were corrected, not waived.

Reproduction: build as in V0, then run `node crates/cam-gui/web/smoke.mjs --resources-review`, `--gui5`, and `--shell` (different `--port` values for concurrent isolated browsers). Native review adds `machines`, `library-vbit`, `library-knife` and `picker` pages; `library-knife` takes a knife job. UI probes continue to describe actual widgets and read-only state.

## Control migration checklist

The [numeric field inventory](ui-redesign-field-inventory.md) maps all 111 stable field IDs. Three legacy/non-rendered slots are explicitly identified instead of being reintroduced as new features. All rendered raw IDs, source/operation/contour qualification, help entries and diagnostic destinations must survive the migration.

Nonnumeric controls and their target ownership:

| Existing behavior | Destination / invariant |
|---|---|
| New/open/save, Undo/Redo, fixture | Header File/document controls; shortcuts remain |
| Prepare/Simulate, Generate/all/prefix, cancel, export | Header and visible retained scope |
| Operation add/enable/select/rename/move/delete | One navigator row per operation plus actions menu |
| Source add/replace/duplicate/delete/reorder | Artwork rows and Source actions |
| Source hide/lock; select/move/rotate/scale | Eye/lock and viewport modes; display semantics unchanged |
| Filled component/layer bulk selection and unresolved repair | Operation Geometry section |
| Closed profile contours/retained side/traversal/order | Profile Geometry tab |
| Knife open/closed chains and copied outlines | Knife Geometry tab |
| Stock page/bounds fit, anchor, unset XY | Stock Position section |
| Work-zero XY and Z datum, start defaults/unset | Stock Work zero / Clearance & start |
| Carving mode, clearing strategy, entry strategy | Flat V-carve tabs; geometry still defines Endmill-only target |
| Top/bottom/face-result references | Each operation's Heights group |
| Face area, pass direction, entry mode | Face Area / Tool & passes |
| Tabs automatic/manual, per-contour anchors and viewport drag | Profile Tabs & entry |
| Profile finish enable, seam, plunge/ramp, line/arc leads | Profile Cutting / Tabs & entry |
| Knife alignment/start, suggestions | Knife Corners & start; explicit fill-blanks action |
| Cutter/profile selection, clear/reset/reapply/capture | Assignment picker and explicit scoped actions |
| Plunge/ramp capability and spindle direction | Cutter geometry / assignment, same data owners |
| Catalog create/duplicate/import/export/save/reload/conflict | Full-width resource workspaces |
| Shared job geometry edit, use cutter, assignments/provenance | Job tools page |
| Applied/reusable machine, T/H, work offset, compensation, M6/return/start, coolant, path control, holder | Machine job inspector / reusable Machine page |
| Top/isometric/front/back/left/right, orbit, pan, zoom, fit | View toolbar and overflow; gestures unchanged |
| Stock coloring/walls/opacity, artwork/cut/travel/edge visibility | View display/layers controls |
| Playback speeds/Fit, time/motion seeking, stage jumps/path filter, quality | Compact simulation transport and details |
| Warning seek, point/section axes, pin/compare | Result inspector and timeline |
| Renderer diagnostics/failure/rebuild | Developer diagnostics expansion |
| Export preparation/cancel/validation/details/save/retry | Export modal, exact prepared bytes and scope |
| Recovery restore/keep/reload/clear, failed file retry | Exceptional status surfaces |

Tests for routed controls must follow the new visible menus/tabs without directly changing application state. Keep observation probes tied to real visible widgets; do not publish imaginary rectangles to make old scenarios pass.

## V4 — authoring panels

All four operation editors now share a sticky ordinal/name/type/state header, compact tabs, flat collapsible sections, bounded numeric rows and the existing generation footer. Face uses Area & heights / Tool & passes; Profile uses Geometry / Cutting / Tabs & entry; Knife uses Geometry / Cutting / Corners & start. Flat V-carve retains Shape & depth / Endmill / V-bit, with carving mode in Shape & depth so it scrolls on short windows. Search reaches numeric fields across tabs; diagnostics select the corresponding tab. Collapsed section IDs include the operation identity. Tab and scroll state follow stable operation IDs through selection, reordering, document adoption and Undo, with backward-compatible recovery defaults.

Artwork now groups Source/actions, Placement and Usage, including disabled operations and references awaiting repair. Knife uses the shared source-management surface. Job settings stays a short scoped tolerance form. Corrected numeric unit labels include Face angle, Profile entry/lead feeds and angles, tab count and anchor fractions; long units no longer wrap. Profile's duplicate tool stepdown limit control was removed from Feeds & speed; its single editor remains with pass depth. The procedural Face coverage/travel/entry and Knife heading diagrams are documented in the asset manifest.

Acceptance exposed a pre-existing Knife Shift-click defect: it extended the retained scene's old selection. It now extends the live qualified artwork selection, like Profile. A real-widget regression test keeps the old scene while adding/removing a chain; browser coverage also verifies Shift selection and Undo.

Machine now has applied identity/provenance, compact controller and output selectors, a bounded job-wide T/H table, and expandable startup/M6/rapid-rate/holder assumptions. Custom holder segment dimensions remain read-only. T/H raw text follows stable job-tool IDs, so shared cutters have one mapping independent of operation selection, including unused cutters and zero-operation jobs. Older operation-scoped drafts remain recoverable. Explicit profile application replaces mapping drafts; Undo restores them. Matching T to H affects assigned tools only, retaining unrelated unused-tool input. Changing the work offset or coolant preserves partial blend values. Applying a machine preserves work zero.

Profile manual anchors use bounded shared field rows. Browser acceptance covers numeric manual anchors, independent start anchors, tab navigation, partial input, restart/recovery and recovered Undo. Existing source replacement, unresolved-reference repair, ordering, hide/lock, duplication and portable save/reopen remain reachable through the reorganized Artwork controls.

Final validation: `cargo test -p cam-gui --locked --quiet` passed 251 tests (180 library plus 71 integration/binary tests), with one ignored test; `cargo clippy -p cam-gui --features ui-review --locked -- -D warnings`, formatting and diff checks passed. Native and WASM builds passed; all 111 numeric field IDs have explicit destinations. Tests cover stable operation view state, diagnostic routing, bounded forms, live Knife selection and job-wide mapping/recovery semantics.

Browser runs passed with no console errors; paths below are relative to `flat-v-carve/artifacts/gui/browser-smoke/`:

| Workflow | Run | Build | Checks |
|---|---|---|---|
| Artwork / Flat V-carve source repair, combined generation, exact checked bytes, portable save and Undo | `2026-09-19T10-06-41.211Z` | `b9919d1ce183` | 7 |
| Source-free Face, ordered Face → carve/knife, playback, export and height-dependency repair | `2026-09-19T10-13-09.796Z` | `1ff0534ca2fe` | 6 |
| Profile contours, tabs/finish/ramp, library apply/modify/reset, export, manual-anchor recovery | `2026-09-19T10-12-01.142Z` | `1ff0534ca2fe` | 18 |
| Knife viewport Shift selection, blank-only suggestions, partial Undo, copied outlines, export/reopen | `2026-09-19T10-15-01.773Z` | `1ff0534ca2fe` | 3 |
| Shared/unused T/H mappings, partial blend preservation, restart/Undo, machine application | `2026-09-19T10-17-21.667Z` | `1ff0534ca2fe` | 3 |

The Artwork run predates only the final compact anchor row and unrelated Machine selector draft fix; the affected Profile and Machine workflows use the final build. A duplicate JavaScript declaration in the older GUI4 scenario was corrected before its passing run. Earlier development captures remain local and are not the final evidence.

Representative reviewed captures: [Artwork](ui-redesign-evidence/v4/native-artwork.png), [Machine](ui-redesign-evidence/v4/native-machine.png), [Flat V-carve](ui-redesign-evidence/v4/native-carve-shape.png), [Face area](ui-redesign-evidence/v4/native-face-area.png), [Face cutting](ui-redesign-evidence/v4/native-face-cutting.png), [Profile cutting](ui-redesign-evidence/v4/native-profile-cutting.png), [recovered manual anchor](ui-redesign-evidence/v4/browser-profile-manual-anchor.png), [Knife](ui-redesign-evidence/v4/native-knife-corners.png), [Job settings](ui-redesign-evidence/v4/native-settings.png), and [640×400 logical at 2×](ui-redesign-evidence/v4/native-carve-compact.png). Refreshed native PNG/JSON pairs for all panels are in `flat-v-carve/artifacts/gui/visual-v4/`.

The authoring footer and scroll body fit the compact window. The viewport toolbar still clips at reduced effective width; that is the V5 toolbar-overflow task. V5 inspection and V6 export/exception/full-matrix audit remain pending. This milestone does not claim those panels or the whole redesign are finished.

The [expanded Machine summary](ui-redesign-evidence/v4/browser-machine-advanced.png) shows the retained startup, M6, rapid-rate and holder assumptions. Its capture scrolls the outer inspector after opening the real advanced section; the bounded mapping list keeps its own position.

## V5 — viewport and inspection

The viewport toolbar is one bounded row. Top/Isometric are quick choices where space permits; all six camera presets, tilt, zoom and gesture help remain in View. Artwork's Select/Move/Rotate/Scale menu moves into View at compact widths. Display owns stock color, walls and opacity; Layers owns artwork/cutting/travel/edge visibility and the renderer-backed View key. At the narrowest width, Layers joins Display. Menus stay open while adjusting controls or paging; explicit camera/mode/stage choices close them. Camera gestures and renderer geometry are unchanged. The motion readout has a light background for contrast over stock and paths.

The collapsed transport shows Play/Pause, Start, all speed rates plus Fit, elapsed/total modeled time, a single time-scaled stage track, stage jumps and options. Precise motion-index seeking and transfer/checkpoint diagnostics are in Motion & diagnostics. Display resolution, warning count and the rapid-rate assumption remain visible. Very short stages retain exact proportions and are reachable in the bounded eight-row Stages menu. Pointer clicks/drags and keyboard arrows/Home/End seek through the existing clock. Warning markers group nearby hit targets with a tooltip; the Warnings tab retains an individual Show action for every problem. Untimed scenes retain a motion-index track.

Result inspection has Summary / Section / Warnings tabs. Summary reads resolved heights, face results, generated tab counts and times from retained inspection data. Section keeps X/Y selection, raster resolution and pin/compare controls; its plot now shares the application's light palette. Warnings state the separate check raster, job-tool identity, first occurrence and worst depth, and explicitly distinguish display estimates from output validation. The selected inspection tab and section axis survive recovery with backward-compatible defaults. Renderer diagnostics and the failure/rebuild drill remain collapsed in Summary.

Acceptance found a pre-existing fractional-seek handoff bug: the application cleared the requested fraction before adopting restored stock. It now adopts first and completes the request afterward. A submitted seek keeps its own fraction while newer pointer requests coalesce to the latest target; a local forward target also replaces an unsubmitted backward seek. Real-widget and application-reply tests cover these paths. Dense-fixture acceptance additionally found that numeric field search could trap the scroll wheel inside the component list; Flat V-carve field search now hides that unrelated list.

Validation: the full GUI suite passed 256 tests (185 library plus 71 integration/binary tests), with one ignored test. The 185 library tests passed after the persistent-menu change; all 17 viewport tests plus the application cancellation regression passed after the final seek/menu fixes. Clippy with warnings denied, formatting, diff checks and native/WASM builds passed. Browser inspection acceptance on build `1db18ed22d8c`, run `2026-09-19T11-29-36.725Z`, asserts the chosen display styles and unchanged inspection point, and covers display-only invariants, quality selection, pinned stale/current comparison, 1280×800, 1440×900, 1920×1080, 960×600 and 640×400 logical layouts, visible speed options, twelve-stage paging and keyboard Home/End. Native captures cover 150% and 200% DPI. Changing Chromium's emulated DPI repeatedly during one session produced inconsistent intermediate canvas sizing, so those exploratory captures are not used as evidence; browser logical-size review and native DPI review are recorded separately.

Browser GUI10 (`2026-09-19T11-06-10.732Z`, build `0741202121e7`) passed 12 checks for continuous 1× motion, cutter/removal agreement, fractional forward/backward scrubbing, Fit, stated/unstated assembly behavior, warning lists and the timeline warning marker. It precedes only popup-sizing/quality-group and cancellation cleanup changes; the final inspection run covers the affected menu paths. Browser GUI9 (`2026-09-19T11-20-51.652Z`, loaded build `2dc9fd79adf1`) passed all 12 dense-fixture checks: 137,717 motions, edit during generation, eight-row stage paging, Fine display rebuild, both section axes, renderer failure/rebuild, twenty scrub bursts, thirty-second idle, cancellation and regeneration of the unchanged 22,883-motion reference. Camera p95 was 14.5 ms with no scene uploads; scrub-burst p95 was 102.5 ms. Declared display memory was 138,741,224 bytes against a 268,435,456-byte budget; WASM linear memory and total GPU memory remain unmeasured. All three runs reported no console errors. The final build differs from the dense run only in the stage menu's minimum width. The harness now records build identity at startup; the dense evidence explicitly notes correction of its old completion-time build reading.

Representative reviewed captures: [Summary](ui-redesign-evidence/v5/native-summary.png), [Section](ui-redesign-evidence/v5/native-section.png), [Warnings](ui-redesign-evidence/v5/native-warnings.png), [150% DPI](ui-redesign-evidence/v5/native-section-150.png), [200% DPI / compact](ui-redesign-evidence/v5/native-compact.png), [view key](ui-redesign-evidence/v5/browser-view-key.png), [stage paging](ui-redesign-evidence/v5/browser-stages.png), [Y section](ui-redesign-evidence/v5/browser-section-y.png), [warning marker](ui-redesign-evidence/v5/browser-warning-marker.png) and [pinned comparison](ui-redesign-evidence/v5/browser-comparison.png). Native PNG/JSON pairs are retained in `flat-v-carve/artifacts/gui/visual-v5/`.

The seek cleanup also clears submitted and queued targets when their worker execution is cancelled; absent retained executions cannot leave a permanent pending seek. Expanded view/transport menus have bounded scrolling, and display quality/path filters use inline expandable groups to avoid nested-popup dismissal. The final inspection rerun (`2026-09-19T11-29-36.725Z`, same build) also verifies the [section plot after scrolling at 640×400](ui-redesign-evidence/v5/browser-section-compact-scrolled.png). V6 export/exception-state polish and the full completion audit remain separate work.

## V6 — export, exceptional states and completion audit

Export now reviews the prepared manifest's exact endpoint and ordered stages, with current operation/cutter names, applied machine/work offset, compensation mode and read-only T/H. Inspection data is used only when its execution fingerprint matches the prepared manifest; unavailable enrichment falls back to stage IDs. The summary never substitutes all enabled operations or a file's first tool number for the actual prepared scope. Validation and Save remain prominent, the body scrolls at compact sizes, and technical report/hash/histograms remain expandable. Existing prepare/cancel/stale/save/retry authority is unchanged.

Issues now show a count, owning operation and Go to field, with code/path on hover. Opening an issue selects its owner before navigating, clears the filter, and uses the existing tab/group/focus mechanism. Qualified paths, whole-operation paths and separately supplied owners are covered, including different operation types. The diagnostic model has no severity field, so the UI does not invent one. Recovery and retained-save failures have a separate warning row. The startup screen offers Start blank job and Open saved job and explains source-free Face work. The injected-renderer-failure surface has a visible Restore viewport action backed by the existing resource-rebuild command.

Validation: the full GUI suite passed 257 tests (186 library and 71 integration/binary tests), with one ignored test. The 186 library tests passed after startup/recovery changes; all five diagnostic tests passed after the final owner-path refinement. Clippy with warnings denied, native/WASM builds, formatting/diff checks and all 111 numeric-field destinations passed. No planner, output-byte writer or timing model was changed.

Final browser completion acceptance: `2026-09-19T11-53-43.565Z`, build `79a6b706cb15`, 11 checks and no console errors. It covers blank-job access; prefix generation with a later enabled operation excluded from the export summary; modal bounds at 1280×800, 1440×900, 1920×1080 and 640×400; failed destination and SHA256-identical download retry; automatic dialog close; visible renderer restoration; partial-draft recovery; and a diagnostic opening the second operation's field. Fresh 150% and 200% browser sessions (`2026-09-19T11-48-49.941Z` and `2026-09-19T11-48-51.075Z`, build `24de733630ca`) each passed four export/scope/retry checks without console errors. The only later UI change is the diagnostic owner-path/label refinement. Those sessions record actual canvas/DPI dimensions and reduced egui effective widths; they do not reuse the earlier unstable mid-session DPI experiments.

Reviewed captures: [prepared prefix](ui-redesign-evidence/v6/browser-export-prefix.png), [compact export](ui-redesign-evidence/v6/browser-export-compact.png), [native export](ui-redesign-evidence/v6/native-export.png), [native validation failure](ui-redesign-evidence/v6/native-export-error.png), [native 150%](ui-redesign-evidence/v6/native-export-150.png), [native 200% compact](ui-redesign-evidence/v6/native-export-compact.png), [browser 150%](ui-redesign-evidence/v6/browser-export-150.png), [browser 200%](ui-redesign-evidence/v6/browser-export-200.png), [save failure](ui-redesign-evidence/v6/browser-save-failure.png), [recovery](ui-redesign-evidence/v6/browser-recovery.png), [issue destination](ui-redesign-evidence/v6/browser-issue-destination.png), [renderer restore](ui-redesign-evidence/v6/browser-renderer-failure.png), [startup](ui-redesign-evidence/v6/browser-start.png) and [empty job](ui-redesign-evidence/v6/browser-empty.png). Native PNG/metadata are in `flat-v-carve/artifacts/gui/visual-v6/`.

The [19-area completion audit](ui-redesign-completion-audit.md) maps every panel to normal and relevant exceptional-state evidence and records deliberate differences from the concepts. Artwork, Library and Job tools were compared again against all three revision-2 images. Original vectors and measured diagrams suffice; photographic/tool texture assets and unsupported concept-only commands remain optional backlog. All V0–V6 milestones are complete within the documented native/browser envelope.

Supplemental browser exception review (`2026-09-19T11-57-59.988Z`, same final build) passed four checks without console errors: [empty library](ui-redesign-evidence/v6/browser-empty-library.png), [no matching items](ui-redesign-evidence/v6/browser-no-match.png), [failed import retains the catalog](ui-redesign-evidence/v6/browser-library-error.png), and [absent machine blocks export](ui-redesign-evidence/v6/browser-export-error.png). Import review saves the private catalog first, honoring the existing guard against replacing unsaved edits. Earlier development attempts are not acceptance evidence.
