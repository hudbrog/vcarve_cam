# UI visual redesign — revision 2 brought forward

Date: 2026-09-18. Status: design and implementation plan; no application changes in this pass.

## Recommendation

Keep the egui/wgpu application and its existing workflow. Bring it close to revision 2 through a stronger shell, consistent compact controls, an actual ordered navigator, and full-width resource workspaces. Preserve the newer operation editing, simulation and export behavior. The largest gains come from layout and information hierarchy; graphics support those changes.

Revision 2 is the visual reference, not a pixel-perfect functional specification. Follow its graphite header, pale panels, teal selection, orange mode selection, line icons, aligned forms and restrained separators. Do not reproduce generated lettering, ornamental gradients, imagined fields, or unsupported actions.

## Evidence and current state

Reviewed source at `8236ae4`, the three [revision-2 concepts](ui-concepts/2026-09-09/README.md), their [prompts](ui-concepts/2026-09-09/revision-2/prompts.md), and the running native application with the user's flower job. Inspected the operation/simulation workspace, Stock & work zero and Library visually; other panels were reviewed from their rendering code and existing milestone documentation. The inspected executable was last written 2026-09-17; its exact source revision was not independently established. Rebuild and capture both targets before implementation baselines are accepted.

The live session was used for navigation only: no machining values, library content or job files were changed. Existing untracked real-data files are outside this work.

| Area | Current implementation / observation | Gap from revision 2 |
|---|---|---|
| Shell | Graphite title row plus a separate file/action row; same general palette already exists | Modes/actions cluster right; document actions consume another full strip; weak visual rhythm |
| Navigator | Fixed 205-point width; 34-point navigation rows; one encompassing scroll region | Selected operation appears both as a navigation entry and in the ordered list; commands dominate; tools fall below the fold |
| Artwork | Independent sources, replacement, duplication, placement, workspace hide/lock | Plain source rows; eye/lock live in inspector; long explanatory copy competes with fields |
| Inspector | 300–480-point resizable panel; mixed row-based and stacked fields, 42-point operation tabs | Uneven density and grouping. Live Setup capture showed unusually large vertical gaps; investigate layout allocation before attributing this to style alone |
| Stock | Stock bounds/thickness, resize anchors, work zero, start XY and clearance | Z datum is edited twice in `setup_panel`; explanations and actions need a clearer hierarchy |
| Library | Improved searchable tool/machine window; 60-point list entries; scrolling forms and bottom apply actions | Still overlays the work area; geometry and profiles cannot be compared as easily as in concept 02 |
| Job tools | Floating management window with tool provenance, usage and geometry editing | Text lists rather than the concept's tool/assignment tables; limited global overview |
| Operation editors | Flat V-carve, Face, Profile and Drag knife are implemented | Different grouping conventions; some editors are long forms; concept gallery does not cover all current settings |
| Viewport | Camera presets/orbit, grid, stock styles/transparency, artwork/path toggles, overlays | Two busy toolbar rows; central stock can feel small; labels and overlays need consistent styling |
| Simulation | Timed continuous playback, stage jumps, motion/time seeking, stock quality, assemblies, warnings, sections | Speed choices, two sliders and technical text compete for height; concept 03 predates these capabilities |
| Export | Validation/preparation and Save as modal, error/retry states, technical details | Already closer to a focused task than the old export concept; retain it and improve scope/readout styling |

The GUI README is partly historical: it says profiles/tabs are later work, while `profile_ui.rs` implements them. Rendering code and current tests govern this plan. Do not restart framework evaluation or mistake old milestone omissions for today's scope.

## Shared visual and interaction rules

### Layout and density

All dimensions below are initial **logical egui points**, to be validated at real DPI settings, not measurements taken from generated images.

| Token | Proposed default |
|---|---|
| Header | 52 high; 16 horizontal inset |
| Navigator | 248 wide, resizable 224–300 |
| Inspector | 344 wide, resizable 320–440 |
| View toolbar | 36 high; overflow rather than uncontrolled wrapping |
| Status | 24 high; exceptional messages may open a separate strip |
| Spacing scale | 4 / 8 / 12 / 16 / 24 |
| Standard row / input | 28 high; 32 for comfortable density |
| Form section | 12 inset, 16 between groups; one separator, no nested cards |
| Icon | 18 or 20; 1.5–1.75 stroke at nominal size; 28 minimum desktop hit box |
| Radius / border | 2–3 radius; 1-point neutral border |
| Simulation transport | Approximately 96 high collapsed; optional detail expansion |

At 1440 wide the proposed 248 + 344 side panels leave about 848 for the work area; at 1280 they leave about 688. At 1280×800, keep roughly 540+ points of viewport height in Prepare and 440+ in Simulate, excluding native window chrome. Validate these budgets with real content rather than reducing the font to make a screenshot fit.

At narrow effective widths, preserve readable forms: compact header/document actions, move secondary view commands to a menu, and allow explicit side-panel collapse with a visible reopen button. Full-width resource pages stack their detail regions when needed. Do not promise a mobile machining workspace. Test large DPI as an effective-width case.

The navigator's Setup and bottom Job tools summary stay anchored. Artwork and Operations share the remaining scrollable region; large tool collections get a capped short list plus Manage. No repeated command stack beneath every operation. Keep panel widths, selection, open sections and per-entity scroll positions across navigation.

### Color, type and controls

| Semantic token | Starting value | Use |
|---|---|---|
| Header | `#2C3741` | App/document/modes/actions; light text |
| Panel | `#EDF2F6` | Navigator and inspectors |
| Raised/field | `#F8FAFC` / `#FFFFFF` | Subtle toolbar surfaces / editable values |
| Divider | `#C5D0D9` | Panel boundaries and form separators |
| Main / secondary text | `#22313F` / `#556779` | Labels and less prominent metadata |
| Selection wash / edge | `#B7E6E9` / `#168E98` | Selected row, active field context |
| Primary action | `#29BCC3` with dark text | Generate and the relevant commit action |
| Active mode | `#FFA64C` with dark text | Prepare / Simulate only |
| Warning / error / success text | `#985710` / `#B02A23` / `#1B704D` | Semantic statuses, always with text or icon |

Check contrast on every actual background before freezing tokens. Selection, focus, hover, enabled state and validation state must be distinguishable. Orange mode fill is not an error indicator. Muted controls must still be readable.

Use one bundled proportional font, initially the existing egui font, with a real weight hierarchy: 14 body, 12 secondary, 14–15 semibold section titles, 18 panel title, 20 app title. Evaluate a font change only after layouts are stable. Tabular numbers where available; monospace for G-code, hashes and technical details. Never render critical labels as bitmaps or depend on platform emoji.

Create reusable field rows: label, editable value, fixed unit column, optional help. Use bounded row height (never consume remaining inspector height). Long labels wrap deliberately; long values can be selected/copied without pushing units offscreen. Standardize number alignment, unset hints, focus borders and inline validation. Retain the raw text editor: `-`, `1.`, unset and invalid states must survive navigation, recovery and Undo. Styling must not change parsing or commit semantics.

Use flat rows and separators for sections. Keep collapsible advanced groups with a concise summary. Essential context remains visible: selected tool, geometry count, height reference, unresolved state and execution scope. Move long explanations to help; never move the meaning of a potentially ambiguous field solely into a tooltip. A search opens matching groups and issue navigation opens/focuses the owning field even when filtered.

### Scope and status language

Keep these distinct throughout: **Global library**, **This job**, **Operation / stage**, and **Display only**. Job cutter identities must not silently become controller T numbers. Show the assigned cutter name; only display `T1 / H1` when actual mappings exist. Use `Not mapped` otherwise.

Keep three status axes separate:

| Axis | Example states | Owner |
|---|---|---|
| Document | Saved / Unsaved changes / Partial input | Header/document controls |
| Plan | Not generated / Generating / Current / Out of date / Failed; All enabled or Through operation N | Compact work-area strip and generation controls |
| Output | Not prepared / Checking / Ready to save / Save failed | Export dialog, with compact summary if useful |

A current plan, attractive stock rendering or empty display-warning list never implies checked output. A displayed stale result remains inspectable with an explicit label. Show prefix scope in generation, simulation and export; export must not widen it. Busy/cancel/recovery failures remain actionable.

## Panel-by-panel design

Each numbered area is an implementation/review unit, not a request to add a new application feature.

### 1. Header and document commands

Combine the title and document actions into a disciplined strip: app mark/title left; document name with saved/dirty state and File menu; Prepare/Simulate mode group; Generate and Export right. Keep Save and Undo/Redo available through compact labeled/tooltip controls and existing shortcuts. At small widths, elide the filename and group secondary file actions.

Use an attached menu on Generate for **All enabled operations** and **Through selected operation**. The primary label and scope readout must make the chosen action unambiguous. Avoid two bright Generate buttons in the same view; an operation footer can expose the secondary prefix action. Busy state uses progress text and Cancel in a reserved location.

Acceptance: no overlap at 1280×800 or large DPI; document, mode and execution scope distinguishable at a glance; keyboard commands still work.

### 2. Left navigator and operations list

Follow revision 2's four groups: Setup, Artwork, Operations, Job tools. Keep Job settings (an actual current feature) in Setup. Show machine applied-name summary and a Global badge on Tool library.

Render every ordered operation once: enable toggle, ordinal, type icon, name, compact tool summary, issue indicator and overflow. Row click edits that operation. Overflow contains Rename, Move earlier/later, Generate through here and Delete; preserve keyboard alternatives. Add operation opens a compact chooser for the four implemented types. Drag reordering is optional later work; do not make it a dependency or remove current move commands.

Move Inspect result into Simulate/inspection navigation rather than repeating it inside the operation list. Disabled operations stay visible; hide artwork and disable operation must look and behave differently. Job tools are job-wide; the selected operation's assignment is summarized in its inspector.

Acceptance: twelve operations are manageable through a bounded scroll; duplicate names remain distinguishable; selection and enable controls never trigger each other; zero-operation jobs still expose stock, artwork, machine and resources.

### 3. Artwork rows and artwork inspector

Use source icon, name, eye and lock per row. Keep toggle hit areas separate from selection. Inspector title is the actual filename with Artwork item scope. Sections: Source (SVG/import interpretation and Replace), Placement (X/Y/rotation/scale), Usage (operations referencing this item). Put duplicate/reorder/delete in a source actions menu.

Preserve current semantics: hide/lock are workspace display/picking settings; hidden assigned geometry still cuts; lock does not block numeric placement edits. State this compactly beside those controls/help. Do not silently reinterpret locking during a visual change.

Use the existing selection/bounds overlay, coordinate model and drag interactions. Add small source labels only on selection/hover if useful; avoid labeling every dense contour. Revision 2's Center on stock is a **new convenience command**, not assumed present: defer until a separate placement-command test covers rotated/scaled sources. Geometry assignment stays with each operation. Usage navigation can be added by querying existing references, without a new document schema.

Acceptance: replacing/reordering sources retains qualified identities and exposes unresolved references; viewport picking and Shift selection are unchanged.

### 4. Stock & work zero

Four groups: **Physical stock** (width/length/thickness), **Stock position** (minimum XY, resize anchor and fit-from-page/artwork), **Work zero** (XY choice/custom point and one Z datum control), **Clearance & start**. Fit commands name their source. Collapse rarely used unset/default commands into each group's actions.

Remove the duplicate Z selector while preserving the clear current Z-datum explanation. Add a small procedural stock diagram with setup axes, work zero, stock top/bottom and clearance, driven by actual values. Distinguish stock resize anchor from work zero. An outside-stock issue names source, side and overhang and offers existing remedies; never move artwork automatically.

Investigate live Setup spacing first: `numbers()` mixes unconstrained layouts and horizontal rows. Record settled-frame rectangles to find the source of the gap; do not assume lowering global spacing fixes it.

Acceptance: common stock dimensions, active datum and clearance are reachable without hunting; resizing obeys the selected anchor and does not move artwork; diagram is labeled when values are unset.

### 5. Machine for this job and reusable machines

Applied-machine inspector has identity/provenance, work offset, job tool mapping table, output behavior and advanced sections. Give T/H their own columns with units/meaning explained; every relevant job tool can be inspected without selecting its operation first. Setup remains the only mapping editor. Export shows a read-only summary and navigation link.

Keep reusable machine creation/editing in the resource workspace's Machines page, with explicit Apply to job. Preserve current settings: tool changes and return behavior, startup position, length compensation, coolant/path control, output precision, spinup, rapid-rate assumptions and holder selection. Group these; do not remove them to match the old concept. Custom holder segments remain read-only until an actual editor is separately implemented.

Acceptance: global edits and applied job edits are visually different; applying a machine leaves work zero unchanged; missing mappings remain explicit.

### 6. Job settings

A short inspector for shared motion and verification tolerances, with units and a small scope note. Default restoration is secondary. Keep geometry import tolerance where the actual source/import model owns it. Never combine display cell size, planner tolerance and G-code precision into one Quality slider.

Acceptance: every value keeps its existing owner and freshness effects; the panel is intentionally short, without filler.

### 7. Shared operation inspector

Sticky heading: ordinal + operation name, type and enabled state. Under it, compact tabs appropriate to the operation. Use the same form rows, group headers, tool/profile summary, validation, advanced sections and footer across types. Persistent search may collapse to a Find action for short panels; preserve Ctrl+F and discoverability.

Dense geometry lists get a summary and bounded internal list or explicit expansion, so hundreds of component rows do not bury depth and tool settings. Preserve source/layer grouping, unresolved-reference repair and selection methods. Store open/scroll state by stable operation ID and section, not visual row number. Inspect generated data in Result inspection, with links from authoring where helpful.

Acceptance: every existing field has a destination, documented against `state.rs::FIELDS` and the nonnumeric controls; no lost help, pending input, undo grouping or diagnostic focus.

### 8. Flat V-carve

Keep Shape & depth / Endmill / V-bit tabs. Shape contains geometry assignment, top reference/offset and maximum depth, then surface quality. Endmill and V-bit begin with a compact cutter/profile assignment; cutting values follow in aligned rows. Entry/ramp and planner limits are secondary groups, preserving capability checks and raw partial ramp drafts.

Explicitly explain the V-bit geometry's role in defining the target in Endmill only mode, even when no V-bit finishing motion runs. Do not hide necessary target geometry simply because the stage is inactive. Preserve combined/rough-only semantics, face-result height references, floor ridge, wall allowance and residual settings.

Acceptance: both modes and custom/library/modified assignments remain clear; unresolved preceding-face references link to the actual operation.

### 9. Face

Use Area & heights / Tool & passes tabs. Area shows stock/rectangle coverage and per-side margins, then top/bottom references and offsets. Passes shows cutter/profile, feeds, stepdown/stepover, 0°/90° direction and entry settings.

Use a procedural top-view diagram to distinguish requested coverage, cutter footprint, entry/exit travel, and explicit entry coordinate. Retain the current entry mode controls and their evaluated explanation. Margins and overrun must not look interchangeable. Keep pass stepdown separate from the tool's stepdown limit.

Acceptance: source-free Face job works; diagram tracks 0° and 90° and explicit entry; the published face plane remains available to following operations.

### 10. Profile

Use Geometry / Cutting / Tabs & entry, with summaries across tabs. Geometry keeps selected closed contours, per-contour retained side, traversal and order. Cutting owns tool/profile, heights, stepdown, through-cut allowance, direction and radial finishing. Tabs & entry owns automatic/manual tabs, tab sizes/anchors, seam/start anchor, plunge/ramp and line/arc leads.

Make manual anchor rows compact and connect them to the existing draggable viewport markers. Show requested anchors and generated bridges with distinct shape/labels; stale generated positions remain labeled. Retain the existing manual-tab constraint (currently one manual anchor per selected contour), count/spacing precedence, and numeric fraction editing. Do not imply arbitrary tab creation through an illustration.

Acceptance: tabs and seams remain source/contour-qualified after reorder; one drag is one Undo; finishing, leads and tab geometry remain inspectable at current/stale states.

### 11. Drag knife

Use Geometry / Cutting / Corners & start. Separate blade geometry from cutting assignment; group top/bottom offsets, pass stepdown, through-cut allowance, closure overlap, swivel depth/feed, corner threshold, path tolerance and initial alignment. Keep suggested values as an explicit action that fills blanks only.

Use an accurate small pivot-to-tip diagram and contextual viewport legend for holder/pivot path, blade tip and heading. Preserve spindle/coolant-off context, planner rejections, open/closed chain selection and optional copied-outline workflow. No rotary axis or automatic physical blade alignment is implied.

Acceptance: heading and offset use the same convention as the simulator; suggestions never overwrite authored cutting values.

### 12. Tool/profile assignment picker

Keep a focused contextual picker for choosing an existing job cutter or a saved library tool/profile. It should be smaller than the full library editor. Show target operation and stage in its heading, compatible types, geometry summary and selected profile. Distinguish **Use tool only** from **Apply tool & profile**; show Applied / Modified / Custom with text.

Keep Clear, Reset overrides, Reapply reviewed profile and Capture/save-as flows explicit. Changed values must not silently affect other assignments. An invalid/unsaved library draft cannot masquerade as its saved version.

Acceptance: selection is reviewable before apply; cancelling leaves the assignment intact; use the existing resource commands and revision checks.

### 13. Global tool library workspace

Closest match to revision-2 concept 02. Replace the floating editor with a center+right resource page while retaining the navigator. Left sublist: search and compact tool rows. Main top: tool identity, modest geometry illustration and geometry/capability fields. Main bottom: profile list/table beside the selected profile's details. Reserve a persistent footer for Save library changes and contextual Apply actions.

Reuse the existing library draft, validation, save/conflict, import/export and machine forms. Navigation away retains draft state as it does now; scope labels distinguish Saved locally from Unsaved library changes. Geometry appears once; profiles own cutting values. Include shaft/stickout and capabilities introduced after the concepts. Do not add flutes or material/machine metadata solely because the generated image depicts them; bind only fields supported by the current model.

Tool illustrations should be parameter-driven diagrams for endmill, truncated V-bit and drag knife, with optional declared shaft/holder. Never invent stickout or a holder body. A decorative generated render is optional and must not supply dimensions.

Acceptance: geometry and at least the selected profile can be compared without losing the tool list; global changes never silently update a saved job; import failures preserve drafts and explain recovery.

### 14. Job tools workspace

Closest match to concept 03. Center+right page with a tool table (name/geometry/usage/controller mapping), followed by a selected tool's assignments (operation/stage/profile/feed/spindle/status). Selecting an assignment reveals its details and Open operation / Reapply reviewed profile actions. Mapping is a readout linking to Machine.

Geometry editing explicitly states all affected assignments. Show provenance as secondary detail; never-configured and unused cutters are identifiable. Reuse current edit/use/capture commands first. Concept buttons such as Create job tool, Remove unused and geometry-only Add from library require a capability check: implement missing commands separately with their own acceptance criteria, or omit those buttons from the initial visual pass. Do not infer support from the mockup.

Acceptance: shared geometry and assignment-specific values are visibly distinct; a geometry edit previews its reach; changing one profile does not modify the other assignments.

### 15. View toolbar and canvas

One compact toolbar: selection/navigation tools as supported, view preset selector with Top and Isometric quick access, Fit, display mode and Layers menu. Keep Front/Back/Left/Right, continuous orbit and zoom accessible. Label **Fit view** versus **Fit playback** to eliminate today's repeated Fit ambiguity.

Move stock coloring (Plain/Tool/Operation), Solid/X-ray, opacity, surface matching and artwork/cutting/travel/edge visibility into coherent popovers. Retain chosen state with a small toolbar summary. Make the stock dominate by reducing chrome before altering the camera's fitting behavior.

Keep the pale grid and simple warm stock; refine lighting/edge contrast only after shell changes. Teal selected geometry, neutral unselected artwork, distinct travel paths, axes and tool marker need a stable legend. Annotation backgrounds must remain readable over plain, colored and transparent stock. Optional dimensions derive from actual geometry and appear contextually, not as permanent visual clutter.

Acceptance: camera gestures, DPI picking, culling, stock walls/transparency and artwork selection keep existing behavior/performance. No material texture is required for fidelity to revision 2.

### 16. Simulation transport and stage timeline

Collapsed transport: Play/Pause, Start, a speed dropdown (all existing rates and Fit playback), elapsed/total modeled time, one primary time track, stage boundary/jump control and display preset. Put precise motion-index seeking in an expandable detail row. Preserve a motion-based fallback when time is unavailable and label it.

Add a custom egui track with time-scaled stage segments, playhead and warning markers as a dedicated interaction task. It consumes existing stage/time/warning data; no new timing model. Keep stage jump controls until keyboard and pointer behavior of the new track is proven. Very short stages and coincident warnings need minimum hit targets/cluster details without falsifying duration.

Move bytes/pages/checkpoints/replay diagnostics out of the default transport into Diagnostics. Keep actual display resolution, warning count and rapid-time assumption visible in concise form. Hiding paths never rewinds cumulative stock. Cutter and holder must follow the same motion/time source as removal. No invented acceleration or tool-change time in labels.

Acceptance: seek backward/within arcs, stage jumps, pause/resume and 1× timing remain equivalent; warning markers seek the reported motion; rendering the collapsed transport uses about 96 points rather than several paragraphs.

### 17. Result inspection, sections and warnings

Right inspector tabs: Summary / Section / Warnings. Summary covers selected stage, resolved heights/passes, modeled times and relevant point/tool readouts. Section retains X/Y plots, pinned comparison and display-cell resolution. Warnings list groups existing problems with first occurrence and worst penetration, with Show seeking to the motion.

State that assembly/rapid warnings are display checks at the reported raster resolution; do not turn them into authoritative export pass/fail badges. Preserve separate authoritative diagnostics. Performance, transport and renderer failure-injection controls belong in a collapsed developer diagnostics surface.

Acceptance: inspect/pin/compare stale and current data without mixing identities; warning absence does not imply unmodeled assemblies were checked; sections remain legible at both panel widths.

### 18. Export dialog

Retain the current modal, rather than restoring the original full-page multi-file concept. Style it with the shared typography, form/table rows and status banners. Add a concise retained scope, ordered operation/tool summary, applied machine/work offset and mapping readout when available from the prepared/current authoritative data. Keep validation outcomes and Save as prominent; technical report, histograms and hash stay expandable.

Preserve preparing, cancel, validation failure, stale job, saving, save failure/retry and success. Save failure retains the exact validated bytes; stale edits block saving them. Links to fix a setting should close/leave the review deliberately and require a fresh preparation as current semantics dictate. Do not add file splitting or editable machine settings here to imitate concept 04.

Acceptance: export prefix never widens; retry saves the same prepared artifact; success closes the dialog; display-only edits never grant output authority.

### 19. Status, issues, recovery and empty states

Normal status is one short line: job/plan state, units and optional contextual readout. Separate exceptional banners for actionable recovery, failed I/O and renderer unavailable states. Successful automatic recovery-saving needs no permanent second row. Preserve Restore draft / Keep current, reload/clear recovery and save retry paths.

Issue list provides count, severity, operation and concise reason with Go to field. Opening an issue selects the owner, clears/adjusts filters as needed, expands the group and focuses the control. Technical codes remain available in detail.

Design explicit states for new job, no artwork, no operations, empty library, no search results, unset values, missing tool/machine, unresolved geometry, generating/cancelled/failed, stale plan, renderer failure and resource-save conflict. Each gets a short explanation and the existing relevant action, without decorative dashboards or implied machining defaults.

Acceptance: action is visible without scrolling a long log; no restored partial value disappears; unusual states are included in visual review.

## Assets and graphics

Create one shared icon registry and an asset manifest with source/license, nominal size and allowed colors. Standard actions use a coherent local vector set or egui painter primitives; custom CAM marks match its stroke/grid. Prefer vectors for small icons. Do not mix emoji, unrelated icon fonts and photographic thumbnails.

| Asset group | Inventory | Production approach |
|---|---|---|
| Shell/navigation | App cube, stock, machine, library, job settings, SVG, eye/hidden, lock/unlock | Local vector/painter; 18/20 variants |
| Operation/tool types | Face, Flat V-carve, Profile, Drag knife; endmill, V-bit, blade | Custom consistent vector silhouettes; verify readability at actual size |
| Actions | Add, more, import/export, replace, duplicate, trash, undo/redo, move, search, help | Same vector family; labels/tooltips and keyboard names |
| View/transport | Top/isometric/view, fit, pan/orbit where supported, layers, play/pause/start, warning | Same family, clear selected/disabled variants |
| Parameter diagrams | Stock/datum/clearance; Face coverage/entry; cutter dimensions; knife pivot/tip; profile tab/lead | Procedural egui drawings driven by real fields, with text laid out by egui |
| Optional polished imagery | Generic cutter renders or a very subtle stock texture | Image generation can explore style; transparent high-resolution source, native text overlay, no baked-in measurements; optional last phase |

Image generation is useful for additional overall visual explorations or decorative assets, not for measured geometry, numeric text or the primary 18-point icon set. The current pass includes an interactive layout study instead: switch among artwork, stock, operation, library, job tools and simulation. It is illustrative, not a working CAM preview; values are synthetic. The written plan governs behavior and scope.

## Implementation shape

Keep changes in `cam-gui`. Proposed small shared modules: `ui_theme` (semantic tokens/density), `ui_widgets` (field/section/status/table/toolbar), `ui_icons`, and `workspace_route` (selected page/entity). These are suggested module names, not an architectural rewrite.

Replace overloaded numeric navigation and window booleans gradually with explicit routes for Prepare inspector, Simulate inspection, Tool library, Machine library and Job tools. Preserve active operation and artwork IDs separately from the visible route. Resource pages suspend presentation of the viewport, not the document or retained computation. Define playback policy explicitly: pause when opening a resource page, retain the playhead, do not resume automatically. This is an intentional small interaction change to review.

Keep command dispatch, revision/freshness checks, resource drafts, platform I/O, worker protocol and planner untouched unless a separately named convenience feature requires changes. Reuse stable field IDs and observation probes; visual restructuring must not break issue routing or automated interaction names.

| Work area | Primary code |
|---|---|
| Tokens, header, navigator | `workspace_ui.rs`, `app.rs`, `operation_list.rs` |
| Shared forms and stock/machine/artwork | `inspector.rs`, `state.rs`, `help.rs`, `issues.rs` |
| Operation-specific forms | `operation_ui.rs`, `face_ui.rs`, `profile_ui.rs`, `knife_ui.rs`, `tool_picker.rs` |
| Resource pages | `library_modal.rs`, `resource_ui.rs`, `resources.rs` |
| View toolbar, transport, inspection | `viewport.rs`, `viewport_inspection.rs`, `stock_style.rs` |
| Contextual diagrams/overlays | `viewport_artwork.rs`, `viewport_face.rs`, `viewport_profile.rs`, `viewport_knife.rs`, `overlay.rs` |
| Export and exception states | `export_ui.rs`, `file_io.rs`, `recovery.rs`, `resume.rs` |

## Delivery sequence and review gates

| Slice | Deliverable | Dependencies / acceptance |
|---|---|---|
| V0 — baseline | Fresh native/browser captures and field/control destination inventory | Same fixtures, window size, DPI, view, playhead, fonts and committed build; establish layout/performance baseline |
| V1 — visual foundation | Tokens, icons, bounded field rows, section/table primitives; Stock panel as proving case | Fix spacing cause; preserve raw text, help/focus and duplicate-Z cleanup; approve at 1280 and 1440 |
| V2 — workspace shell | Header, navigator, anchored tools, scope/status, overflow and panel sizing | Keep all operation commands and empty-job access; compare full workspace to revision 2 |
| V3 — resource workspaces | Full-width library/machines and Job tools; focused assignment picker | Explicit routes and draft preservation; tables use existing command semantics |
| V4 — authoring panels | Artwork, Machine, Job settings; four operation editors and procedural diagrams | Apply field/control inventory; cover geometry repair and anchors; review each type separately |
| V5 — inspection | View toolbar, compact transport, stages/warnings, result inspector | Preserve seek/time/picking behavior; custom timeline is its own bounded task |
| V6 — completion | Export/recovery/error/empty-state polish; both-platform visual audit | Review exact prepared scope and retry behavior; optional decorative graphics only after core acceptance |

V1–V3 should deliver the largest visible improvement. Avoid doing only a palette/icon pass and calling the redesign complete. Review each slice as real egui screens; approve the style once and reuse it rather than independently restyling every panel.

Separate optional backlog: Center on stock; new resource management commands missing from the current API; drag operation reorder; additional font; textured materials/photoreal tool renders. None is required to achieve the concept's layout.

## Verification and definition of done

Capture deterministic native and Chromium/WebGPU screens at 1280×800, 1440×900 and 1920×1080, with 100% and 150% DPI coverage and a 200% effective-width check. Include one long filename, duplicate operation names, twelve operations and dense component lists. Use frozen playheads for image comparisons. Compare real screenshots side-by-side with the concept; use golden diffs for accidental change, not pixel similarity to AI imagery.

Fixture matrix: multi-source artwork; source-free Face; combined and Endmill-only flower; Face→carve height reference; Profile with tabs/leads/finish; Drag knife with open/closed chains; shared cutter across operations; custom/modified library profile; unmapped machine; prefix-generated output; tight holder warning; stale/partial/recovery/failure states.

Behavior checks should reuse existing `cam-gui/tests` and browser scenarios. Add focused tests only for new behavior or meaningful risks: bounded layout/non-overlap, route/draft preservation, issue-to-field navigation, eye/lock hit targets, stage/warning seeking and exact export scope. No tests that merely assert a color constant. Run fmt/clippy and appropriate GUI tests per slice, then affected browser scenarios and native input checks. Compare frame time/picking responsiveness on the dense flower; icons and diagnostics must not add avoidable per-frame asset decoding or large allocations.

Done means every numbered panel has a reviewed normal state and relevant exceptional state; all existing controls have a reachable destination; forms are legible without clipping; Job tools and current scope remain discoverable; draft/Undo/recovery and checked-export behavior remain intact; both targets agree on layout and interaction within their existing qualified envelope. This planning pass did not run the application test suite because application code was not changed.

The accompanying conversation layout study was checked in headless Chromium across twelve views at 320, 736, 1024 and 1440 pixels: no page-width overflow or JavaScript exceptions in those 48 combinations; navigation and the illustrative warning-to-playhead action worked. Artwork, library and simulation renders were visually inspected. This verifies the study only, not an egui implementation, final icon rendering in the host, or CAM correctness.
