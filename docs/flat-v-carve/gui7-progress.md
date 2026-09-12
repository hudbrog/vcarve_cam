# GUI7 facing and ordered preparation

Status: GUI7a–GUI7d implemented and ready for manual review; user review
pending. Starting commit: `4be4b9759009d95d0ef710242c8086b3d1290189`.
Ending source: the uncommitted worktree on that commit; the plan reorder and
GUI6 review-reference edits were already present when GUI7 implementation began.

The workspace now orders up to twelve supported operations and faces stock
without an imported document. The new workflow adds a source-free **Face**
operation with explicit coverage, height and cutting values, generates the
selected prefix through the retained service, shows each operation's stock
context in the same timeline and simulator, and prepares checked output for
exactly the generated scope. All previous GUI2–GUI6 workflows remain available.

## Contracts and implementation

Operation ordering is a shared document command, not a GUI copy:
`cam_core::project::v5::commands::{add_operation, remove_operation,
set_operation_enabled, move_operation, rename_operation, duplicate_operation}`
(`crates/cam-core/src/project/v5/commands.rs`). The GUI's
`operation_authoring` module only binds an intent (which kind, under which
stable ID) to those commands, so both clients validate through the engine's own
structural gate and reference inspection. A newly created operation leaves
every machining value unset; only the job-tool snapshot and the canonical
height references exist.

`crates/cam-gui/src/scene.rs` replaces the GUI6 pair of scene adapters. One
builder now projects the executed prefix: artwork (filled components and knife
chains), per-stage motion ranges, cumulative stock checkpoints at every
operation and stage boundary, and the knife pivot/tip detail. The timeline
publishes one entry per executed stage (`report.gui2.groups`), and the
viewport's path visibility and "After …" checkpoints are derived from it.
Hiding an earlier operation's paths never rewinds the stock: the removal at the
current playhead is a property of the stock field, not of the drawn range.

Generation takes an explicit scope (`session::GenerateScope`:
`AllEnabled` or `ThroughOperation { operation_id }`). The retained service
binds that scope to the plan handle, so preparation and export cannot widen a
prefix: `PrepareOutput` re-plans nothing and rejects a document whose machining
identity changed. The Flat V-carve height panel can bind its top to a plane a
preceding Face operation published; reordering or disabling that face leaves a
saveable unresolved reference with a located issue
(`HEIGHT_REFERENCE_FORWARD` / `HEIGHT_REFERENCE_UNRESOLVED`) instead of
silently facing the stock top.

Face fields reuse the operation's own assignment IDs (cutting feed, plunge
feed, spindle speed, stepdown, stepover, endmill geometry) and add their own
coverage fields (`75…88` in `state::FIELDS`): pass angle, entry/exit overrun,
four margins, rectangle area, top/bottom offsets and the tool stepdown limit.
Empty text stays unset, partial text stays in recovery, and the planner reports
each missing value before planning.

## Evidence and acceptance audit

| GUI7 requirement | Evidence |
| --- | --- |
| Face job with stock/tool/cutting settings, generated paths, actual heightfield playback, checked export, reopen | `tests/face.rs::face_job_generates_playback_output_and_reopens`, browser tour `gui7-face-playback.png` / `sequence.ngc` |
| Source-free creation invents nothing | browser tour asserts the fresh document's stepdown/feed are absent; `crates/cam-core/tests/operation_edits.rs::created_operations_leave_machining_values_unset` |
| Rectangle/whole-stock coverage, margins, travel overrun, pass pattern and angle, rejection routing | `tests/face.rs::facing_coverage_controls_change_what_is_removed_and_where_travel_goes` and `::rejected_facing_settings_are_located_and_never_export` (FACE_ANGLE_UNSUPPORTED, FACE_STEPOVER_RANGE, missing-thickness issue with `field_path`) |
| Requested coverage distinguished from overhang | coverage rect from `inspection.face.covered`; removed volume exceeds the requested area only by the engaged overrun, asserted in the same test |
| Add/reorder/enable/rename/delete operations | `crates/cam-core/tests/operation_edits.rs`, `crates/cam-gui/tests/operation_lifecycle.rs`, browser tour "ordered face then carve" |
| Height dependency, prefix generation and one-program prefix export | `operation_lifecycle.rs::prefix_generation_and_export_bind_the_selected_scope` (prefix scope in the retained plan, whole list strictly larger), `face.rs::face_before_the_known_carving_publishes_per_operation_stock` |
| Reorder across a dependency, repair it | `face.rs` + browser tour `gui7-unresolved-dependency.png` (located issue, then repaired by reordering) |
| Per-operation stock context and unchanged knife stock | `face.rs::face_before_the_known_carving_publishes_per_operation_stock`, `::face_then_knife_keeps_the_knife_stock_intact`, browser tour `gui7-face-then-carve.png` / `gui7-face-then-knife.png` |
| Supported mixed milling/knife sequence and located contact rejection | `face.rs::face_then_knife_keeps_the_knife_stock_intact` (F3 `KNIFE_CONTACT_UNSUPPORTED` for a blade planted shallower than the faced material), browser tour `gui7-knife-contact-rejected.png` |
| Previous workflows preserved | full `cam-gui` suite, `cam-core`, `cam-service` suites; GUI3/GUI4/GUI5/GUI6 browser scenarios unchanged |

## Checks

- `cargo test -p cam-gui --locked`: 59 library tests plus the integration
  suites pass, including the new `tests/face.rs` (five tests) and the rewritten
  `tests/operation_lifecycle.rs` (three tests). Log:
  `artifacts/gui/gui7-tests.txt`.
- `cargo test -p cam-core -p cam-service --locked`: 42 suites pass, including
  the new `crates/cam-core/tests/operation_edits.rs` (three tests). Log:
  `artifacts/gui/gui7-core-tests.txt`.
- `cargo clippy -p cam-core -p cam-service -p cam-gui --all-targets --locked --
  -D warnings` and `cargo fmt --all -- --check` pass. Log:
  `artifacts/gui/gui7-clippy.txt`.
- Native release build: `artifacts/gui7/review/facing-native/cam-gui.exe`,
  SHA-256 `928def3fb1e7f8aa7aee113b6832e8eee5543c9f1ab0eb9ceef2518a2f7e094a`.
- Browser review package: `artifacts/gui7/review/browser` (wasm
  `ed7cb07d823c4d9e8a03d5785e35a2c044fc114d32a3bf189564b8154efec76e`, offline
  build `bef2879796f592b20d552755c62dffe68ca794c94785538ead3e34339c39302c`).
- Real Chromium/WebGPU tour `node crates/cam-gui/web/smoke.mjs --gui7`:
  Chrome 152.0.7977.83 on Windows x86_64 with an NVIDIA Ampere WebGPU adapter,
  zero console errors. Evidence:
  `artifacts/gui/browser-smoke/2026-09-12T18-29-27.090Z`, log
  `artifacts/gui/gui7-browser-smoke.txt`. The tour records 40 face motions, 208
  motions for Face → Flat V-carve and 132 for Face → drag knife; the exported
  `sequence.ngc` (SHA-256
  `3e45087a530a07ecf612e9944c0d51d2dda0e01bd14a6bfc93fb6fe7393ef0e8`) matches
  the displayed checked-output hash.
- `artifacts/gui7/review/manifest.json` records the source-tree hash, both
  package identities and the 199 source inputs used for this review build. This
  source tree is uncommitted, so the manifest identifies the actual files.

## Limits and review handoff

- **Coverage, not surface measurement.** Coverage controls decide what the
  planner faces; the simulator shows the modelled removal. Nothing here
  measures a real surface.
- **Approved pass angles only.** 0° and 90° raster facing ships; any other
  angle is rejected with a located reason.
- **Conservative mixed-sequence contact.** Knife contact over material an
  earlier operation removed is refused (`KNIFE_CONTACT_UNSUPPORTED`); only the
  supported Face → drag-knife fixture is exercised. Arbitrary prior
  carving/profile removal is not assumed supported.
- **Prefix export is one program.** Ordered sequential bundles remain GUI10.
- **Machine mapping is explicit.** A newly created operation's own tool
  snapshot has no controller mapping; preparation refuses the sequence with
  `MACHINE_MAPPING_MISSING` naming the tool until the user maps it in
  Job tools.
- **No physical claim.** No machine was controlled, and no blade-tracking or
  surface-quality claim is made beyond the modelled plan and the emitted bytes.
- **Native window tour.** The native package builds and every native
  integration test (including the real worker process) passes, but no scripted
  window interaction was performed for this slice; the trained native tour is
  part of the user review above.

Agent inspection during implementation found and fixed: a machine-profile
application that panicked on a source-free job (the machine panel now resolves
the selected operation's own tool), stock thickness reading as pending text for
Face operations, and the field routing of Face diagnostics. Those fixes are
covered by `app::inspector::face_ui::tests::a_configured_face_job_leaves_no_pending_text`
and `app::issues::tests::face_planner_fields_route_to_the_face_editor`.

Use [the review recipe](gui7-review.md) for launch instructions, fixtures,
exact settings and the review tour. GUI8 profiling and workholding is the next
dependent slice.
