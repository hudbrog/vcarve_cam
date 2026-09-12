# GUI8 profiles, workholding and finish controls

Status: GUI8a implemented and checked; GUI8b–GUI8d in progress. Starting
commit: `80e53a5` (the GUI7 worktree as committed). User review is pending for
every letter; no physical machining claim is made anywhere in this report.

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

Not started.

## GUI8c — finishing

Not started.

## GUI8d — starts and entries

Not started.
