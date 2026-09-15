# Field-testing fixes — plan

Source: the "CAM Module — Field Testing Report" received 2026-09-13 (drag knife
on cardboard, Inkscape SVG import, stock resizing after import, facing, Flat Z
engraving, general project workflow and machine settings).

This document is a workstream layer on top of the existing plan set. It does
not replace them:

* parameter semantics, operation contracts and milestones stay owned by
  [2.5d-cam-plan.md](2.5d-cam-plan.md) and
  [2.5d-cam-checklist.md](2.5d-cam-checklist.md) — the decisions below must be
  written back there when they change;
* coordinate and data contracts stay owned by
  [technical-design.md](technical-design.md);
* GUI slices follow the [2.5d-cam-ui-implementation-plan.md](2.5d-cam-ui-implementation-plan.md)
  review-package convention (`scripts/build-gui.ps1`, a `gui*-progress.md`,
  a review recipe).

Priorities are the report's own: **P0** = may damage tool, stock, machine or
cutting mat; **P1** = correctness; **P2** = workflow.

---

## 0. Compatibility policy (2026-09-14)

The project is pre-release and does not preserve saved jobs. Backward
compatibility is therefore **not** an input to any decision in this document:
schema versions, field names, defaults, the import mapping and the meaning of
an existing field may all change when the change is the better design.
Compatibility is not an acceptable reason to keep a mode, a default, a field
or a migration path alive.

What this does *not* license:

* removing the schema-4 planning substrate (`project::CamJob`). It is not a
  compatibility layer: the planners, the contour catalogue and post all run on
  it, and `plan_v5` converts to it internally. Collapsing it is a refactor to
  justify on its own merits, not a deletion.
* renaming GUI field labels without updating the tests in the same commit
  (`state.rs::FIELDS` doubles as the probe contract). That is a test contract,
  not compatibility.
* fabricating geometry. A stroke stays a centreline rather than an outline
  because of caps, joins, miters and dashes — that is mathematics, not history.

Decisions that were previously deferred or bent for compatibility are
re-opened and are being taken now:

* **W4:** `ImportMode` is deleted, not defaulted. The importer publishes every
  reading the file supports and the operation chooses.
* **W1:** the exemption that let profile and pocket entries plunge with an
  undeclared capability is removed; the schema-4 model's explicit
  `plunge_capable` contract returns everywhere.
* **W5:** the stock-resize anchor can become a document field with a default
  chosen on merit rather than "what the fields always did", and the import
  mapping may be changed if a better one is found. Both are follow-ups recorded
  here; the landed behaviour is unchanged until they are taken.
* **Open question 9** (new): the schema-3 `job::Job` document exists only for
  the `cam` CLI. Re-pointing the CLI at schema 5 would leave exactly one
  document model behind the GUI.

---

## 1. Provenance and the reproduction rule

Every finding is treated as a claim about a specific binary, not about `main`.
Step 0 of every workstream is therefore the same: reproduce it against the
tested build, then against `main`, and record the difference.

Open question **Q1** asks which build was tested. The most likely candidate is
the GUI9 review package (`flat-v-carve/artifacts/gui9/review/`,
`docs/flat-v-carve/gui9-review.md`), which is also the newest user-facing
binary (`large-job-native/cam-gui.exe`). The plan assumes that until the tester
confirms; the engine version reported by the shipped CLI is `0.7.7`.

Two findings are already implemented in `main` (§3, items 3.1-adjacent and
6.2). They stay in the plan as *findability and verification* items rather
than as new code.

### 1.1 Reproduction baseline captured while writing this plan

Three minimal fixtures were run through the shipped CLI
(`flat-v-carve/target/release/cam.exe`, matching `main`):

| Fixture | Command | Observed |
|---|---|---|
| Inkscape 1.4-style circle, inline `style="fill:none;stroke:#000000;stroke-width:1"`, A4 mm page | `cam import a.svg --output a.job.json` | `SVG_STROKE: visible strokes must be converted with Inkscape Stroke to Path` |
| Same circle after Stroke to Path, styled through a `<style>` block with `class="cls-1"` | `cam import b.svg --output b.job.json` | `SVG_STYLESHEET: CSS stylesheets are unsupported; use presentation attributes or inline styles` |
| 100×100 page, filled circle `A 10,10` tangent to page `y = 0` | `cam import c.svg` then `cam inspect` | selected-region bounds are `y = 80.000 … 100.000`; page top maps to setup maximum Y |

So report §3.1 is reproduced exactly, and §3.2 has a concrete, testable rule
rather than a guess: the importer *does* flip Y (see W5), the convention is
simply undocumented, unasserted, and not surfaced anywhere the user can see it.

---

## 2. Architecture spine (report §9)

Report §9 is right that several findings share one root: there is no written
coordinate contract, so every surface re-derives it. Workstream W5 fixes this
for SVG and stock; W3 fixes it for facing parameters; W1 needs it to state
where "outside the stock" is.

The contract to write down (and test) is four named spaces with one conversion
each:

| Space | Definition | Converted where |
|---|---|---|
| SVG/document | `viewBox` + page size in mm, Y down | once, in `svg/mod.rs` (`page_matrix`, `viewport`) |
| Artwork/model | page mm, Y up, origin at the page's bottom-left | once, at import, through `Placement` |
| Stock/work | setup mm, origin at `setup.work_zero`-relative stock coordinates | stock rectangle + operation geometry |
| Machine | output coordinates after the work offset and the machine profile | `post/sequence.rs` only |

Invariants to enforce with tests, not prose:

1. Import converts once. Nothing downstream re-imports, re-flips or re-scales.
2. Changing stock dimensions or any operation parameter never re-places
   already imported artwork.
3. Preview, verification and post consume the same transformed geometry. A
   check that reads the emitted block list back and compares it with the plan
   motions already exists for export; extend it to the facing/entry question
   (§W1) and to parking (§W9).
4. Artwork or selected geometry outside the stock rectangle is reported as an
   issue with the offending items named, never silently.

---

## 3. Status of every reported item against `main`

| # | Finding | Prio | Status in `main` | Anchor |
|---|---|---|---|---|
| 1.1 | Facing plunges inside material | P0 | **Present**; reproduced by inspection | `cam-core/src/operations/face.rs:527`, `:584` |
| 1.2 | Z reference not prominent | P0/P1 | **Present** | `cam-gui/src/inspector.rs:512-513` |
| 2.1 | Offset/overhang semantics unclear | P1 | **Present**; parameters exist, semantics undocumented | `cam-core/src/project.rs:512-544`, `face.rs:219-256` |
| 2.2 | Facing offsets distort the preview | P1 | **Reproduced and fixed** for the artwork/stock scale; camera fit still open | `cam-gui/src/scene.rs::build_with_preset`, `executed_frame` |
| 3.1 | Inkscape CSS/`<style>` rejected | P1 | **Fixed** (W4): `<style>` cascaded, ignored properties warned, no interpretation mode — every reading is published and the operation picks | `svg/style.rs`, `svg/mod.rs` |
| 3.2 | SVG Y transform wrong | P1 | **Documented and asserted** (W5): page top → stock max Y, one conversion, golden A4 test | `svg::page_to_artwork`, `technical-design.md` §1/§4 |
| 3.3 | Stock resize breaks relative position | P1 | **Fixed** (W5): named resize anchor (min corner default), *Stock from artwork bounds*, outside-stock issue naming items | `project/v5/commands.rs`, `cam-gui/src/inspector.rs` |
| 4.1 | Flat Z cuts thin features too high | P1 | **Root cause identified** | `vcarve/settings.rs:196-205` |
| 5.1 | Drag knife motion discontinuous | P1 | **Root cause identified** | `post/sequence.rs:497-500`, `:946-953` |
| 6.1 | No plain New project | P2 | **Present** | `cam-gui/src/workspace_ui.rs:130-158` |
| 6.2 | Imported artwork cannot be deleted | P2 | **Already implemented**; verify + findability | `cam-gui/src/inspector.rs:353`, `session.rs:458` |
| 7.1 | Machine-level tool/Z reference | P2 | **Absent** | `project/v5/machine.rs`, `post/sequence.rs:47-62` |
| 7.2 | Parking position after program end | P2 | **Absent** | `post/sequence.rs:955` |
| — | Facing simulation performance | P1 (report §8.9) | **No measurement in the report**; needs a number | — |

Progress against this plan lives at the end of each workstream below as
`Status:`. Landed work has its own tests and does not need re-derivation.

Two facts matter for the P0 items and are the reason W1 is cheap to make safe:

* tools already carry `capabilities.plunge_capable` / `ramp_capable`
  (`project.rs:275-283`, `JobToolV5`), and the GUI already edits them
  (`resource_ui.rs`); and
* the v5 sequence already has `MotionPurpose`/`MotionKind::{Approach, Entry,
  Plunge, Ramp}` vocabulary, but `checks.rs::check_plan_v5` never consults
  capability — it only checks ids, ranges, ownership and completeness
  (`checks.rs:88-180`).

In other words the model knows about plunge capability and nothing enforces
it.

---

## 4. Workstreams

### W1 — Facing: guarantee that no depth change happens over material (P0, 1.1, half of 2.1)

**Current behaviour.** `raster_rows` builds every row with
`cross_min - radius - entry_overrun` at one end and
`cross_max + radius + exit_overrun` at the other (`face.rs:219-256`). On
zig-zag passes the reversed rows therefore *enter* at the exit-overrun end, and
when the footprint there is not wholly outside the stock
(`footprint_wholly_outside_stock`, `face.rs:279`) the entry branch descends
from `heights.top_z.max(cut_z)` to the cutting depth at plunge feed
(`face.rs:527-575`, and the first-motion branch at `:584-625`). With the
exit overrun at `0` the footprint touches the stock edge, "not wholly outside"
is true, and the tool plunges at the stock boundary. That is exactly the
reported crash for a face mill.

**Fix.**
1. Make the two overruns directional: an overrun applies to the *pass
   entry/exit* side of the row as executed, not to the `cross_min`/`cross_max`
   side of the coverage rectangle. State in the plan doc whether the
   measurement is to the cutter centre or the swept edge, and by extension how
   it combines with the coverage margins (see Q3).
2. Replace the "descend from stock top" fallback with a hard invariant: a
   motion may lower Z below the resolved stock top only when
   `footprint_wholly_outside_stock` is true *at that position*, or when the
   tool is marked `plunge_capable`/`ramp_capable` and the entry is a real
   plunge/ramp within the tool's stepdown.
3. If neither holds, return an incomplete plan with a located issue
   (`FACE_ENTRY_UNSAFE`) naming the coverage, the overrun and the tool — never
   a silent plunge.
4. Add the same invariant to `checks.rs::check_plan_v5` as a *basic check*, so
   no future planner, operation or hand-edited plan can emit an unsafe entry.
   This is the part that makes the fix durable rather than operation-local.
5. Route the tool's `plunge_capable` into the face planner's `missing_fields`
   so an unset capability is reported like every other required value.

**Tests.** `cam-core/tests/face_core.rs` (asymmetric and zero overruns, both
pass angles, both patterns, plunge-capable and not); a new
`face_entry_safety` fixture asserting no emitted block descends below stock top
while the footprint overlaps stock; a post-readback test asserting the emitted
`G1`/`G0` sequence equals the planned motions (report: "the generated G-code
must also match the toolpath shown in the preview").

**Acceptance.** With a non-plunge-capable tool and overruns `0`, facing either
enters wholly outside the stock or refuses with a named reason. No G-code block
descends into material.

**Status: landed.** `raster_rows` now returns unoriented rows, `pass_span`
applies the entry overrun to the side a pass starts from and the exit overrun
to the side it leaves, and the allowed envelope reserves the larger of the two
at both ends. The planner refuses an entry that reaches material when the tool
is not marked `plunge_capable = true` (`FACE_ENTRY_UNSAFE` when it is marked
false, with the entry point and the clearance that would clear the stock;
`MISSING_MACHINING_SETTING` when the answer was never given). Tangent contact
counts as clear: a cutter that only touches the stock boundary has zero overlap
with the material, so the default whole-stock face at zero overrun still plans
with no extra declaration. `checks.rs::check_plan`/`check_plan_v5` enforce the
same rule over any plan (`PLAN_ENTRY_UNVERIFIED`); a rapid descent into
material is never authorized. The Face panel now carries the cutter's plunge
capability next to the tool geometry, and its field path routes back there from
the issue list.

**Re-opened and closed 2026-09-15.** Measuring the entry rule showed that the
plan it reads was not whole: the motion chain skipped a position at every
depth-layer boundary, and because the post emits one block per motion endpoint
the machine executed that gap as a straight `G0` from the clearance plane down
to the next layer's cut depth — below stock top over the stock by the full
1.997 mm depth of a plain whole-stock face at zero overrun with 10 rows per
layer, while 11 rows per layer transited at the tangent plane and cut nothing,
so the safety of the program depended on a row-count parity. The plunge side
also alternated with the layer, so a tool that could not plunge needed
clearance at both ends of the coverage.

Landed: every transition is an explicit motion (retract → travel at the
clearance plane → descend), `PLAN_MOTION_DISCONTINUITY` is a basic check, and
the descent rule is measured from the position the machine is actually at. A
new `entry` field (`min` / `max` / `at` / `alternate`) makes the entry the
user's choice, `at` taking an explicit position on the pass axis, and
`FACE_ENTRY_UNSAFE` now names the end, the position the cutter clears the stock
from and the travel that reaches it. The new check immediately found the same
defect between contours in the **profile** planner (a retract, then a motion
claiming a start 15 mm away), which is fixed the same way. Evidence and tests:
[facing-behaviour-plan.md](facing-behaviour-plan.md),
`flat-v-carve/artifacts/facing-entry/`, `crates/cam-core/tests/face_core.rs`,
`crates/cam-core/tests/face_workflow.rs`.

**Follow-up (unblocked by the compatibility policy, §0).** Profile and pocket
entries may still plunge with an *undeclared* capability: the plan's own
planners do not yet require the declaration there. This used to be a
compatibility question — requiring it would have stopped jobs that never
declared the value — and is now simply unfinished work: the schema-4 model
required `plunge_capable` explicitly and verified it, and requiring it again
for profile and pocket entries restores that contract.

### W2 — Z datum safety and machine tool reference (P0/P1, 1.2 + 7.1)

**Current behaviour.** The active Z datum is two small selectable labels inside
the "Physical stock" panel (`inspector.rs:512-513`), with a one-line caption at
`:528`. Work zero affects output coordinates only ("simulation stays in setup
coordinates"), so a wrong datum is invisible until the machine moves. Nothing
in the machine configuration records how the machine measures tools.

**Fix.**
1. Make the datum a first-class, always-visible control: a header/banner in
   Setup and on the preparation/export page stating the resolved Z of stock
   top and stock bottom in both setup and output coordinates, plus the physical
   consequence ("this is the surface the tool will touch at Z0").
2. Extend the applied machine configuration with an explicit tool-measurement
   reference, e.g. `tool_touch_off_reference: table | stock_top | fixture`
   (+ optional offset), resolved through
   `project/v5/machine.rs::resolve_sequence_profile` like every other
   preparation field: missing = reported, conflicting = a named conflict, never
   a silent default.
3. Cross-check: if the datum and the machine reference imply different physical
   surfaces, raise a warning at generate time and a **gate** before checked
   export (report 1.2 asks for exactly this).
4. Keep the datum per job (`setup.work_zero.z`) — it is a property of this
   stock, not of the machine — and let the machine reference supply the
   *default suggestion* and the conflict check. Confirm with the tester (Q2).

**Acceptance.** A mismatched datum produces a warning before generation and a
named, actionable block before export; the datum is visible without scrolling
to the middle of the stock panel.

**Status: partly landed, on the tester's instruction.** Open question 2 was
answered as "either way is fine — just document it in the (?) section in the
UI", so the machine-level tool-reference field (7.1) is deferred and the work
here is prominence plus documentation:

* Setup now has its own `Z datum` control with a help icon, labelled
  `Z0: stock top` / `Z0: stock bottom` instead of sharing a row with the XY
  choice, and it states the resolved consequence in the machine's terms
  ("Z0 is the bottom surface of the stock: every output Z is 3.000 mm below the
  top surface the tool first touches.").
* The help entry explains what each surface means, that every output Z is
  measured from it, that simulation stays in setup coordinates, that CAM cannot
  see how the machine touches off tools after M6, and that choosing the wrong
  surface is what cuts through the spoilboard or cutting mat.

Still open from this workstream: a warning/export gate driven by a machine
reference — that needs the field deferred above, so it is not in this batch.

### W3 — Facing parameters, labels and preview (P1, 2.1 + 2.2)

**Current behaviour.** `FaceSettings` has `area`, four `margins`
(`project.rs:512-544`) and `entry_overrun_mm`/`exit_overrun_mm`; the GUI labels
them "Coverage margins", "Travel overrun", "Face entry overrun"/"Face exit
overrun" (field ids 76-77, 78-81; `face_ui.rs`, `state.rs::FIELDS`). Their
interaction is described only by two small captions. On the preview side the
scene bounds are `stock ∪ artwork ∪ toolpath` (`scene.rs:272-300`, `:483-487`),
so enlarging an overrun moves the camera and reads as the stock shrinking or
the artwork shifting.

**Fix.**
1. Write the definition down in `2.5d-cam-plan.md` §facing: coverage =
   area + margins; overhang = cutter *centre* travel beyond the coverage
   boundary, applied per pass entry/exit; state explicitly whether overhang is
   additive to the margins in any case and whether any endpoint is measured to
   the cutter edge (the report itself is unsure — see the parenthetical in its
   §2.1 example).
2. Rename the GUI labels to match the definition and add the resolved numbers
   ("entry 60 mm → tool centre starts at Y = -60") next to the fields. Field
   labels are probe keys used by the GUI tests, so rename code and tests in one
   commit.
3. Preview: draw the requested coverage, the allowed sweep envelope and the
   safe entry/exit zones as three distinct things, and keep the stock rectangle
   exactly `setup.stock.xy` (already true in `scene.rs`). Make the camera fit
   stable — fit to stock ∪ artwork, and show toolpath extent as an explicit
   overlay rather than as an implicit zoom change.
4. Add an issue when coverage/overhang extends beyond the stock and the tool is
   not plunge-capable (the physical "you are cutting air at the back of the
   machine" case).

**Tests.** A facing scene test asserting stock dimensions and artwork position
are byte-identical before/after changing each of the four margins and both
overruns; a preview test asserting coverage/envelope/entry overlays are drawn
for the report's example (Y stock `0…100`, facing offset `20`, start overhang
`60`, Ø51 tool → entry at `Y = -60`).

**Status: the scale half of 2.2 landed; the rest is open.** The tester's own
reproduction (`real_data/facing_job.json`: 200 × 100 × 18 mm stock, its page-sized
`flower_box.svg`, a whole-stock face with a Ø50.2 plate at 25 mm stepover) was
generated and the first raster row read back: the motions run to `x = 225.1`
and `x = -25.1`, so the frame grew from 200 mm to 252.2 mm across. The artwork
had already been normalized against the 200 mm frame, so the SVG was drawn
**26% larger than the stock block** — the report's "the loaded SVG is rendered
larger than the stock material", including why it looked intermittent: at pass
angle 90 the raster leaves through the 100 mm axis, the frame only grew to
202 mm, and the error fell under 1%.

The scene now settles **one display frame before the first vertex is written**
(`build_with_preset` + `executed_frame`): the stock rectangle, the artwork, the
knife chains and the executed toolpath are all normalized against it, so a path
that travels outside the stock widens the frame instead of rescaling anything
drawn inside it. `crates/cam-gui/tests/scene_frame.rs` reproduces the reported
job at both pass angles, reads the contour vertices back out of the payload and
asserts the artwork returns as the 200 × 100 mm rectangle it occupies — while
the toolpath, read back from the same payload, is asserted to leave the stock.
Before the fix the same test recovers `-26.1 … 226.1` for the artwork at
0 degrees.

What this does **not** change, and what is still open from W3, is the size of
the travel itself and how the view frames it: the raster still puts the cutter
centre a full radius past the coverage at both ends of every row (25.1 mm with
this plate) with both overruns at `0`, and the camera still fits
`stock ∪ artwork ∪ toolpath`, so a large overrun still reads as the whole scene
zooming out. The definition question is Q3-adjacent — whether the default
whole-stock face should carry an implicit cutter-radius overrun (which is what
currently keeps the tangent entry of a non-plunging tool legal, W1) or ask for
it as a stated entry/exit overrun — and the overlay/camera half of step 3 above
is still unstarted.

Two more preview defects were reported directly by the tester and fixed in the
same batch, both recorded in [preview-display-progress.md](preview-display-progress.md):
the preview camera is now a free orbit (drag to rotate, middle/Shift drag to
pan, wheel to zoom toward the pointer, Top/Isometric/Front/View/Zoom/Fit) with
one shared frame for the renderer, the picker and every overlay; and the
simulated stock's four walls now follow the material left in each boundary cell
instead of standing at the original stock height, so a faced plate reads as a
plate rather than a tray with the original envelope around it.

**Landed 2026-09-15, with W1.** The parameter vocabulary is written into
`2.5d-cam-plan.md` §11.1 and §11.2; every facing `(?)` entry now defines its
number (coverage = area + margins; travel = cutter-centre travel past the
coverage, applied where a pass enters and where it leaves; the envelope is
coverage + travel + radius; the stock rectangle is the material), with the
field labels deliberately left as they were. The panel states the coverage, the
sweep envelope, the entry and the clearance at it before Generate, offers the
one-click travel or position that would clear the stock, and carries the entry
choice (both ends, an explicit position, or the per-layer flip). The viewport
draws the coverage and nothing else: the requested area, the pass span, the
travel and the entry are stated with numbers in the panel, because the tester
found the additional outlines and lines in the viewport read as areas and had
to be decoded rather than read (2026-09-15; `viewport_face.rs`,
`gui2.facePlans`). A scene test asserts that changing any
facing parameter moves neither the stock rectangle nor the artwork.

Still open, deliberately: the camera still fits `stock ∪ artwork ∪ toolpath`
rather than the plan's "fit to stock ∪ artwork with the travel as an overlay".
The rescaling defect is fixed (the frame is settled before the first vertex),
and the travel is now drawn as its own overlay, so this is a display-range
trade-off — showing the travel or keeping the stock's scale — that the tester
should settle rather than one this batch took silently. An explicit-position
drag in the viewport (the entry line as a handle) is a natural follow-up.

### W4 — Inkscape/SVG import compatibility (P1, 3.1)

**Current behaviour.** A `<style>` element is a hard failure
(`svg/mod.rs:291`); an inline declaration outside the fixed allow-list is a
hard failure (`svg/style.rs:83-101`); a stroke without a fill is a hard failure
in fill mode (`svg/mod.rs:733-751`, message at `:746-750`) — accepted only in
the knife centerline mode. That is the report's exact 7-step sequence.

**Fix.**
1. Support `<style>`: parse rules for element, `.class`, `#id` selectors,
   apply them with CSS specificity and `!important`, in the same inheritance
   walk that already handles inline `style` and presentation attributes.
2. Demote unknown/irrelevant non-geometric properties to **warnings** (they
   already have a `diagnostics` channel and a warning severity at
   `svg/mod.rs:930-937`). Properties that change geometry must stay hard
   failures: `filter`, `mask`, `clip-path`, markers, `transform-origin`,
   references, external stylesheets.
3. Failures must name the element id and the element's `inkscape:label`, and
   say what to do — the report shows a user editing raw XML to guess.
4. Accept a visible stroke in fill mode as a *centerline with a reported
   diagnostic* when the job asks for engraving, or keep rejecting it but offer
   the one-click "import as centerline" path in the same message; decide with
   the tester (Q4 covers the related depth question).

**Fixtures.** Add `flat-v-carve/fixtures/fieldtest/` with: the Inkscape 1.4
circle (inline style), the same after Stroke to Path (CSS `<style>` + class),
an Inkscape layer group with `enable-background`, a named-colour file, and the
two failing fixtures from §1.1 verbatim.

**Acceptance.** The report's scenario imports with no XML editing, and each
ignored property is listed as a warning in the import report.

**Status: landed, then superseded by the mode-free importer (§0).** `<style>`
elements are parsed once and cascaded with the
inline `style` attribute and the presentation attributes by `!important`,
specificity (element / `.class` / `#id`) and source order, so the report's
Stroke-to-Path file imports with no XML editing and its `fill-rule` reaches the
geometry (the fixture is asserted to be a ring, not a disc). A CSS property
outside the supported subset — including Inkscape's own editor properties — is
now a warning (`SVG_STYLE_IGNORED`) naming the element and the property, never a
silent drop; a property that changes the drawn geometry (`filter`, `mask`,
`clip-path`, markers, `transform-origin`, the CSS `transform` property) is
refused, as is a rule whose selector this subset cannot match
(`SVG_STYLE_SELECTOR`). External stylesheets stay hard failures
(`SVG_STYLESHEET`): `<?xml-stylesheet?>` and `@import`, since the importer has
no file or network access. Every element-scoped diagnostic now names the
element's `id` and its `inkscape:label`. The fixtures are
`flat-v-carve/fixtures/fieldtest/` with their own README, and
`crates/cam-core/tests/svg_fieldtest.rs` pins each acceptance line.

Step 4 was first landed as "keep rejecting, name both remedies". §0 then
removed the reason to keep a mode at all, so the refusal is gone: the importer
has no `ImportMode`, publishes every reading a drawing supports (see §4 of
`technical-design.md`), and the operation picks the one it needs. A filled
shape therefore offers its own outline to a knife, a closed stroked outline is
selectable by a profile, and an element carrying both a fill and a stroke is
both an area and a line instead of an error telling the user to separate them.
What stays a refusal is anything the importer cannot describe honestly: text,
references, masks, clip paths, filters, external stylesheets and the CSS
`transform` property. What was an error and is now a diagnostic: an open
subpath carrying a fill (closed for filling, `SVG_OPEN_PATH`), a stroke read as
a centreline (`SVG_STROKE_CENTERLINE`, naming the width it discarded), a
too-short subpath (`SVG_DEGENERATE_PATH`), a gradient fill
(`SVG_PAINT_OPACITY`) and an element that draws nothing (`SVG_NO_PAINT`).
Progress note: [fieldtest-import-readings-progress.md](fieldtest-import-readings-progress.md).

**Slice B of the same decision (identity).** Because two shapes that differ
only in colour or layer cut identically, neither is ever used to decide
geometry — but they are how the drawing is recognised. Each catalogue entry
now carries the element's `id`, its `inkscape:label`, its nearest named layer
and its resolved colour; the pickers name rows from them (`Flower`, not
`carve::0`) with a colour swatch, the viewport draws excluded artwork in the
colour the drawing used, and each picker offers "select all in <layer>" and one
swatch per colour. The pickers also say what each reading is for, so "no filled
components to carve" now reads as "nothing in the drawing is filled: strokes
and the outlines of filled shapes are knife and profile geometry". One
consequence left to decide: *Create knife outlines* is now redundant, because a
filled shape already offers its own outline as a knife path.

### W5 — Artwork and stock coordinates (P1, 3.2 + 3.3, architecture spine)

**Current behaviour.** Import flips Y about the *physical page height*
(`svg/mod.rs:315`, `:548-601`), so the page's top edge maps to the stock's
maximum-Y edge; this is a legitimate CNC convention but appears nowhere in the
docs or the UI. Stock resize
(`project/v5/commands.rs:805::apply_stock_rectangle`) writes an absolute
rectangle: `min_x/min_y` stay and width/length change, so shrinking a
page-sized stock leaves the artwork "floating in space" exactly as reported.

**Fix.**
1. Document the mapping (page top → stock max Y; page bottom-left → setup
   origin at the default placement) in `technical-design.md` and in the
   Artwork panel caption (`inspector.rs:415`).
2. Add a golden test on an A4/mm fixture asserting the exact setup bounds of a
   shape tangent to the page's top-left corner (the fixture from §1.1 pins the
   current answer: `y = 80…100` on a 100 mm page).
3. Give stock resize explicit anchor semantics: a named anchor
   (min corner / centre / artwork bounds) shown in the UI, defaulting to
   today's min-corner behaviour so no saved job changes meaning. Add "Stock
   from artwork bounds" (the artwork analogue of `Stock XY from SVG page`,
   `inspector.rs:437`, `resources.rs:364`).
4. Report artwork outside the stock as an issue, naming the items, and offer
   the anchor change that fixes it. Never move artwork as a side effect.

**Acceptance.** Resizing the stock to 100×100 or 110×110 leaves every imported
coordinate unchanged; the user is told when geometry now lies outside the
stock.

**Status: landed.** The mapping is written down where the contract lives:
`technical-design.md` §1 now names the four XY spaces and their single
conversions, and §4 states the mapping in user terms. `svg::page_to_artwork`
(`y_artwork = page_height − y_svg`) is the one expression of the flip, and the
Artwork panel caption states it: the page's top edge is the stock's maximum Y,
the page's bottom-left corner is the setup origin, and placement is the second,
independent step. A golden fixture
(`fixtures/fieldtest/a4-corner-square.svg`, A4 in mm, shapes pinned to both
page corners) asserts the exact setup bounds in
`crates/cam-core/tests/svg_coordinates.rs`, including the plan's own 100 mm
example (`y = 80…100`).

Stock resize now carries an explicit anchor, offered in Setup as *Stock resize
anchor* with help: **Min corner** (the default — chosen because it was the
existing behaviour at the time, not because saved jobs demanded it; §0 re-opens
the default), **Stock centre**, and **Artwork bounds**.
`commands::anchor_stock_rectangle` owns the geometry and is a pure
function of the requested rectangle, the previous rectangle and the placed
artwork bounds; an explicit min-corner edit is taken as written, and only the
stock rectangle moves — artwork placement is never touched by any anchor. The
anchor is workspace state rather than document state today; §0 makes a document
field with a better default a legitimate follow-up, since no saved job has to
keep its meaning. *Stock from artwork bounds* is the artwork
analogue of *Stock XY from SVG page* (all items, zero margins, through the
existing fit-stock proposal).

Geometry outside the stock is reported, never relocated:
`commands::artwork_outside_stock` / `stock_overlap_issues` name each offending
item and how far past which side it reaches (`STOCK_ARTWORK_OUTSIDE`, path
`setup.stock.xy`), tangent contact counts as inside, and the Setup panel shows
the same names next to the stock numbers with the two fixes beside them. Tests:
`stock_resize_anchors_move_the_rectangle_and_never_the_artwork` and
`artwork_outside_the_stock_is_named_in_an_issue_that_offers_the_fix` in
`crates/cam-core/tests/artwork_commands.rs`, plus the Setup-panel probes in
`crates/cam-gui/src/inspector.rs`.

Leave the acceptance question (open question 6: which default anchor) with the
plan: the default is deliberately today's min-corner behaviour, and the tester
sees and can change it in the same panel.

### W6 — Flat Z engraving on thin features (P1, 4.1)

**Root cause.** A V-bit's reachable depth at a sample is
`(signed_distance − tip_radius − guard) / tan(half-angle)`, clamped to the
depth cap (`vcarve/settings.rs:196-205`). For a stem narrower than the tip
radius the available clearance is ≤ 0 and the depth becomes exactly `0`, so the
tool cuts in the air — while the endmill stage, which is driven by the flat
floor target, looks correct in the same operation. That matches the report's
observation that only thin single-line features are wrong.

**Fix (needs one semantics decision, Q4).** For features narrower than the
width where the V-bit tip fits, choose one and make it explicit in the plan and
the preview:

* *engrave a centerline at the requested flat depth* (single pass, material
  removed wider than drawn — what the tester appears to expect), or
* *keep the tapered rule but never below a minimum contact depth*, or
* *refuse the feature* and report it by name as unreachable.

Whichever is chosen must hold for the whole family (medial/centerline spokes,
retained, rest) so the "different Z-calculation path" the report noticed
becomes one documented rule. The final-finish expectations in
`vcarve/verify.rs:199-267` and the quality sample checks
(`quality.rs`) must be updated in the same commit, and the depth must remain
bounded by `target.depth_cap()`.

**Tests.** A synthetic needle/stem fixture at flat depth 1 mm and 2 mm: assert
the emitted Z reaches the requested depth (or that the feature is reported as
unreachable), plus a stock-simulation assertion that material is removed along
the line. Re-run the accepted flower job
(`real_data/flower_box-svg.job-real.json`) at 1 mm and 2 mm and diff the motion
counts.

### W7 — Drag knife motion quality (P1, 5.1)

**Root causes (both present).**

* Knife stages are forced to `PathControl::ExactPath`, i.e. `G61` exact stop,
  regardless of the profile (`post/sequence.rs:497-500`), so the machine
  decelerates to a stop at every vertex — "even with blend enabled".
* Every flattened polyline vertex becomes one `G1` (`post/sequence.rs:946-953`);
  nothing in the knife planner simplifies or fits arcs — `drag_knife/mod.rs`
  only cleans degenerate segments.

**Fix, in this order.**
1. *Measure*: motion count, program length, and the number of full stops for
   the existing knife fixtures (`real_data/knife*`, `artifacts/knife-flower-repro`).
   Record it as the baseline evidence artifact.
2. Allow the profile's path control for knife stages under a knife-specific,
   **verified** blend tolerance, using the existing knife replay/evidence
   machinery (`drag_knife/evidence.rs`, `replay.rs`, `KnifeReplayStatus`) to
   prove swivel geometry stays inside tolerance. Keep exact path as the default
   until that evidence exists.
3. Add tolerance-based simplification and/or arc fitting in the knife planner
   so a smooth SVG curve becomes few moves. Emitting real `G2/G3` needs a new
   interpolation variant carried through `toolpath.rs` → `sequence.rs` →
   `post/sequence.rs` → `checks.rs` → verification; budget that as the larger
   half of the workstream.
4. Any geometry-changing option (simplification, arc fitting) is an explicit
   setting with its tolerance visible, never an implicit behaviour change.

**Acceptance.** The same flower cut with materially fewer reversals at an
unchanged geometry tolerance, with replay evidence inside tolerance and the
before/after numbers recorded.

### W8 — Project and artwork workflow (P2, 6.1 + 6.2)

* **New project (6.1).** The File menu offers only operation-flavoured starts
  (`workspace_ui.rs:130-158`). Add a plain **New** that creates an empty v5
  document (Setup defaults, no artwork, no operations), with a confirmation
  when the current job is dirty. Keep the existing entries as convenience
  shortcuts that start from an SVG *and* create the operation.
* **Delete artwork (6.2).** Already implemented (`inspector.rs:353` →
  `session.rs:458` → `project/v5/commands.rs:369`). Verify it in the tested
  build, then fix findability: a row action in the artwork list, Delete-key
  support for the selected item, and undo. Re-check that removing the last
  artwork leaves a valid, exportable-empty document.

### W9 — Machine settings: touch-off reference and parking (P2, 7.1 + 7.2)

* **Touch-off reference** is W2 step 2; land it in
  `project/v5/machine.rs` + `post/sequence.rs::SequenceProfile` first.
* **Parking position.** `program_start_position_mm` exists
  (`post/sequence.rs:54`) but is used only as the *start* bridge (`:868`,
  `:1151`); the program ends `M5 M9 M2` with no move (`:955`). Add
  `parking_position` (machine X/Y, plus a park-Z strategy: clearance /
  stock-top + offset / explicit Z) to the machine configuration, and emit
  **once per program, after the last stage**: retract to safe Z → move to the
  parking position in machine coordinates (`G53`) → `M5 M9 M2`. Never per
  operation. Unset parking keeps today's output byte-for-byte.

**Tests.** Post readback: the emitted program ends with exactly one parking
block before `M2`; the numeric readback confirms the machine-coordinate
position; a job with parking unset is unchanged.

### W10 — Facing simulation performance (P1 in the report's order, no data)

The report lists "facing simulation performance" as item 9 but gives no
numbers. Treat it as a measurement task first: time plan + simulation for the
face test geometry at the display presets in use, record it next to the
existing GUI9 benchmarks, then decide whether a workstream is warranted. Do not
optimize before the number exists.

---

## 5. Sequencing

**Stage 0 — pin and reproduce.** Confirm the tested build (Q1). Reproduce all
13 items against it and against `main`; write
`docs/flat-v-carve/fieldtest-2026-09-baseline.md` with one row per finding, the
fixture, the command and the observed result — including the physical setup
(tool, stock, mat, offsets) so the fixes can be re-tested identically.

**Stage 1 — P0 safety.** W1 first, then W2's datum/prominence half.
Gate: the no-plunge-over-material invariant is a *basic check* in
`checks.rs::check_plan_v5`, covered by tests with a non-plunge-capable tool.
Ship a review package; this is the stage the tester re-runs on the machine.

**Stage 2 — P1 correctness.** W5 (coordinate spine) → W6 → W3 → W4 → W7.
Gate: preview/G-code equality tests and the coordinate invariant tests pass;
the accepted flower job and the knife fixtures are re-measured.

**Stage 3 — P2 workflow.** W8, then the machine half of W2/W9.

Every stage ends with: `cargo test` and `cargo clippy` clean in
`flat-v-carve/`, a rebuilt review package when the GUI is visible
(`./scripts/build-gui.ps1`), a `gui*-progress.md`-style note, and an evidence
artifact under `flat-v-carve/artifacts/`.

---

## 6. Verification matrix

| Finding | Automated evidence | Manual / physical |
|---|---|---|
| 1.1 facing plunge | unit tests over overruns × patterns × angles; entry-safety fixture; post readback equality | re-run facing on the test stock with the face mill; confirm no descent outside the intended entry zones |
| 1.2 Z datum | "datum mismatch" warning test; export gate test | intentionally set the wrong datum once; confirm the program refuses or warns before motion |
| 2.1/2.2 facing params | parameter-definition tests; stock/artwork invariance test; overlay test | change each parameter and watch the preview: stock and artwork must not move |
| 3.1 CSS | fixture set from real Inkscape output imports with warnings listed | re-run the report's 7-step Inkscape scenario |
| 3.2/3.3 coordinates | golden bounds test; resize invariance test; out-of-stock issue test | import an A4 drawing, resize to 100×100, confirm nothing moves and any outside geometry is named |
| 4.1 thin features | needle fixture depth assertions; flower job at 1 mm and 2 mm | carve the flower at 2 mm and inspect the stems |
| 5.1 knife | motion/full-stop counts before/after; replay tolerance evidence | cut the cardboard test piece again and judge the motion |
| 6.1/6.2 workflow | New project → empty operation list; delete artwork → valid document, undo | find and use both actions without instructions |
| 7.1/7.2 machine | profile round-trip tests; parking readback test | run a program to completion and confirm the gantry parks clear of the part |

---

## 7. Risks

* **Renaming probe keys.** GUI field labels double as test probe keys
  (`state.rs::FIELDS`). Any rename (W3) must update the tests in the same
  commit or the review package's probe contract breaks.
* **Relaxing knife path control.** Blending changes swivel geometry; keeping it
  behind verified tolerance evidence is the whole point of W7 step 2.
* **Thin-feature semantics.** Changing the depth rule changes material
  removal; it must be a documented, visible setting rather than a silent
  improvement (W6, Q4).
* **Coordinate-contract changes.** No longer a compatibility question (§0):
  saved jobs need not survive a mapping change, so the mapping can be chosen on
  merit. The landed mapping (page top → stock maximum Y) is kept because it is
  the convention a CNC operator expects, not because changing it is expensive;
  changing it would still churn the fixtures and tests that pin it.
* **Machine settings growth.** Every new machine field must follow the
  "never fabricate a default" rule in `project/v5/machine.rs` — a missing
  field is reported, not guessed.

---

## 8. Open questions

1. **Which build** was tested, and can we have the input files (the Inkscape
   SVG, the job file, the exported G-code) for each finding? The exported
   G-code for the facing crash and the knife run would let us confirm 1.1 and
   quantify 5.1 without a machine.
2. **Z datum:** should the machine configuration be able to *warn about* a
   conflicting job datum, or should the machine reference be authoritative and
   force the job datum? **Answered 2026-09-13: either is fine for now —
   document it in the (?) help and make the active datum prominent.** The
   machine-reference field and any warning/gate are therefore deferred; see
   W2 Status.
3. **Facing semantics:** is `entry`/`exit` overhang additive to the facing
   offset/margins, and is it measured to the tool centre or to the swept edge?
   The report's own example (end at `100 + 20`, with the parenthetical
   `− 25.5`) leaves this open. **Deferred by the tester on 2026-09-13.** What
   landed in W1 does not depend on the answer beyond the one decision recorded
   in the plan doc: overrun is measured to the cutter centre along the pass
   direction, and it is travel beyond the coverage boundary (margins included)
   rather than an addition to the facing offset.
4. **Flat Z thin features:** for a stem narrower than the tool can reach,
   should the CAM engrave a single centerline at the requested depth (removing
   more material than drawn) or refuse it by name? **Deferred by the tester on
   2026-09-13; W6 stays unstarted.**
5. **Drag knife:** may smoothing move the path by a stated tolerance, or must
   geometry be preserved exactly and only the output representation change
   (arcs instead of chords)?
6. **Stock resize:** which anchor should be the default (min corner, centre,
   artwork bounds), and should artwork be movable back inside automatically?
7. **Parking:** machine coordinates (`G53`) or work coordinates, and which Z
   strategy should be the default?
8. **Plunge capability:** should a non-plunge-capable tool be a hard export
   gate everywhere (profile, facing, pocket entries), or only where the
   planner already has an outside-stock entry?
9. **The `cam` CLI's document format.** The CLI still reads and writes the
   schema-3 `job::Job`, which is the last consumer of that model. Should the
   CLI speak schema 5 (leaving one document model), or is the schema-3 format
   worth keeping for its own sake? §0 means compatibility is not part of this
   question.
