# Web UI: product and interaction design

Date: 2026-09-05\
Status: product design baseline; U1/U2, bounded U3 planning/2D inspection, U5 M5/M6 integration, and the local tool library are implemented.\
Confirmed direction: a persistent CAM workspace with guided setup steps.

This design expands M7 into a complete product experience. Read the [integration and delivery plan](integration-plan.md) for current implementation status, remaining service work, and acceptance scenarios, and the [M5 verification report](u5-verification.md) for the latest checked slice. Features described below remain product targets unless the implementation reports mark them delivered.

## 1. Product intent

Help a CNC-router operator turn SVG artwork into an inspectable, reproducible combined endmill/V-bit job and, when verification and machine output are available, a LinuxCNC program. The user specifies the desired shape and actual setup; the planner coordinates the two tools.

The interface should answer five questions continuously:

1. What artwork and physical dimensions am I machining?
2. What finished shape do these settings request?
3. What will each tool remove, and what remains?
4. What has actually been checked, and where are the unresolved problems?
5. Does the output correspond to this job and this machine profile?

Primary use is a single operator on a local desktop or laptop, with mouse and keyboard. The planning baseline remains bundled TypeScript browser assets served by a local Rust application. Everyday OS and supported browsers still need confirmation. Tablet layouts should support inspection and editing; phone layouts are for occasional review and recovery, not the primary CAM workspace.

## 2. Scope of a fully featured first release

Included: portable jobs; SVG inspection and region selection; physical placement; stock/depth/tool/quality setup; endmill-only and combined planning; target and stock visualization; motion inspection; spatial diagnostics; verification review; machine-profile setup; combined/per-tool export; local recovery; undo/redo; keyboard access; reproducible reports.

Local reusable tool, setup, and machine presets are proposed conveniences. Applying one copies a snapshot into the job, previews the changed values, and invalidates affected output. Editing a preset must never silently alter existing jobs. No feed or spindle recommendations are generated. Imported jobs retain unset machining values until the user supplies them or explicitly applies a preset.

The [local tool library](tool-library-ui.md) implements tool definitions, optional
cutting presets, persistence, browser management, job capture, and reviewed,
undoable snapshot application through the Rust store. Setup and machine preset
libraries remain proposed.

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

A plan opens as an inspection artifact with its embedded job. “Edit as job” creates an editable draft and retains the original artifact. Users can inspect partial plans and download diagnostic artifacts even when machine output is unavailable.

### 4.2 Artwork

Show the imported physical page dimensions, artwork bounds, source filename, and selected-region count. Let the user inspect actual normalized geometry before machining setup. Raw SVG must not be injected into the page as active markup; use normalized data or an inert rendered preview.

Selection works both on the canvas and through an accessible source/component list. Display SVG labels when present and stable component IDs in details. Support click selection, additive selection, Select all, Clear selection, and Fit selection. Keep visibility toggles separate from machining inclusion. Preserved holes belong to the selected filled region; they must not appear selected for removal just because their parent is selected.

Keep separate notions of an inspected item and the set of regions included for machining. A click on a toolpath or issue should not change machining selection. Include explicit controls for inclusion so selection changes are visible and undoable.

Unsupported SVG features produce a list with the source element when available and a concrete remedy, such as converting text/strokes to paths in Inkscape. Do not silently drop unsupported artwork. An invalid replacement import retains the current job. “Replace artwork” previews changed components; do not assume old IDs identify the same shapes after reimport. Require review of the new inclusion set.

### 4.3 Stock & origin

Expose stock thickness, artwork scale, linked physical width/height, rotation, and XY origin. Show stock top as Z = 0 and machining depth as positive downward in forms; display machine Z as negative in motion inspection. Name page bounds, artwork bounds, and any future stock bounds distinctly.

Origin controls offer page/artwork anchor presets and an explicit source-page XY point. Display the resulting workpiece axes immediately. Preserve the core convention `workpiece XY = scale × rotate(page XY − origin)` with page Y upward. “Move origin” and “move artwork” must not be ambiguous drag modes. Arbitrary extra translation, independent X/Y scaling, and stock X/Y size are not current job fields and must not be invented in serialization.

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
| Advanced computation | Endmill layer/loop/motion limits; V-bit path/motion/subdivision/depth-pass/cleanup limits; quality-sample spacing/count; reachability-cell and stock-slice limits. Preserve all current fields even when the section is collapsed. |

Use units beside every numeric input. Preserve the user's text while they type; an empty field is not zero. Unset capability is “Not specified,” distinct from Yes and No. Show missing fields as a completion checklist; invalid supplied values receive inline errors. Rust owns the authoritative validation and supported ranges. The UI may check syntax and finite numbers immediately, but it must not duplicate geometric feasibility rules.

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

2D top view with physical axes is required first. A 3D orbit view, stock surface, and arbitrary section query are full-release features with new display-data contracts. Do not present existing debug SVGs as if they already provide them. Navigation includes fit job/selection, pan, zoom, top/isometric presets, and reset camera. Preserve camera and layer choices across settings edits where useful.

Playback animates recorded motions; it never simulates new cuts in client geometry code. A scrubber can use motion index or normalized progress before credible timing exists. Any future duration estimate must state its modeled scope and exclude unmodeled tool-change/probing behavior. A low-resolution mesh is labeled Visual preview; changing display resolution must not alter a verification result.

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

The detail table includes code, severity, stage, affected region/motion when known, measured value or interval, requested limit, and the next useful action. Distinguish sample maxima from global bounds. M4 numerical depth budget and XY coverage tolerance are shown in evidence details; do not label a sampled residual maximum “maximum error.” A missing location is shown as job-level, never as a fabricated map marker.

Keep geometry findings visually distinct: endmill paths use blue; V-bit paths teal; missed reachable stock pink with hatch; possible overcut purple with crosshatch; cutter-limited detail amber with a labeled boundary; unresolved results use a neutral stipple and explicit label. Selection uses a separate outline. Legend toggles update the actual drawing, and issue labels make the meaning independent of color.

### 4.7 Export

Machine setup contains the LinuxCNC profile snapshot, work offset, tool-number mapping, clearance policy, tool-length-compensation ownership, M6 contract, and output precision when M6 supplies them. Current `m6_contract` is editable prose, not validated macro behavior. Do not offer an “accept” checkbox as a substitute for the missing backend contract.

The export screen summarizes current job/plan/profile identity and required checks, then offers a combined program or independent per-tool programs. Preview generated program text alongside the tool sequence and formatted-motion findings. The service generates and validates the exact bytes before making files downloadable.

Offer a job snapshot, plan/report, and setup summary alongside machine output. Make Download job/report distinct from Export machine program, so blocked machine output does not prevent saving or debugging. Per-tool output must be self-contained according to the M6 contract. The UI does not send programs to LinuxCNC or expose a Run machine control.

Before M5/M6 exist, keep this step visible with specific unavailable capabilities and preserve editable profile data. No illustrative prototype state may suggest that current M4 can export a checked machine program.

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

## 7. Design review and remaining decisions

Confirmed: workspace with guided steps. Proposed: six-step organization, neutral technical appearance, separate inspection/selection concepts, local presets, recovery, and full-release 3D/cross-section inspection.

Resolve before UI implementation: primary everyday host and browser; preferred information density on the user's actual display; initial 2D/3D delivery priority; scope of local tool/preset management; terminology for workpiece origin versus machine work offset. These do not block the current design work.

Resolve with M5/M6: explicit verification scope/bounds, machine-profile schema, compensation/M6 semantics, formatted-output report, and authoritative export eligibility. These require Rust and machine-contract evidence rather than visual design choices.

The first interactive concept demonstrates the layout, step navigation, view modes, layer controls, and a stale-result transition using labeled illustrative data. It is a design review aid, not an implementation or evidence of a valid toolpath. Follow-up design should cover import rejection, incomplete setup, finite-tip limitations, inconclusive verification, recovery, and export preview as well as the normal job.

Concept validation: the local Edge browser rendered 1024, 736, 360, and 320 px layouts in both light and dark themes without horizontal overflow or JavaScript errors. Interaction checks covered all six steps, view/layer changes, cross-sections, edits marking the sample stale, reverting edits, and keeping machine output unavailable. Desktop and narrow screenshots were visually inspected. This validates the layout study only; production service integration, accessibility auditing, and supported-browser qualification remain future work.

## 8. Evidence used

This plan is grounded in the current working tree, which is changing during M4 work. It does not update the Rust milestone status.

- [Architecture](../architecture.md): product boundaries, local browser baseline, Rust ownership, portability, and machine boundary.
- [Implementation plan](../implementation-plan.md): M5 verification, M6 machine output, M7 workflow acceptance.
- [Technical design](../technical-design.md): coordinates, tolerance meanings, artifacts, output requirements.
- [Current job schema](../../../flat-v-carve/crates/cam-core/src/job.rs): nullable setup, schema 3, two tool slots, current machine-profile fields.
- [SVG model](../../../flat-v-carve/crates/cam-core/src/svg/mod.rs): source/component mapping, placement, bounds, and importer limits.
- [Endmill settings](../../../flat-v-carve/crates/cam-core/src/pocket/settings.rs), [V-bit settings](../../../flat-v-carve/crates/cam-core/src/vcarve/settings.rs), and [combined quality report](../../../flat-v-carve/crates/cam-core/src/vcarve/quality.rs): actual controls and current evidence scope.
