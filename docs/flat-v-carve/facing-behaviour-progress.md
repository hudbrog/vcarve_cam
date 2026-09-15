# Facing behaviour — progress (2026-09-15)

Implementation note for [facing-behaviour-plan.md](facing-behaviour-plan.md),
which covers W1 and W3 of
[field-testing-fixes-plan.md](field-testing-fixes-plan.md) plus the entry
choice. Evidence: `flat-v-carve/artifacts/facing-entry/`.

## Slices

**S1 — a truthful motion chain, and one entry (cam-core). Landed.**
`face.rs` emits every transition as a motion: the end of a layer retracts,
travels to the chosen entry at the clearance plane and descends there, instead
of handing the next layer a `start` the tool has never been at. `pass_span`
resolves one entry per operation — `min`, `max`, an explicit position, or the
pre-2026-09-15 per-layer flip — and the entry travel applies only where the
cutter descends; a pass that continues its layer begins where the previous pass
ended. The allowed envelope is derived from the settings and the pattern, is
asymmetric when the entry is, and still refuses a path that leaves it
(`FACE_ENVELOPE_EXCEEDED`). `FACE_ENTRY_UNSAFE` names the end, the position the
cutter clears the stock from and the travel that reaches it; an explicit
position inside the pass's own span is refused with both allowed positions
(`FACE_ENTRY_INSIDE_COVERAGE`).

`checks.rs` gained `PLAN_MOTION_DISCONTINUITY` (a per-stage continuity
invariant) and now measures every descent from the position the machine is
actually at, so a plan cannot hide a descent behind a claimed start.

The new check immediately found the same defect in the **profile** planner: a
retract at the end of one contour, then a motion claiming a start 15 mm away at
the next contour's entry. `profile/mod.rs` now emits the travel at the
clearance plane inside a stage. The 12-stage flower-box batch, the knife jobs
and the lettering job are unaffected (0 discontinuities measured).

**S2 — the field, the help and the resolved numbers (document + GUI). Landed.**
`FaceSettings.entry` in the v5 document and its schema-4 mirror, the resolver
and the new-operation default (`min`); one new field id, `Face entry at`;
`(?)` help rewritten as definitions for the four margins, the two travel
values and the new field; the entry choice and the readout in the Face panel.
The readout is not a second implementation: `face_entry_preview` is the
planner's own resolution, exposed through
`v5::inspection::face_entry_preview`, and a test asserts the panel's promised
entries and coverage equal the plan's for all four modes.

Field labels were **not** renamed (the tester's decision), so `state.rs::FIELDS`
keeps its existing keys and only gains one.

**S3 — preview. Landed, with one trade-off left open.** The scene publishes
`gui2.facePlans` (requested area, coverage, envelope, pass span, entry
positions and whether each clears the stock) before a plan exists, and
`viewport_face.rs` draws **the coverage alone**: the tester found the extra
outlines and lines confusing — a closed travel box read as an area the tool
would sweep, and the entry line had no vocabulary attached — so those numbers
stay in the panel, where each has a field and a `(?)` definition. The camera
still fits
`stock ∪ artwork ∪ toolpath`; the plan's "fit to stock ∪ artwork" was not taken
because the rescaling defect it was meant to cure is already fixed and the
choice is a display-range trade-off for the tester.

**S4 — evidence. Landed.** `artifacts/facing-entry/` holds the two-layer
program (`face-two-layer.gcode`) and the plan report (`face-plan.json`) for a
Ø4 endmill that cannot plunge facing a 30 × 20 rectangle inside a 40 × 30
stock: 31 planned motions, 31 emitted blocks, two descents, both at `X-17.000`
with the cutter 15 mm clear of the stock, and the export's own readback passed.
The write is opt-in (`CAM_FACE_ENTRY_OUTPUT`), like the other measurement
artifacts in this repository.

**S5 — write-back. Landed.** `2.5d-cam-plan.md` §11.1 (entry choice, travel
semantics, envelope, the continuity invariant, the stock-is-the-material
assumption) and §11.2 (raster order), plus the W1/W3 status in the
field-testing plan. Both review packages are rebuilt
(`scripts/build-gui.ps1 -Target native` and `-Target web -NoOpt`): the Face
panel changed, so the tester's build has to be the one carrying the entry
choice.

## Tests

| Test | File |
| --- | --- |
| every motion starts where the previous one ended (2 patterns × 2 angles × 4 entry modes) | `crates/cam-core/tests/face_core.rs` |
| the basic checks refuse a chain that skips a position (`PLAN_MOTION_DISCONTINUITY`) | `crates/cam-core/tests/face_core.rs` |
| every descent happens at the chosen entry, one per layer for zig-zag | `crates/cam-core/tests/face_core.rs` |
| the entry travel belongs to the pass that descends | `crates/cam-core/tests/face_core.rs` |
| an explicit position equals the equivalent travel | `crates/cam-core/tests/face_core.rs` |
| an entry inside the pass span is refused with the allowed positions | `crates/cam-core/tests/face_core.rs` |
| one entry leaves the other end to the exit travel | `crates/cam-core/tests/face_core.rs` |
| the reported minimum clears; one reserve below refuses | `crates/cam-core/tests/face_core.rs` |
| a whole-stock face at zero overrun plans for a tool that cannot plunge (tangent side entry) | `crates/cam-core/tests/face_core.rs` |
| the program reproduces every planned motion, and passes readback | `crates/cam-core/tests/face_workflow.rs` |
| the readout matches the plan for all four entry modes | `crates/cam-gui/src/face_ui.rs` |
| every emitted motion stays inside the envelope the panel shows | `crates/cam-gui/src/face_ui.rs` |
| facing parameters never move the stock or the artwork | `crates/cam-gui/tests/scene_frame.rs` |
| the scene publishes request, coverage, envelope and entry | `crates/cam-gui/tests/scene_frame.rs` |

`cargo test` and `cargo clippy --all-targets` are clean in `flat-v-carve/`.

## Still open

* The camera-fit trade-off above.
* A job saved before the `entry` field existed has no value for it and
  therefore faces from the low end (`min`) rather than the old per-layer flip.
  §0 of the field-testing plan makes that acceptable: saved jobs are not
  preserved, and the default is the predictable one.
* Dragging the entry line in the viewport to set an explicit position (the
  mode is already reachable from the panel).
* Ramped entry along the row for a tool that can ramp but not plunge — the
  second answer for a face mill, listed as a follow-on in the plan.
* A machine-level warning when a job's entry clearance cannot be verified
  (the W2 half).
