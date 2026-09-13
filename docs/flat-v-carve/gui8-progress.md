# GUI8 profiles, workholding and finish controls

Status: GUI8a–GUI8d implemented and checked; ready for manual review. Starting
commit: `80e53a5` (the GUI7 worktree as committed). User review is pending; no
physical machining claim is made anywhere in this report.

GUI8 introduces the fourth first-release operation: a **closed milling
Profile**. A profile selects explicit closed contours of the imported artwork,
states which side of each contour keeps the material, and cuts them to an
explicit depth in ordered passes. Tabs, radial finishing and starts/entries are
added by the following letters on top of that same operation.

## GUI8a — basic closed profile

The workspace can now create a profile operation alone (`File → New profile job
from SVG`) or after the established carving (`+ Add operation → Add Profile`),
select the closed contours it cuts with an explicit inside/outside/on-contour
side per row, choose the endmill, heights and depth passes, generate, simulate
and export the checked program, then save and reopen the job.

### Contracts and implementation

Operation creation is a shared document command: `NewOperationKind::Profile`
appends one Profile operation with its own job-tool snapshot and canonical
zero-offset height references, and every machining value (contours, sides,
stepdown, direction, feeds, geometry) stays unset. The GUI's
`operation_authoring` module only binds the intent (kind plus stable ID) to
that command, so a new profile never looks configured.

`crates/cam-gui/src/profile.rs` is the editor adapter: it reads and writes the
canonical `ProfileSettingsV5` fields, lists the closed contours of the
displayed catalogue (owner-qualified wire ID, role, advisory side, perimeter,
bounds and vertices), and binds an explicit contour selection through
`cam_core::project::v5::commands::set_contour_selection`. Selection is sent
through the retained service, which re-imports the artwork there: a reference
from a replaced source is refused as a whole instead of being rebound.

The scene publishes the closed-contour catalogue once per scene
(`report.gui2.profileContours`) so the editor and the viewport never import an
SVG in the frame function, exactly like the knife chains. Contour clicks in
**Select profile contours** mode assign the clicked contour with the
importer's advisory side (Shift-click adds/removes); the operation's own table
is where the side is chosen explicitly. The contour checks, side buttons and
side per row are keyed by the contour's qualified identity, never by row
position.

Cancelling, stale results and scope rules are the GUI7 ones: the profile is one
more entry of the same ordered operation list, generated through the same
retained service, displayed by the same timeline (`Profile rough` /
`After profile rough`) and simulator, and exported by the same
through-operation prefix preparation.

`crates/cam-core/src/project/v5/inspection.rs` gained
`inspect_profile_fields`, so the generation pre-check, the located issue list
and the editor all use the planner's own required-field list; the profile's
field paths route to the profile controls (contours, stepdown, direction,
assignment, heights, tabs, finishing and entries).

### Evidence and acceptance audit

| GUI8a requirement | Evidence |
| --- | --- |
| Create a profile alone or after the known carving | `crates/cam-gui/tests/profile.rs::profile_job_generates_playback_output_and_reopens` (new profile job from SVG), `::profile_after_the_known_carving_keeps_each_operation_stock` (carving then profile), `crates/cam-gui/tests/operation_lifecycle.rs` covers the added kind |
| Qualified contour table with explicit sides | `app::inspector::profile_ui::tests::profile_editor_owns_its_contour_table_and_cut_fields` (per-contour keys, three sides, no carving mode), `crates/cam-gui/tests/profile.rs` selects two outers and one hole on their advisory sides |
| Stale or unknown contour references are refused, not rebound | `::an_incomplete_profile_reports_its_own_missing_fields`, `crate::profile::select_in` re-validates against the catalogue on the service side |
| Offset/direction/height inspection | `profile_ui::profile_evidence` reports the stored sides, chosen direction, the cutter radius as the nominal standoff and the plan's resolved top/bottom heights and per-stage motion counts; `::profile_job_generates_playback_output_and_reopens` asserts the resolved heights (0 → −6 mm) come from the plan |
| Simulate the cut against real stock | Same test: the stock prefix reports zero removal at the start and real removal at the end, through the existing heightfield |
| Export the checked program and reopen | Same test: `Prepare` returns a program with checked bytes, and the saved document reopens with its selection, sides and direction |
| Rejection cases stay located | `::profile_side_depth_and_through_allowance_change_the_cut` (`PROFILE_THROUGH_ALLOWANCE`), `::an_incomplete_profile_reports_its_own_missing_fields` (planner required fields), `app::issues::tests::profile_planner_fields_route_to_the_profile_editor` |
| Previous workflows preserved | Full `cam-gui`, `cam-core` and `cam-service` suites; the GUI3–GUI7 fixtures and tours are unchanged |

### Checks

- `cargo test -p cam-gui --locked`: 62 library tests plus every integration
  suite pass, including the new `tests/profile.rs` (four tests). Log:
  `artifacts/gui/gui8a-tests.txt`.
- `cargo test -p cam-core -p cam-service --locked`: every suite passes. Log:
  `artifacts/gui/gui8a-core-tests.txt`.
- `cargo clippy -p cam-core -p cam-service -p cam-gui --all-targets --locked --
  -D warnings` and `cargo fmt --all -- --check` pass. Logs:
  `artifacts/gui/gui8a-clippy.txt`, `artifacts/gui/gui8a-fmt.txt`.

### Limits

- **Offset is shown, not measured.** The editor reports the sides, the chosen
  direction, the cutter radius and the plan's resolved heights; the drawn
  centerline in the viewport is the generated compensated path. Nothing here
  measures a real edge.
- **One manual anchor per contour.** Automatic tab placement (GUI8b) covers
  several tabs around one contour; manual anchors are addressed by their
  contour.
- **Rectangular tabs only.** Ramped shoulders remain diagnosed, never
  silently changed.
- **No physical claim.** No machine was controlled; the acceptance tour is
  software and simulation review.

## GUI8b — tabs

The profile can now leave visible material bridges in the same cutting
workflow: automatic placement by count or spacing, or manual anchors that are
added, moved (numerically and by dragging in the viewport) and removed, with
the generated bridge drawn from the plan's own footprint.

### Contracts and implementation

`cam_core::contours::Contour::anchor_ring()` publishes the closed ring in the
vertex order anchor fractions address: the setup-space ring rotated so its
first vertex is the same physical vertex as the placement-independent source
ring's canonical start. Uniform placement scaling preserves arc-length
fractions, so an editor that walks this ring reproduces the planner's own
`resolve_anchor` exactly — the UI can therefore offer a numeric and drag
equivalent for a tab position without re-deriving the parameterization.
`crates/cam-core/tests/contour_catalogue.rs::the_anchor_ring_starts_at_the_fraction_zero_vertex`
pins that contract for identity and rotated/scaled placements.

The scene publishes each closed contour's `anchor_ring` alongside its vertices,
so `crates/cam-gui/src/profile.rs` can compute a candidate position
(`ring_point`) and the inverse (`project_ring`) without importing an SVG in the
frame function. `crates/cam-gui/src/viewport_profile.rs` draws two clearly
different things:

* **candidate markers** (yellow squares) at the document's own requested
  anchors, and
* **generated bridges** (green quads) from the retained plan's
  `tabPlacements[].footprintMm`, which the planner publishes as the exact
  protected cross-section, independent of the display grid.

Dragging a marker captures the anchor's contour scope and the document
revision at press time, projects the pointer onto that ring while dragging,
and emits exactly one move on release. `App::ui` commits that as one undo
transaction (`remember`, then `Document::edit_anchor`) and drops the long raw
token so the field shows the document's own formatting. Escape or a document
revision change cancels the gesture; the camera-yaw gesture no longer competes
with an anchor drag.

Manual-anchor authoring is a service command
(`ArtworkCommand::ProfileTabAnchor` → `profile::tab_anchor` →
`cam_core::project::v5::commands::set_contour_selection`/`reattach_anchor`), so
the catalogue import that binds a new anchor's fingerprint happens in the
worker, never in the frame function. Each anchor's numeric row is scoped to
*its own contour* (`state::Draft::key_scoped`): the draft key is
`{contour wire ID}/{operation}/Tab anchor fraction`, so reselecting, reordering
or removing one row can never move another row's text. Partial text stays
attached to that anchor and blocks Generate/Save; a removed anchor's text is
dropped from the store without ever blocking a save.

The tabs group states the display limit where the user configures it: the
heightfield's cell size is shown, together with the fact that a tab narrower
than one cell may not appear in the raster while the drawn bridge boundary is
exact. `cam-service` now advertises `profileTabs` (rectangular tabs with
automatic or anchored placement); ramped shoulders stay unadvertised and the
planner still reports them.

### Evidence and acceptance audit

| GUI8b requirement | Evidence |
| --- | --- |
| Add/move/remove tabs numerically | `crates/cam-gui/tests/profile.rs::tabs_hold_material_and_follow_their_manual_anchors` (add on a contour, move by fraction, remove, and the generated bridge follows), `app::inspector::profile_ui::tests::tab_controls_state_their_placement_mode_and_anchor_rows`, `::an_anchor_draft_stays_with_its_own_contour` |
| …and by dragging | `viewport_profile.rs` drag gesture (candidate → projected ring position → one move event → one undo transaction); the candidate marker is drawn at the same position the numeric row states |
| Retained material in the actual cutting workflow | `::tabs_hold_material_and_follow_their_manual_anchors`: the same job with tabs removes less material than without, and every placement publishes its four-corner footprint |
| Scrub rough and final passes | The timeline's `Profile rough` / `After profile rough` entries and the stock checkpoints are the GUI7 ones; the generated bridge is drawn from the plan at every playhead |
| Export and reopen | `::profile_job_generates_playback_output_and_reopens` and the tab test's `exportReady` assertions; profiles save and reopen with their tabs, anchors and placement mode |
| Source anchors versus silent repair | `::a_replaced_source_leaves_tab_anchors_unresolved_until_reattached`: replacing the source leaves the anchor unresolved with `ARTWORK_REVISION_MISMATCH` at `tabs.placement.anchors[0]`, generation stays blocked, an explicit reattach resolves it, and re-binding the contour selection is a separate explicit step |
| Candidate versus generated geometry | Candidate markers and generated bridges are separate overlay layers; a candidate is never drawn as if it were cut, and the generated quads come from the plan |
| Visible sub-cell display limits | The tabs group reports the display cell size and the raster limitation; the exact generated boundary is drawn as an overlay regardless of the grid |
| Single-gesture Undo | The drag handler pushes exactly one history entry per released gesture and none for a cancelled or failed gesture |

### Checks

- `cargo test -p cam-gui --locked`: 64 library tests plus every integration
  suite pass, including the six `tests/profile.rs` tests. Log:
  `artifacts/gui/gui8b-tests.txt`.
- `cargo test -p cam-core -p cam-service --locked`: every suite passes. Log:
  `artifacts/gui/gui8b-core-tests.txt`.
- `cargo clippy ... -D warnings` and `cargo fmt --all -- --check` pass. Logs:
  `artifacts/gui/gui8b-clippy.txt`, `artifacts/gui/gui8b-fmt.txt`.

### Limits

- **Automatic placement is feasibility-checked.** A requested count that does
  not fit a contour's straight spans is refused with
  `PROFILE_TAB_NO_SPACE` naming the contour; the editor does not shrink tabs or
  drop some of them silently. A tab anchored where the compensated path has no
  room is refused with the same located reason.
- **One manual anchor per contour.** Several tabs around one contour come from
  automatic placement; a second manual anchor on the same contour is refused.
- **Rectangular tabs only.** Ramped shoulders remain diagnosed.
- **Drag is a display gesture.** The released position is committed as a source
  fraction; the cut still comes from the planner's resolved placement.

## GUI8c — finishing

The profile can now leave a radial allowance and run a configured finishing
pass: the rough passes stand off the wall by the allowance, the finishing pass
cuts the finished wall with its own feed, both passes keep using the
operation's one tool assignment, and the timeline states the passes in
execution order so rough and finished stock can be compared.

### Contracts and implementation

The planner already owned this: `ProfileFinishSettingsV5 { enabled,
radial_allowance_mm, feed_mm_min }`, rough loops at `radius + allowance`,
finishing loops at `radius` with the finishing feed, one stage per
`ProfileRough`/`ProfileFinish` run, and `PROFILE_FINISH_RANGE` /
`PROFILE_FINISH_ALLOWANCE` for a bad allowance or a zero-offset selection. The
slice adds the editor group (**Radial finishing** → allowance, feed, and the
pass-order explanation), keeps `profile::set_finish_enabled` unsetting the new
values so the planner reports exactly what is missing, and reports both feeds
and the allowance in the evidence readout. No path is constructed in the UI.

One real display consequence had to be fixed: a profile with finishing emits
one stage boundary per contour per pass, and the display checkpoint builder
counted its evenly spaced frames *after* fitting the budget, so a job that
needed more frames than the display budget held was refused outright with a
byte message. `stock_preview::build_with_marks` now reserves the initial and
final frames, keeps as many stage boundaries as the budget allows (newest
first), and reports the number it could not keep in
`PreviewMeta::dropped_stage_marks`. The Simulate panel states that limit
explicitly next to the display cell size, and the timeline can still seek every
stage boundary by replaying forward from the nearest earlier checkpoint
(section 10.4). Toolpaths, checks and output are unaffected: only display
checkpoints are dropped.

The timeline's repeated role labels were also corrected while adding the second
pass: a role that occurs more than once is numbered
(`Profile rough paths (1 of 3)`, `After profile finish (1 of 3)`) in execution
order, instead of renaming later entries after the operation. Single-stage
operations keep the exact labels GUI2–GUI7 published.

### Evidence and acceptance audit

| GUI8c requirement | Evidence |
| --- | --- |
| Leave a radial allowance and run the configured finish pass | `crates/cam-gui/tests/profile.rs::radial_finishing_adds_a_finish_pass_at_the_allowance_and_feed`: both stage roles generate, the first rough checkpoint removes less than the finished state, and the emitted program carries `F300` (roughing) then `F150` (finishing) |
| Per-pass inspection | The same test reads `inspection.stages` (rough and finish motion counts), the per-stage timeline groups, and the stock frames after the first rough pass and at the end; the Result inspection panel's pin/compare uses the same stage positions |
| Assignment preservation | The same test asserts feeds, spindle speed, stepdown limit and tool are unchanged after enabling finishing and setting the allowance, and that only the finishing feed is added |
| Regenerate with tabs still present | `::finishing_keeps_the_tabs_and_their_anchors`: a manual tab and its anchor survive enabling finishing and regeneration, the placement count stays one, the anchor fraction is unchanged, and the finished job removes at least the same material |
| Rejections stay located and routed | `PROFILE_FINISH_ALLOWANCE` for a positive allowance on an on-contour selection (same test), `PROFILE_FINISH_RANGE` from the planner, and `app::issues::tests::profile_planner_fields_route_to_the_profile_editor` covers the finish field paths |
| No UI-side path construction | The editor writes only `finish.*`; the offsets, stage order, feeds and any rejection come from the retained plan |
| Display limits stay visible | `stock_preview::tests::more_stage_boundaries_than_the_budget_holds_drop_the_oldest_marks` (it no longer fails a usable job; the drop count is reported) and the Simulate panel's explicit note |

### Checks

- `cargo test -p cam-gui --locked`: 65 library tests plus every integration
  suite pass, including the eight `tests/profile.rs` tests. Log:
  `artifacts/gui/gui8c-tests.txt`.
- `cargo test -p cam-core -p cam-service --locked`: every suite passes. Log:
  `artifacts/gui/gui8c-core-tests.txt`.
- `cargo clippy ... -D warnings` and `cargo fmt --all -- --check` pass. Logs:
  `artifacts/gui/gui8c-clippy.txt`, `artifacts/gui/gui8c-fmt.txt`.

### Limits

- **One finishing pass per contour.** The planner emits a single finishing
  loop at the finished wall after each contour's rough loops; local pass/depth
  selection inside a stage remains GUI9 work.
- **Allowance is radial only.** It is a wall allowance; floor and depth
  behaviour are the same as the rough passes.
- **Display checkpoints are bounded.** A job whose stage count exceeds the
  display budget keeps the newest boundaries and states the number of dropped
  ones; the cut itself is never truncated.

## GUI8d — starts and entries

The profile can now control where and how the real cut begins and ends: an
anchored start (numeric or dragged), a plunge or ramp entry, and supported
tangent line/arc leads on the lead-in and lead-out — with an invalid entry
provoked, explained where it lives and repaired.

### Contracts and implementation

`crates/cam-gui/src/profile_ui.rs::profile_starts` states the start (automatic
source seam or one selected contour at a fraction, with the same presets and
the same draggable marker the tabs use), the entry (plunge or ramp with its own
angle and feed) and each lead (none, tangent line, tangent arc) with the values
that mode actually uses. The group opens by default because it carries the
cut's entry, not an optional extra.

Every choice is a document value or a service command:
`cam_core::project::{ProfileEntry, LeadSpec}` and
`cam_core::project::v5::{StartSelectionV5, ContourAnchorV5}`.
`ArtworkCommand::ProfileStart` chooses the start against the current catalogue,
`ArtworkCommand::ProfileAnchorReattach` reattaches an unresolved start or tab
anchor to an explicitly picked contour, and the ramp/lead values are ordinary
operation fields. `profile::set_entry` and `profile::set_lead` never invent
values: choosing a mode leaves its parameters unset so the planner reports
exactly what is missing, while a caller that has resolved values (a recipe or a
test) supplies them and they are kept.

Nothing is resolved in the UI. The planner repositions the seam on the
compensated loop, wraps an overshooting ramp onto the loop, checks each lead's
whole cutter sweep against every selected contour's retained side and against
the tab bridges, and reports a located reason for anything that does not fit:
`PROFILE_ENTRY_CAPABILITY`, `PROFILE_RAMP_NO_SPACE`, `PROFILE_LEAD_SIDE`,
`PROFILE_LEAD_NO_SPACE`, plus the ramp-capability requirement as a located
missing field. `app::issues` routes those field paths to the profile controls,
and `the ramp-capable tool` control lives in the same tool group.

### Evidence and acceptance audit

| GUI8d requirement | Evidence |
| --- | --- |
| Move a start (drag and numeric) | `crates/cam-gui/tests/profile.rs::moving_the_start_and_choosing_the_entry_change_the_real_motion`: after anchoring the start at one contour's half-source position, a pass enters within one cutter standoff of that point, while the automatic job enters only at its own seams; the viewport drag writes the same field (`App::ui` → `Document::edit_anchor`) and the group offers preset positions plus the exact numeric row |
| Choose a supported entry and see it in the motion | The same test: the plunge job descends with no XY travel, the ramp job descends while travelling, and a tangent line lead-in adds a 4 mm approach and more motions than the same job without it |
| Typed candidate commands and source-revision binding | The start/reattach/tab authoring commands are typed `ArtworkCommand`s executed by the retained service; an anchor keeps its stored fingerprint, so a changed source needs an explicit reattach (see GUI8b's replaced-source test) |
| Provoke and repair an invalid entry | `::an_invalid_entry_is_located_and_can_be_repaired`: an unset ramp capability is a located missing field, `Some(false)` is `PROFILE_ENTRY_CAPABILITY`, switching to a plunge repairs it; a tangent arc on an on-contour selection is `PROFILE_LEAD_SIDE` and removing the arc repairs it |
| Lead and tab constraints | The planner's lead/tab overlap checks (`PROFILE_LEAD_NO_SPACE`, `PROFILE_RAMP_NO_SPACE`) and the retained-side check are exercised by the same tests and by `::tabs_hold_material_and_follow_their_manual_anchors`; a lead or ramp that cannot fit a small hole is refused with the reason naming the contour |
| Every drag has a numeric/list equivalent | The start and tab rows expose their fraction numerically, the add menus offer 0/25/50/75% presets, and `app::inspector::profile_ui::tests::start_entry_and_lead_controls_state_their_supported_choices` pins the control names |

### Checks

- `cargo test -p cam-gui --locked`: 66 library tests plus every integration
  suite pass, including the ten `tests/profile.rs` tests. Log:
  `artifacts/gui/gui8d-tests.txt`.
- `cargo test -p cam-core -p cam-service --locked`: every suite passes. Log:
  `artifacts/gui/gui8d-core-tests.txt`.
- `cargo clippy ... -D warnings` and `cargo fmt --all -- --check` pass. Logs:
  `artifacts/gui/gui8d-clippy.txt`, `artifacts/gui/gui8d-fmt.txt`.

### Limits

- **One start per operation, not per contour.** The anchor positions the
  selected contour's seam; other contours keep their automatic seams.
- **Two lead shapes.** Tangent line and tangent arc ship; they are checked
  against retained material and tab bridges, so a lead that cannot fit is
  refused rather than trimmed.
- **Ramp needs a capable tool and room.** The capability is the user's
  declaration; the fit is the planner's check on the contour's own loop.
- **No physical claim.** Entries are modelled and emitted, not measured on a
  machine.

## Manual review

The review build, the starting fixtures, the exact settings and the tour for
each letter are in [the GUI8 review recipe](gui8-review.md). Technical status
is "ready for manual review"; user acceptance is pending and is not implied by
these tests.

### Review builds

- Native release: `artifacts/gui8/review/profile-native/cam-gui.exe`, SHA-256
  `8ba832268730cfe5de285408d2cbbb64020ec1605ada9196beb7dcf3b5c063b7`
  (recorded in its `SHA256SUMS`).
- Browser package: `artifacts/gui8/review/browser` (WASM SHA-256
  `d57c25d25e6c71df92ae8e6fd62ee13275816b4ec6c9a4ddfe26a1f1bb9d21be`), served
  by `artifacts/gui8/review/serve-package.mjs`; offline bundle
  `9c5d18c9f1477783a27a506fa9da3ce3c3b16ee5d7c9826d9e2a262feb474623`.
  Both binaries were rebuilt from the committed source on 2026-09-13: the
  first packages predated the final formatting pass over `app.rs` and
  `profile_ui.rs`, so their hashes no longer identified the built source.
  Rebuilding did not change the machining result: the tour's prepared program
  is the same `065bbdc3…` hash.
- `artifacts/gui8/review/manifest.json` records the per-file SHA-256 of every
  source input it was built from (287 files, including these review documents,
  so editing a document shifts that tree hash), the repository `head`, and both
  package identities. The packages were built from exactly the committed
  source content at `ac29c4b`; only the review documents changed afterwards.

### Implementing agent's end-to-end browser run

`node crates/cam-gui/web/smoke.mjs --gui8` against the packaged review build,
real Chromium 152.0.7977.83 on Windows x86_64 with an NVIDIA Ampere WebGPU
adapter, zero console errors. Log: `artifacts/gui/gui8-browser-smoke.txt`;
evidence directory
`artifacts/gui/browser-smoke/2026-09-13T05-41-58.389Z` (and the earlier
`2026-09-12T22-14-52.830Z` run) with screenshots of the
new job, contour selection, generated profile, rough stock, the refused tab,
the tabbed result, the finishing stock, the ramp requirement, the ramp entry,
the prepared output, the reopened job and the pending-text state.

The tour exercised: new profile job from SVG with nothing invented (894
motions generated), the rough stock checkpoint, a located tab rejection on the
small hole (then repaired by removing the hole from the selection, 860
motions), radial finishing with its own pass and checkpoint (1828 motions), an
anchored start at 50% with the ramp-capability requirement reported as one
setting needing attention and then satisfied (1864 motions), prepared checked
output (`065bbdc3e061570e363361cf960cff6d3d14c3bbc8bc28aaecb5857650014c74`)
whose downloaded bytes match that hash, and a portable reopen that keeps the
contours, tabs, finishing, start anchor and ramp entry.

The browser tour found a real defect that the native suites could not: the
application's asynchronous reply handler did not recognize the four new
profile command kinds, so a profile selection (or anchor edit) answered by the
worker was reported as "Unknown GUI2 worker response" and never reached the
document. `App::accept` now adopts them, and
`app::inspector::profile_ui::tests::a_profile_selection_reply_is_adopted_not_reported_as_unknown`
pins that path. Failed tasks also report their code and message
(`HEIGHT_RANGE_INVALID: …`) instead of a raw diagnostic JSON.

Native status for the same slice: the release package builds and every native
integration test passes, including the real worker process, the canonical
carving parity test and the eight profile tests. No scripted native window
interaction was performed for this review; the trained native tour in the
recipe above is part of the user review.

## Manual review feedback (2026-09-13)

**Finding (user, first review pass).** "In *Tool & cutting* in the profile
operation I can't seem to select a tool from my existing tool library; instead
it has 'endmill' and 'endmill-2', which I actually don't have in the library."

The profile panel offered only the job's own tool snapshots — a combobox over
`job.tools` — so a job-tool copy created by the operation could be picked, but
nothing could be brought in from the loaded tool library, and there was no
Applied/Modified/Reset state for the profile's cutter. The same gap existed in
the library modal for a Profile (or Face) document: the "use this tool" action
only offered the carving rough/finish roles and a knife role.

**Fix (second review pass: one picker for every operation).** The Flat V-carve
tool tabs' cutter element was extracted into
`crates/cam-gui/src/tool_picker.rs` and every operation now renders that same
element: the job tool in use, **Change tool…** revealing the library tool and
cutting-profile pickers plus the job's own snapshots (**Assigned job tool**,
**Clear … cutting values**), then the copied profile line with **Change
profile…** and **More… → Reset … overrides / Reapply reviewed profile**. Only
the words differ per assignment; the layout, commands and
Applied/Modified/Custom meaning are one implementation:

- `Cutter::endmill`/`Cutter::vbit` (Flat V-carve's two stages),
  `Cutter::milling(op, "Face")` and `Cutter::milling(op, "Profile")` (each
  operation's single assignment) and `Cutter::knife` (drag knife) all call
  `App::tool_picker`. The panel, the reveal toggle and the library combos are
  role-slot keyed, so a Face, Profile and Knife editor each keep their own
  selection, and every picker targets its **own operation** (the carving tabs
  previously assumed the first operation when opening the library picker).
- Clearing values, using a job tool, applying a library tool or cutting
  profile, resetting and reapplying all go through role-aware document
  commands: `R::Clear` (new) → `resources::clear_assignment_values`,
  `R::UseTool` → `resources::use_job_tool`, `R::SelectLibraryTool`,
  `R::ApplyToolProfile`, `R::Reset`, `R::Reapply`. A library tool's rotation is
  copied with the new role-aware `resources::set_assignment_spindle_direction`
  (previously a Flat V-carve-only code path that failed for `Milling` with
  "Unsupported milling assignment").
- `use_job_tool` now accepts a snapshot that carries **no** geometry for any
  role (an incomplete cutter is a state the planner reports, not a mismatch),
  while a wrong-kind snapshot is still refused.
- The library modal addresses the **selected** operation (not the first one)
  and offers `Milling` for a Face/Profile document, so "Use tool & profile"
  works there too.
- `cam_core::project::v5::resources::set_assignment_spindle_direction` is a new
  role-aware command: copying a library tool's rotation onto the addressed
  assignment used to be a Flat V-carve-only code path that failed for
  `Milling` ("Unsupported milling assignment").
- `ResourceCommand::clear_fields` now clears the profile's assignment drafts
  (cutting/plunge feed, spindle speed, tool stepdown limit, cutter geometry)
  after a tool or profile change, so no stale raw text survives the copy.

**Consequences a reviewer will see (all honest, none invented):**

- Applying a library tool copies its geometry into the job as an independent
  snapshot (`endmill-2` in the tour) with copied provenance; the job keeps no
  live dependency on the library.
- Changing the cutter clears the assignment's cutting values, its baseline and
  its rotation. If the library tool declares a rotation it is copied; if it
  does not, the rotation is unset again and the planner reports it.
- The copied cutter needs its own controller mapping before checked output
  (`POST_TOOL_MAPPING` names the tool, and the T number must be unique in the
  applied configuration).

**Evidence.** `crates/cam-gui/tests/profile.rs::a_profile_takes_its_cutter_and_cutting_values_from_the_tool_library`
applies a library tool + cutting profile to a profile that cannot plan before
it, asserts the copied feeds/speed/stepdown/rotation and geometry, the Applied
→ Modified → Reset cycle, and that the job then generates and prepares;
`crates/cam-core/tests/collection_resources.rs::a_library_tools_rotation_is_copied_onto_the_addressed_milling_assignment`
pins the role-aware rotation copy (profile accepted, sibling carve untouched,
knife refused); `app::inspector::profile_ui::tests::the_tool_group_offers_the_job_tool_and_library_actions`
pins the panel, and
`app::inspector::tool_picker::tests::every_operation_renders_the_same_picker`
pins that the Endmill, V-bit, Face, Profile and Knife editors all render the
same picker element (and
`::the_cutters_keep_the_established_probe_names` keeps the probes the earlier
tours address). The browser tour now loads the library fixture, takes the
cutter and cutting profile through the profile's own **Change tool…** picker,
maps it, and prepares the checked program; the knife (GUI6) and carving (GUI5)
tours were re-run against the same build and still pass, so one picker did not
change the reviewed workflows. Evidence directories:
`artifacts/gui/browser-smoke/<timestamp>` from the runs recorded in
`artifacts/gui/gui8-browser-smoke.txt`, `gui6-browser-smoke.txt` and
`gui5-browser-smoke.txt`.

**Rebuilt review artifacts** (the fix changes code, so both were rebuilt):
native `91b7611862f30c484f956a0f62fa57a62a5c8b055d4a99f4d642f4b2036587db`,
WASM `307642617e3202fda261e7fa28d208dec9c4403f195f1f40da238936e587c3f3`,
offline bundle `594c4b41b639cf217c786569799e0bb4edaabdae5dc477806f2f539eb077a90b`.

The **Face** operation now uses the same picker too, so profiles, facing, the
carving stages and the knife all behave identically.
