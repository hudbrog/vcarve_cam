# UI redesign implementation

The scope and acceptance criteria are in the [visual redesign plan](ui-visual-redesign-plan.md). Each V milestone is committed independently. Optional conveniences listed in the plan remain separate from this work.

| Milestone | State | Evidence |
|---|---|---|
| V0 baseline | Complete | Fresh native/browser captures; 111-field inventory; baseline tests |
| V1 visual foundation | Next | Shared tokens, icons and bounded fields; Stock proving panel |
| V2 workspace shell | Pending | Header, navigator, status, execution scope |
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
