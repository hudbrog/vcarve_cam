# UI redesign implementation

The scope and acceptance criteria are in the [visual redesign plan](ui-visual-redesign-plan.md). Each V milestone is committed independently. Optional conveniences listed in the plan remain separate from this work.

| Milestone | State | Evidence |
|---|---|---|
| V0 baseline | Complete | Fresh native/browser captures; 111-field inventory; baseline tests |
| V1 visual foundation | Complete | Shared tokens/icons, bounded forms, Stock sections and live datum diagram; 167 tests |
| V2 workspace shell | Complete | Compact header, one ordered row per operation, anchored job tools, scope/status, adaptive panels; 171 tests |
| V3 resources | Pending | Library/machines and job tools pages |
| V4 authoring | Pending | All four operations, artwork, machine, diagrams |
| V5 inspection | Pending | View controls, transport, timeline, result inspection |
| V6 completion | Pending | Export, recovery, exceptional states, full visual/behavior audit |

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
