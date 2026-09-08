# Web UI: product design and delivery plan

Date: 2026-09-05, consolidated 2026-09-08 from the former product-design, integration-plan, simulator, and WebAssembly reports (all retained in Git history)\
Status: product baseline and delivery roadmap. U1–U3, U5, and U7–U9 are implemented: local service, background planning, 2D stock inspection, M5 verification, M6 checked output, the local tool library, streamed plan storage, the 3D stock simulator, and the static WebAssembly build. U4 (engine-derived 3D/section inspection and large-artwork rendering) and U6 (native file lifecycle, durable recovery, release qualification) remain, together with the M6 controller validation and the M8 machining trial tracked in the [implementation plan](implementation-plan.md).

This document expands M7 into a complete product experience. Features described below remain product targets unless marked delivered; per-slice validation evidence lives in the Git history of the former `web-ui/` reports.

## 1. Product intent

Help a CNC-router operator turn SVG artwork into an inspectable, reproducible combined endmill/V-bit job and, when verification and machine output are available, a LinuxCNC program. The user specifies the desired shape and actual setup; the planner coordinates the two tools.

The interface should answer five questions continuously:

1. What artwork and physical dimensions am I machining?
2. What finished shape do these settings request?
3. What will each tool remove, and what remains?
4. What has actually been checked, and where are the unresolved problems?
5. Does the output correspond to this job and this machine profile?

Primary use is a single operator on a local desktop or laptop, with mouse and keyboard. The baseline is bundled TypeScript browser assets served by a local Rust application; the same bundle also runs statically through the in-browser WebAssembly engine (§11), with the native application remaining the everyday default. Everyday OS and supported browsers still need confirmation. Tablet layouts should support inspection and editing; phone layouts are for occasional review and recovery, not the primary CAM workspace.

## 2. Scope of a fully featured first release

Included: portable jobs; SVG inspection and region selection; physical placement; stock/depth/tool/quality setup; endmill-only and combined planning; target and stock visualization; motion inspection; spatial diagnostics; verification review; machine-profile setup; combined/per-tool export; local recovery; undo/redo; keyboard access; reproducible reports.

Local reusable tool, setup, and machine presets are proposed conveniences. Applying one copies a snapshot into the job, previews the changed values, and invalidates affected output. Editing a preset must never silently alter existing jobs. No feed or spindle recommendations are generated. Imported jobs retain unset machining values until the user supplies them or explicitly applies a preset.

The [local tool library](tool-library.md) is implemented: tool definitions, optional cutting presets, persistence, browser management, job capture, and reviewed, undoable snapshot application through the Rust store. Setup and machine preset libraries remain proposed.

The established product boundaries still apply: one flat endmill, one V-bit, one depth cap across selected regions, uniform flat stock, and XYZ motion. CAD drawing, SVG repair/tracing, multiple depths, arbitrary tool stacks, automatic feeds and speeds, cloud accounts, collaboration, and direct machine control remain outside this release. Stock footprint/clamp visualization is an optional future extension; current stock data contains thickness only, so the UI must not imply fixture collision coverage.

## 3. Workspace structure

The large viewport stays in place while the user moves between steps. A step is a useful editing context, not a modal page that erases the view. Users may revisit any step and inspect a partial job. Prerequisites gate calculations and output, not navigation or saving an incomplete job.

| Area | Content and behavior |
| --- | --- |
| Application bar | Job name, saved/recovery state, Open, Save/Save as, undo/redo, connection state, primary action. File management lives here. |
| Left navigator | Artwork; Stock & origin; Carve & tools; Plan & inspect; Verification; Export. Each has a text state such as Needs input, Available, or Needs update. Expand source components under Artwork and stage/motion families under Plan. |
| Center viewport | Shared 2D/3D camera, selection, target/stock modes, toolpath layers, cross-section tools, location highlights. Local view controls belong here. |
| Right inspector | Settings for the active step or selected item. A heading explains whether it is showing job settings, a region, a motion, or an issue. Back returns to the previous context. |
| Bottom drawer | Issues, computation activity, and motion sequence. Selecting an issue focuses the viewport and relevant field. Expand details when needed instead of permanently shrinking the canvas. |
| View status strip | Units, datum, selected object or cursor coordinates, displayed artifact freshness, and visual resolution. Verification status remains separately visible. |

At large widths, use adjustable left and right panels around the viewport, with sensible minimum widths. At laptop widths, collapse the source tree before squeezing the inspector or drawing. At narrower widths, use a wrapping step navigator and stack the inspector below the viewport. Preserve essential actions in every layout. Do not require precise hover to reveal them.

Proposed visual direction: a restrained engineering workspace, neutral surfaces, compact labeled controls, clear section spacing, tabular numeric fields, and a large calm drawing field. Follow the system light/dark preference with a user override. Use a single interaction accent and consistent geometry colors. Avoid dashboard cards, decorative metrics, and material textures that obscure stock errors.

## 4. Workflow and screen specifications

### 4.1 Open or create a job

The start view offers Open SVG, Open job/plan, and recent local jobs. File drop is a secondary shortcut. Examples are explicitly labeled synthetic and never become machining presets. No account or network connection is needed for the ordinary workflow.

Opening a job rebuilds normalized geometry through Rust. Opening a plan rechecks identity and recomputes its analysis according to engine compatibility. A filename or cached green badge does not establish validity. Show migration results and newly missing settings; preserve the original until the user saves. Unsupported future schemas produce an actionable error and preserve the original file.

A plan opens as an inspection artifact with its embedded job. "Edit as job" creates an editable draft and retains the original artifact. Users can inspect partial plans and download diagnostic artifacts even when machine output is unavailable.

### 4.2 Artwork

Show the imported physical page dimensions, artwork bounds, source filename, and selected-region count. Let the user inspect actual normalized geometry before machining setup. Raw SVG must not be injected into the page as active markup; use normalized data or an inert rendered preview.

Selection works both on the canvas and through an accessible source/component list. Display SVG labels when present and stable component IDs in details. Support click selection, additive selection, Select all, Clear selection, and Fit selection. Keep visibility toggles separate from machining inclusion. Preserved holes belong to the selected filled region; they must not appear selected for removal just because their parent is selected.

Keep separate notions of an inspected item and the set of regions included for machining. A click on a toolpath or issue should not change machining selection. Include explicit controls for inclusion so selection changes are visible and undoable.

Unsupported SVG features produce a list with the source element when available and a concrete remedy, such as converting text/strokes to paths in Inkscape. Do not silently drop unsupported artwork. An invalid replacement import retains the current job. "Replace artwork" previews changed components; do not assume old IDs identify the same shapes after reimport. Require review of the new inclusion set.

### 4.3 Stock & origin

Expose stock thickness, artwork scale, linked physical width/height, rotation, and XY origin. Show stock top as Z = 0 and machining depth as positive downward in forms; display machine Z as negative in motion inspection. Name page bounds, artwork bounds, and any future stock bounds distinctly.

Origin controls offer page/artwork anchor presets and an explicit source-page XY point. Display the resulting workpiece axes immediately. Preserve the core convention `workpiece XY = scale × rotate(page XY − origin)` with page Y upward. "Move origin" and "move artwork" must not be ambiguous drag modes. Arbitrary extra translation, independent X/Y scaling, and stock X/Y size are not current job fields and must not be invented in serialization.

Expose clearance Z and starting XY in a Travel section. Before M6, clearance comes from endmill planning. The current machine-profile snapshot also has a clearance field; the service must report mismatches and later provide one resolved editing policy instead of letting the UI silently choose one.

Changing artwork scale does not scale cutter diameter, depth, feeds, or tolerances. Show that consequence next to the scale control, list changed physical bounds, and invalidate the plan. Numeric placement is always available as an alternative to canvas manipulation.

### 4.4 Carve & tools

Lead with desired shape: maximum carve depth, horizontal endmill wall allowance, maximum floor ridge, and permitted cutter-limited detail residual. Use a labeled cross-section to explain sloped walls, the broad flat floor, shallow narrow details, and the finite tip. Desired target and cutter-achievable result are separate views.

Below the shape settings, show the two fixed tool roles in execution order: Endmill clearing, then V-bit rest machining and finish. No freeform operation reordering is offered. The endmill-only option still requires V-bit geometry because its angle defines the target.

| Group | Editable settings |
| --- | --- |
| Endmill geometry | Diameter, usable cutting length, explicit plunge capability. |
| Endmill cutting | Spindle RPM, cutting feed, plunge feed, stepdown, stepover, ramp capability. |
| Endmill strategy | Depth-dependent or deepest-region clearing; plunge or ramp entry; ramp angle and feed when selected. Explain practical behavior with a diagram, without deriving paths in TypeScript. |
| V-bit geometry | Included angle, actual flat-tip diameter (zero for pointed), maximum cutting diameter, usable cutting height. Label included angle so it cannot be mistaken for the half-angle. |
| V-bit cutting | Spindle RPM, cutting/plunge feeds, stepdown, stepover, explicit plunge capability. Current M4 supports direct plunge and does not offer V-bit ramps. |
| Quality | Floor-ridge and cutter-limited detail allowances, with affected geometry linked from results. These are physical finish choices. |
| Advanced accuracy | Import geometry tolerance, motion tolerance, verification tolerance; integer precision as Auto or an explicit supported value. Numerical error stays separate from physical allowances. |
| Advanced computation | Endmill layer/loop/motion limits; V-bit path/motion/subdivision/depth-pass/cleanup limits; quality-sample spacing/count; reachability-cell and stock-slice limits. Preserve all current fields even when the section is collapsed. Empty planning-budget and tolerance fields resolve to visible engine defaults. |

Use units beside every numeric input. Preserve the user's text while they type; an empty field is not zero. Unset capability is "Not specified," distinct from Yes and No. Show missing fields as a completion checklist; invalid supplied values receive inline errors. Rust owns the authoritative validation and supported ranges. The UI may check syntax and finite numbers immediately, but it must not duplicate geometric feasibility rules.

Do not preselect an unconfirmed machining value. Numerical/resource presets, if introduced, must be engine-provided, explicit, and recorded. A current implementation requiring explicit detail residual overrides any older prose suggesting an automatic zero default.

### 4.5 Plan & inspect

Primary action is Generate plan, with Combined and Endmill only modes. The normal combined workflow requires V-bit planning settings explicitly; do not inherit the CLI's implicit endmill-only behavior when that block is absent. If fields are missing, the action opens the completion checklist and focuses the first required field.

Computation stays in the background. Show named stages, elapsed time, available counts, and a cancel action. Only display percentages when the service has meaningful completed/total work; never fabricate an ETA. The previous result can remain visible with its revision and a Needs update label. Edits are allowed while planning; a completed older task becomes a historical result and cannot replace the current one.

Cancellation keeps the last complete result and returns the draft to an editable state. Incomplete diagnostic results are inspectable when the engine intentionally returns them; cancellation fragments are not promoted into usable plans.

The viewport supports these modes:

| Mode | Main question | Data and interaction |
| --- | --- | --- |
| Artwork / target | What shape is requested? | Selected opening, holes, depth cap, nominal target, cutter reachability where available. |
| After endmill | What remains for the V-bit? | Actual endmill sweeps, layer selection, remaining target and wall allowance. |
| After V-bit | What is the combined result? | Actual combined stock, boundary/detail/floor path families, final finish. |
| Residual / error | Where does result differ? | Separate missed reachable stock, allowed floor ridges, cutter-limited detail, possible overcut, and unresolved cells. |
| Cross-section | What happens through this detail? | User-defined section across the model; nominal target, removal, and available uncertainty bands with XY location and depth units. |
| Motion inspection | How do the tools move? | Sequence scrubber, tool transition, entry/retract/rapid/cut visibility, current XYZ/feed/tool/layer, selection linked to the source region where available. |

2D top view with physical axes is the primary view and is delivered. A 3D stock surface with motion playback is delivered as the labeled visual simulator (§10); it renders recorded cuts at a stated resolution and creates no verification or planning claim. Engine-derived 3D inspection against target/error data and arbitrary section queries remain U4 features with new display-data contracts. Navigation includes fit job/selection, pan, zoom, top/isometric presets, and reset camera. Preserve camera and layer choices across settings edits where useful.

Playback renders exactly the recorded motions at a labeled visual resolution and creates no verification or planning claim. A scrubber can use motion index or normalized progress before credible timing exists. Any future duration estimate must state its modeled scope and exclude unmodeled tool-change/probing behavior. A low-resolution mesh is labeled Visual preview; changing display resolution must not alter a verification result.

Keep the preview selectable at high path counts using rendering detail levels and separate hit-testing data. If display paths are simplified, the selected motion inspector still reports the original numeric segment.

### 4.6 Verification

Use a review screen with an overall scoped result and an issue list linked to the drawing. Never compress every form of evidence into a green checkmark.

| Evidence | Example display language | Consequence |
| --- | --- | --- |
| Target visualization only | Target preview — no planned cuts | No planning or verification claim. |
| M3/M4 complete | Planning checks complete — slices and samples | Display the actual check scope and limits. Does not enable machine export. |
| Incomplete | Plan incomplete — reachable floor remains | Highlight available evidence; provide a relevant editing or diagnostic action. |
| Inconclusive | Verification inconclusive — refinement limit reached | Show unresolved region/bounds if supplied; allow an explicit settings change and rerun. |
| M5 passed | Required geometric bounds verified | Report bound, requested tolerance, scope, artifact identity, and model limitations. |
| M5 failed | Verification failed — overcut exceeds limit | Block machine output and focus the finding. |
| Old artifact | Needs update — settings changed | Retain for comparison; invalidate export eligibility. |
| M6 output checked | Formatted motion checks passed | Applies to the exact generated output/profile, not every future export. |

The detail table includes code, severity, stage, affected region/motion when known, measured value or interval, requested limit, and the next useful action. Distinguish sample maxima from global bounds. M4 numerical depth budget and XY coverage tolerance are shown in evidence details; do not label a sampled residual maximum "maximum error." A missing location is shown as job-level, never as a fabricated map marker.

Keep geometry findings visually distinct: endmill paths use blue; V-bit paths teal; missed reachable stock pink with hatch; possible overcut purple with crosshatch; cutter-limited detail amber with a labeled boundary; unresolved results use a neutral stipple and explicit label. Selection uses a separate outline. Legend toggles update the actual drawing, and issue labels make the meaning independent of color.

### 4.7 Export

Machine setup contains the LinuxCNC profile snapshot, work offset, tool-number mapping, clearance policy, tool-length-compensation ownership, M6 contract, and output precision when M6 supplies them. Current `m6_contract` is editable prose, not validated macro behavior. Do not offer an "accept" checkbox as a substitute for the missing backend contract.

The export screen summarizes current job/plan/profile identity and required checks, then offers a combined program or independent per-tool programs. Preview generated program text alongside the tool sequence and formatted-motion findings. The service generates and validates the exact bytes before making files downloadable.

Offer a job snapshot, plan/report, and setup summary alongside machine output. Make Download job/report distinct from Export machine program, so blocked machine output does not prevent saving or debugging. Per-tool output must be self-contained according to the M6 contract. The UI does not send programs to LinuxCNC or expose a Run machine control.

## 5. Editing, files, and recovery

- Save valid incomplete jobs without requiring a plan. Temporarily malformed field text belongs to local recovery state; it must not overwrite a valid portable job.
- Undo/redo operates on meaningful edits such as a field commit, selection change, placement move, or preset application. Camera changes have separate view history if needed. Undoing an edit still requires identity checking before an old artifact becomes current again.
- Distinguish Saved to file, Recovery draft saved, and Unsaved changes. Autosave must not claim the portable file was written. Keep a recovery snapshot across reloads and service restarts, with time and original file identity.
- Detect externally changed job files before overwriting; offer reload, save a copy, or an explicit overwrite. Two browser tabs should not silently overwrite the same draft: propose a single editor lease with takeover, while other tabs can inspect.
- Ordinary file open/save uses explicit file selection or a service-mediated local dialog; browser downloads are a portable fallback. Do not rely on remembered browser handles as the only storage mechanism.
- Reconnecting restores a service-owned task snapshot or clearly states that the task was lost after restart. Never infer successful completion from a disconnected progress bar.

## 6. Accessibility and usability requirements

All essential canvas actions have numeric/list equivalents. Controls have persistent labels and units, errors link to fields, focus is visible, dialogs restore focus, and keyboard commands do not intercept typing in inputs. Selection is synchronized between drawing and list. Use platform-appropriate shortcuts for save and undo/redo, plus Escape to exit a tool and a visible shortcut reference.

Support reduced motion, scalable text, both themes, and clear focus/highlight contrast. Do not depend on red/green alone. Announce stage changes and result summaries accessibly without announcing every motion frame. Touch users can inspect a selected point or issue without hover.

Long job names, translated-length labels, many components, narrow windows, and long diagnostic messages are design test cases. The viewport and heavy parsing/rendering work must not block typing, navigation, or cancellation. Performance targets are established from representative fixtures and recorded hardware, not arbitrary promises.

## 7. Ownership and service architecture

Rust remains authoritative for SVG normalization, geometry, machining rules, planning, stock/reachability calculations, verification, artifact identity, migration, and postprocessing. TypeScript owns forms, navigation, local draft text, selection presentation, cameras, display rendering, and task/result presentation.

The UI programs against a replaceable `CamService` interface with three adapters: deterministic captured fixtures, the same-origin local HTTP service, and the in-browser WebAssembly worker (§11). The UI must not call the CLI by shell command from browser code or copy machining formulas into client validation. CLI behavior remains a parity oracle for identical inputs and engine versions; the live integration checks compare them on every run.

Implemented capability inventory:

| Capability | Status |
| --- | --- |
| SVG import and inspection | Embedded source, normalized source components, physical bounds, stable selection IDs, diagnostics. |
| Portable jobs | Schema 3, migrations from 1/2, nullable machining fields, validation. |
| Endmill/combined plans | Background tasks with cancellation, bounded paged motion previews, stage/layer filters. |
| Stock/quality inspection | Layer/slice polygons and point samples with residual/reachability evidence, diagnostic region links. |
| Continuous stock verification | M5 service/UI for combined plans; original and rounded-coordinate scopes. |
| Machine profile/output | M6 profile editing, combined/per-tool output, exact-byte readback checks and gated downloads. |
| Local browser service | `cam-web`/`cam serve`, same-origin `ui-7` API, shared planning/verification/export queue, bundled UI. |
| 3D and arbitrary cross-sections | Browser-side visual simulator (§10). Engine-derived display meshes/heightfields and section queries with error/resolution metadata remain U4 work. |
| Local tool library | Named tools/presets, revisioned persistence, import/export, capture, changed-value review, undoable application (§ [tool library](tool-library.md)). Durable recovery of unsaved library forms and separate setup/machine preset libraries remain proposed. |

The service supports 32 MB SVG sources and 64 MB job JSON and advertises these limits and a 128.1 MB request envelope limit through capabilities; saved plans and display responses retain separate bounds. The local server binds to loopback only, serves bundled assets and API from one origin, rejects unexpected Host/Origin values, and requires a session header on document/capability requests. Long computations run in private compute workers (the native service relaunches its own executable; the wasm mode spawns disposable Web Workers) with bounded queues, five-minute timeouts, cancellation, and retained-result eviction. Complete plans stream into service-owned temporary files; verification and export reopen those files directly, so plan size no longer controls worker-message size. The wire contracts, resource limits, and development workflow are documented by the [web workspace README](../../flat-v-carve/web/README.md) and enforced by its live tests.

## 8. State, identity, and cancellation

Model independent state dimensions instead of one overloaded `status`:

| Dimension | States / values |
| --- | --- |
| Document | No job; editing revision; valid incomplete job; valid configured job; invalid draft text. |
| Persistence | Unsaved changes; recovery saved; saved to file; external conflict. |
| Service | Connecting; available; disconnected; incompatible. |
| Task | Queued; running; cancellation requested; cancelled; succeeded; failed. |
| Planner outcome | Complete; empty; incomplete; inconclusive, retaining the engine's scope and meaning. |
| Verification | Not available; not run; running; passed; failed; inconclusive, with named check scope. |
| Freshness | Current; stale; incompatible engine/schema. |
| Output | Unavailable; blocked with reasons; generating; failed; checked files available. |

Task success means the computation returned a result; it does not mean the planner completed or verification passed. An empty endmill stage can be valid for a V-bit-accessible region, while an entirely empty selection cannot generate a useful combined job. Read the engine's interpretation instead of translating every empty state into an error or success.

Revision rules (implemented and load-bearing for future work):

1. Commit each document edit as a monotonically increasing revision. Keep transient numeric text separate until parsed; invalid text blocks computations based on that draft.
2. Start tasks from an immutable snapshot and store its engine-issued input identity.
3. Apply progress only to its task ID. Deduplicate/reorder events by sequence and reconcile after reconnect.
4. A terminal result becomes current only when its identity matches the current accepted job and engine. Otherwise retain it as a labeled previous result.
5. Cancellation/completion races are resolved by service terminal state. A superseded or cancelled task never silently installs a current result.
6. Verify/export recheck identities server-side at time of use. Client button state is only a helpful reflection of that decision.

Conservative invalidation baseline: every serialized job change invalidates the current plan because the engine hashes the whole job, including metadata/profile fields. Camera, visible overlays, and inspector navigation do not change the job. Finer dependency-based reuse is a later Rust contract; the UI must not promise that a name/profile edit preserves current fingerprints.

An export binds the current job, motions, verification settings/evidence, machine profile, postprocessor configuration, output precision, and resulting bytes. Output formatting can fail after a geometric plan passes. Keep that failure separately inspectable and never reuse a checked-download status after regeneration with changed options.

## 9. Data and rendering strategy

Maintain four separate stores: portable job data; recoverable form text/edit history; immutable artifact/task metadata; view state such as camera, selected item, and layers. Do not serialize view caches into the strict Rust `Job` schema. A local preset/recent-job index is separate application data with its own versioning.

The 2D viewport renders normalized polygon rings and motions through a renderer abstraction (SVG today). The U4 3D inspection view consumes engine-derived display data; display tessellation, culling, and camera math are UI concerns, but target depth, stock removal, and verification are not.

Avoid transferring whole large JSON artifacts for every control change. Summaries, diagnostic metadata, and bounds arrive first; use artifact-keyed geometry/motion chunks or typed buffers for heavy views (implemented for motions: bounded pages fetched until the preview is complete). Cancellation of display requests and worker-based decoding should prevent obsolete work from blocking the latest view. Resource exhaustion produces a reduced visual representation with a visible resolution label or a clear failure; it never changes evidence silently.

Synthetic cutting settings stay in examples and tests only. No frontend framework or renderer choice beyond the current pinned stack is implied for U4; select any addition by build/distribution simplicity, accessibility, testability, and measured renderer workload.

## 10. 3D stock simulator (delivered)

A browser-side 3D material-removal view driven by the recorded motions of a current plan task: the stock starts as a slab, the recorded endmill/V-bit moves carve it with an animated tool, and the operator can play, speed up (0.25–200×), step, scrub, or jump to the finished result. It runs entirely from data the service already exposes (paged motions, tool profiles, stock thickness); no Rust, schema, or HTTP changes were made for it, and simulation runs in a Web Worker.

Ground rules that must survive future changes:

- **Visual preview only.** M5 remains the only authority for overcut, residual, and quality bounds. The simulator applies exactly the motions the Rust engine recorded — it never invents, extends, or reorders cuts — and the UI labels it *Visual preview* with its cell size.
- **Heightfield representation.** A tiled Uint16-quantized heightfield (floor-rounded, very slightly conservative) with a parallel tool-owner channel for per-stage coloring. Target cell size is `min(0.1 mm, smallest cutting detail / 4)` with a hard 8192-texel long-side cap and a 64 M dirty-cell budget; caps are named constants with rationale beside them in `web/src/sim/`. Heights upload as R16F half-float textures (normalized R16 returns zero in vertex shaders on ANGLE/D3D11); rewind uses a bounded undo log rather than full snapshots.
- **No new serialized fields.** The stock XY rectangle is display-only, derived from nominal-target bounds inflated by the largest tool radius plus 1 mm. Jobs keep storing thickness only.

Measured behavior on the reference machine: playback sustains well above the 15× real-time gate (29.7× measured in-browser, 660–8,400× in Node benchmarks); flower-scale one-shot seek stays within seconds to ~1.5–2 minutes for the full-job End jump with continuously updating progress; orbit runs at 70 fps at the 9.87 M-triangle worst case on an RTX 3090. An M5 cross-check integration test holds the simulator's removed area inside every published depth-band interval. Integrated-GPU fps validation remains hardware-pending.

Phase-3 extension proposals (unordered): rapid/approach collision flags (a cheap byproduct of the heightfield); a cross-section plane; result mesh export (STL/GLB); per-operation coloring; zoom-region full-resolution detail (clipmap-style); refining the display stock rectangle from a loaded verification report's `domain`.

## 11. Static WebAssembly deployment (delivered)

The planning pass runs in the browser as WebAssembly behind the same `CamService` contracts, so the UI build can be statically hosted with no local service. The native portable application is unchanged and remains the everyday default.

| Piece | Location | Role |
| --- | --- | --- |
| Engine serial fallbacks | `cam-core` | One-worker hosts take serial paths instead of panicking on `thread::scope` spawns; native behavior and result order unchanged. |
| `cam-service` crate | `crates/cam-service` | Pure DTOs shared by the HTTP service and the browser build; both adapters emit byte-identical wire shapes. |
| `cam-wasm` crate | `crates/cam-wasm` | wasm-bindgen entry: document commands, plan/verify/export compute, admission, limits, default options. JSON-in/JSON-out, tested natively. |
| Browser workers | `web/src/service/wasm/` | A parent worker owns the engine instance and the task ledger; a disposable compute child per task mirrors the native compute process. Cancellation terminates the child. |
| Service selection | `web/src/service/wasm.ts`, `auto.ts`, `main.tsx` | `createWasmService` implements `CamService` over the worker; `?mode=wasm|live|fixture` overrides automatic detection (probe `/api/v1/session`, else wasm). |
| Build integration | `web/scripts/build-wasm.mjs`, `pnpm build:wasm` | wasm-pack builds `cam-wasm` for `wasm32-unknown-unknown` into `web/src/wasm/gen`; `pnpm build` runs it first (~640 KB gzipped module in the bundle). |

The wasm service reports the same capabilities, envelopes, task snapshots, paged motions, slices, and verification/export results as the HTTP service, so all existing zod schemas validate it unchanged. Tool-library operations are not offered in wasm mode and the UI disables those controls. Retained plans live in the parent worker's memory instead of temp files.

Limits and future work: single-threaded execution costs roughly 4× the native multithreaded time on equivalent artwork; browser memory holds the retained plan; Safari/Firefox were not exercised. Threaded execution remains future work — it requires cross-origin isolation (COOP/COEP response headers, not available on every static host) and moving the engine's scoped thread sites to a browser-compatible scoped-parallelism scheme. The getrandom `wasm_js` opt-in (for the transitive `boostvoronoi` → `cpp_map` → `rand` chain) is declared once in `cam-wasm` with target flags in `.cargo/config.toml`, inert on native targets; `Instant`/timers stay unavailable on the browser target behind the existing `Timer` gate.

## 12. Delivery stages

These U stages expand M7; they do not rename or block the Rust M stages.

| Stage | Deliverable | Status |
| --- | --- | --- |
| U0 — design baseline | Screen map, interaction model, capability gaps, wireframes, terminology. | Done (this document). |
| U1 — shell and contracts | Scaffold, form conventions, mock/live adapter boundary, navigator/viewport/inspector. | Done. |
| U2 — artwork and setup | Open/import, physical placement, region inclusion, settings, save/reopen, undo/recovery. | Done. |
| U3 — plan and 2D inspection | Background tasks, stage progress, cancellation, stale-result protection, paths/slices/issue drawer. | Done, including bounded stock slices and diagnostic links. |
| U4 — full visual inspection | Engine-derived 3D stock view, section tools, large-artwork interaction. | Open. The visual simulator (§10) covers playback and the carved surface; comparing source/desired/residuals in 3D and arbitrary sections need new engine display/query contracts. |
| U5 — verification and output | Bound-aware review, spatial diagnostics, machine profiles, formatted-output preview, downloads. | Done, including exact-output parity and server-side outcome gating. |
| U6 — usable local release | Local presets, file conflicts/recovery, offline packaging, lifecycle/browser checks, help and first-run workflow. | Open; folded into the M8 release work. |
| U7 — plan storage | Complete plans on disk, bounded browser responses, streamed artifacts. | Done. |
| U8 — 3D stock simulator | Animated material-removal preview (§10). | Done through phase 2; integrated-GPU fps validation pending. |
| U9 — WebAssembly engine | In-browser engine for statically hosted UI (§11). | Done; threaded execution future work. |

No calendar commitments are made here.

## 13. Remaining acceptance scenarios

Scenarios not yet fully covered, for U4/U6 and release qualification:

- 3D comparison of source, desired shape, each tool's result, and residuals, with approximation labels surviving every view (U4).
- Arbitrary cross-section queries against engine-derived data (U4).
- Large motions/components on real artwork: typing, cancellation, navigation, and issue selection remain usable; drawing detail reduction is explicit (U4).
- Unsaved changes, malformed draft, external file edit, and two tabs: recovery is distinct from portable save; conflicts do not overwrite unnoticed (U6).
- Full assistive-technology qualification, supported-browser matrix, and a clean-machine installation test on the intended everyday host (U6/M8).

Use focused UI state tests for identity/cancel/recovery/export gating, browser scenarios for the ordinary workflow and key failures, and contract fixtures checked against Rust. Reuse existing machining tests; do not duplicate planner mathematics in frontend tests. Compare normalized settings, identities, motion sequences, and report semantics for CLI parity rather than relying on screenshots.

## 14. Open decisions

- Primary everyday host/browser and preferred information density on the user's actual display (release qualification).
- Clearance-field reconciliation between job planning settings and the machine profile: report mismatches now; define one resolved editing policy later.
- Whether future versions intentionally separate geometry, plan, profile, and output dependencies instead of whole-job fingerprint invalidation.
- U4 display-data contracts: engine-derived meshes/heightfields, per-view resolution, spatial diagnostic references, and section/mesh query ownership.
- Terminology for workpiece origin versus machine work offset.
