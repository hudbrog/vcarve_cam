# UI redesign completion audit

This closes the 19-area review in the [visual redesign plan](ui-visual-redesign-plan.md). The [milestone log](ui-redesign-progress.md) records builds, browser runs, tests and qualifications. Evidence is cumulative across V0–V6; unchanged panels were not all rerun for the last export-only build.

## Panel coverage

| Area | Delivered presentation | Normal and exceptional-state evidence |
|---|---|---|
| 1. Header and document commands | Graphite header, document identity/state, Prepare/Simulate, generation scope, export; compact File commands | V2 shell run: long filename, collapsed panels, prefix generation; V6 checked prefix and blocked export |
| 2. Navigator and operations | Stable ordered operation rows, separate enable/actions controls, anchored job tools | V2 twelve duplicate-name operations, move/rename/delete/Undo, zero operations; V6 issue owner selection |
| 3. Artwork | Source/actions, placement, usage; independent eye/lock; selection tools in viewport menu | [Artwork](ui-redesign-evidence/v6/native-artwork.png); V4 GUI4 source replacement, unresolved references, cross-source selection and portable reopen |
| 4. Stock and work zero | Bounded units/fields, dimensions/position/datum/clearance groups, live schematic | V1 normal/bottom datum and partial text/Undo; V4 compact form audit |
| 5. Machine | Applied identity, controller/output groups, job-wide T/H, expandable assumptions; reusable machine workspace | [Machine](ui-redesign-evidence/v4/native-machine.png); V4 shared/unused mappings, partial blend, restart/Undo; V6 absent-machine export failure |
| 6. Job settings | Short scoped tolerance form | [Settings](ui-redesign-evidence/v4/native-settings.png); typed missing-field routes and 111-field inventory |
| 7. Shared operation inspector | Sticky identity/type/state, compact tabs, scroll body, persistent footer | V4 stable ID/tab/scroll tests and browser authoring; V6 issue-to-owner/tab/group/field |
| 8. Flat V-carve | Shape & depth / Endmill / V-bit, shared assignments, advanced limits | [Carve](ui-redesign-evidence/v4/native-carve-shape.png); V4 combined/endmill-only and source repair; V5 dense component search and generation cancellation |
| 9. Face | Area & heights / Tool & passes; measured area/entry diagrams | [Face](ui-redesign-evidence/v4/native-face-area.png); V4 source-free authoring, Face→carve/knife, height-dependency repair |
| 10. Profile | Geometry / Cutting / Tabs & entry, bounded manual anchors | [Profile](ui-redesign-evidence/v4/native-profile-cutting.png); V4 tabs/finish/ramp, library modified/reset, recovered manual anchors and Undo |
| 11. Drag knife | Geometry / Cutting / Corners & start; heading diagram and explicit blank-only suggestions | [Knife](ui-redesign-evidence/v4/native-knife-corners.png); V4 open/closed chains, Shift selection, partial Undo, derived outlines/export/reopen |
| 12. Assignment picker | Focused staged selection with explicit scope, geometry/profile distinction | [Picker](ui-redesign-evidence/v3/resources-picker.png); V3 cancel/draft preservation, apply/reapply/reset; V4 assignment authoring |
| 13. Global library | Center/right workspace, item list, measured cutter schematic, geometry/profile sections, persistent footer | [Library](ui-redesign-evidence/v6/native-library.png); V3 invalid/import/conflict/draft retention and compact layouts; V6 empty/no-match and failed-import captures |
| 14. Job tools | Tool/mapping table, assignment table/detail, explicit shared geometry scope | [Job tools](ui-redesign-evidence/v6/native-tools.png); V3 shared/unused tools and assignment isolation; V4 job-wide T/H with no operations |
| 15. View toolbar and canvas | One bounded row; View/Display/Layers menus, six views, stock/layer legend | V5 display-only invariants, inspection point preservation, menu bounds; native/browser compact and DPI captures |
| 16. Transport and timeline | Time-scaled stages, keyboard/pointer seeking, warning markers, bounded stage list, optional details | V5 GUI10 fractional seek/time/cutter/removal; GUI9 dense paging/quality/scrub/cancel; tiny stages remain reachable |
| 17. Inspection | Summary / Section / Warnings; light section plot, explicit raster assumptions, retained pin/compare | V5 X/Y, stale/current comparison, individual warnings/markers, tight-holder fixture, compact scrolled plot |
| 18. Export | Modal with exact prepared scope/stages/tools, read-only machine/T/H, validation, persistent Save, expandable report | V6 prefix with a later enabled operation, failed save/exact-byte retry/success; native absent-machine failure; existing stale/cancel/late-reply tests |
| 19. Status, issues, recovery, empty states | Quiet normal status; separate recovery/save warning row; counted owner-aware issues; startup actions; visible renderer restore | V6 blank job, missing setting and focused repair, partial/recovered draft, failed save, injected renderer failure/restoration; V3 resource conflict/empty states |

## Visual comparison and deliberate differences

The final Artwork, Library and Job tools captures were reviewed against revision-2 concepts 01–03. The shared graphite header, pale workspace, teal selection, warm mode control, ordered navigator, flat grouped inspector and full-width resource workspaces now follow the concepts. The implementation uses denser text and smaller diagrams to keep the existing fields and actions usable at 1280×800 and reduced effective widths.

Small icons are local vector/painter assets. Cutter, stock, Face and Knife diagrams derive from actual values. Generated photographic cutters, wood textures, extra fonts and new overall mockups were unnecessary for this pass. The [asset manifest](ui-asset-manifest.md) records the assets and rules. Concept-only commands and unsupported fields (for example flutes/material metadata, Center on stock and new tool lifecycle commands) remain the plan's optional backlog. No implied machining defaults were added.

The export modal is intentionally retained. Its rows come from the prepared manifest's stage IDs, in manifest order; human names and T/H are read only from the matching current revision. Retained inspection enriches rows only when its execution fingerprint matches. Missing enrichment falls back to stable stage IDs. Selecting another operation cannot widen export. H is labelled as an applied mapping and is used only for tool-table compensation.

The issue model has no severity field. The UI therefore reports settings needing attention, operation ownership, reason and Go to field, with diagnostic code/path on hover; it does not invent warning/error classifications. Both qualified field paths and separately supplied operation owners are supported.

## Validation envelope

Native and browser evidence covers 1280×800, 1440×900 and 1920×1080; normal and compact 640×400 layouts; native 150%/200% scale; and fresh browser sessions at 150%/200% device scale. The browser review records CSS viewport, device ratio and canvas backing dimensions alongside actual egui control bounds. It verifies the reduced effective widths (approximately 853×533 and 640×400) instead of assuming the browser's CSS dimensions are egui points. Earlier experiments that changed DPI repeatedly within a running session are excluded.

V6's full GUI suite passed 257 tests (186 library plus 71 other tests), with one ignored test. The library suite was repeated after startup/recovery presentation changes; focused diagnostic tests cover the final ownership refinement. Clippy with warnings denied, formatting, native/WASM builds and the complete numeric-field inventory passed. Browser acceptance verifies rendered controls and checked file hashes; it does not mutate application state to bypass interaction. File-picker failure is injected through the platform boundary in a temporary browser profile.

V5's dense-performance acceptance remains applicable: no renderer/planner payload or timing-model change was introduced in V6. The recorded camera p95 is 14.5 ms without scene uploads; scrub-burst p95 is 102.5 ms. Declared memory remains below its configured budget. Total GPU and WASM linear memory were not measured, and no continuous-60-fps scrub claim is made.

Known platform limits remain explicit: browser download fallback confirms requested bytes, not a disk write; the renderer-loss drill tests retained-resource restoration, not every possible physical device failure. Short windows use scrolling and one open side panel. Technical reports, advanced settings and long sections are intentionally scrollable. These qualifications do not leave a required redesign milestone open.
