# Facing behaviour — plan (W1 + W3, plus the entry choice)

**Status: implemented 2026-09-15** — see
[facing-behaviour-progress.md](facing-behaviour-progress.md) for what landed
against each slice and what is deliberately left open.

This is the working plan for the facing half of
[field-testing-fixes-plan.md](field-testing-fixes-plan.md): workstream **W1**
(no depth change over material, P0) and workstream **W3** (facing parameters,
labels and preview, P1), plus one requirement neither covers — **one entry the
user chooses**, so a tool that cannot plunge descends at the one place whose
clearance the user has actually verified.

The field-testing plan is guidance, not a specification. Where this plan
disagrees with it, this plan wins for facing; the semantic changes are written
back into [2.5d-cam-plan.md](2.5d-cam-plan.md) §11 and, where coordinates are
involved, [technical-design.md](technical-design.md).

The goal is not only safety. It is that a user can predict, **before**
generating, where the tool enters the material, where it travels, and what
every number in the panel does — and that the generated program does exactly
what the preview showed.

Not in scope here: arbitrary pass angles and rotated-area scanning (§11.2 of
the CAM plan), a protected-boundary face mode, ramped entry along the pass
(listed as a follow-on below), and the facing simulation measurement (W10).

---

## 1. Evidence: what facing does today

All of it verified against `main` (engine `0.7.7`) with two throwaway probes in
`crates/cam-core/tests/` (run, then deleted; the measurements below are the
evidence and they become permanent tests in §5). The synthetic fixture is a
100 × 60 × 8 mm stock at the origin, a whole-stock face, a Ø10 endmill,
`max_stepdown_mm` 1, stepdown 1, 2 mm of facing depth, `plunge_capable = true`;
real jobs are named where they are used.

### 1.1 The motion chain skips a position at every depth-layer boundary (P0, new)

At the end of a layer the planner retracts and then hands the next layer's
first pass a `start` at the entry point — a position the tool has never been
at. Zig-zag, two layers, 42 motions: motion 20 is the retract, ending at
`(-5.000, 55.000, 5.000)`; motion 21 *declares* a start of
`(105.000, 5.000, 5.000)` and ends at `(105.000, 5.000, -2.000)`. 110 mm of
travel is missing from the plan. One-way has the same single gap per layer
boundary (78 motions, gap after motion 38: `(105.000, 55.000, 5.000)` →
`(-5.000, 5.000, 5.000)`), and so does the tester's own job: three gaps, at
`x = 225.100`, one per layer boundary after the first.

`post/sequence.rs` emits exactly one block per motion and takes only
`motion.end`; the machine's position carries over modally. The machine
therefore executes the straight line from where it really is to the next
endpoint — `G0 X105.000 Y5.000 Z-2.000` from `(-5.000, 55.000, 5.000)`.
Sampled along that segment, it is below stock top with the cutter sweep
overlapping the stock to a depth of **1.997 mm**, the full facing depth. (If
the control moves Z first and XY after, the XY leg then ploughs at full depth
instead; either behaviour is a crash, the linear one is what LinuxCNC does.)

Whether it cuts depends on the **parity of the row count**, which is the worst
part: at stepover 5 the same job has 11 rows per layer, each layer ends where
the next begins, and the implied move is a transit at the tangent entry plane
that touches nothing. At stepover 6 (10 rows) it cuts the full depth. With
overruns 10/10 and 10 rows it cuts 1.461 mm. Nothing in the settings names that
difference.

The tester's job escapes only by the same accident: its three implied moves run
along `Y` at `x = 225.100`, which is *exactly* the tangent plane of the
modelled stock (`200 + 25.1`), so they cut nothing — provided the blank is
exactly the modelled stock. One millimetre of blank outside the rectangle turns
each of them into a rapid that cuts up to the full depth, right next to the
entry the user believes is clear.

`checks.rs` cannot see any of it: the entry invariant reads `motion.start` and
`motion.end`, and the claimed start is the entry point in the air, so the
descent looks clear. The invariant that the field-testing plan calls the
durable half of W1 is only as strong as the chain's continuity. Emitted as a
real motion, the same travel would be refused immediately: `descent_clear`
requires both ends clear **and** no crossing of the stock, and a rapid is never
authorized.

This is the P0 the field report describes, one layer below the entry check that
was landed for it: the fix that made an *entry* safe never touched the
*between-layer* move that does the same thing.

The same probe ran over every shipped v5 fixture that plans — the 12-stage
flower-box batch job (137 717 motions), the knife jobs and the lettering job —
and found **zero** discontinuities outside facing. Facing is the only planner
that retracts at the end of a repeat and then re-declares a start, so making
continuity a basic check costs nothing elsewhere.

### 1.2 The plunge side alternates with the layer (the user's complaint)

`reversed = zigzag && (layer_index + row_index) % 2 == 1` (`face.rs`). Layer 0
therefore starts every pass at the coverage's low end and layer 1 starts at the
high end. Measured on a 200 × 100 stock with a 100 × 50 face rectangle at
(50, 25), entry overrun 60, exit overrun 0: layer 0 enters at `x = -15`, layer 1
at `x = 215`. Both are clear of the stock, so the plan completes — but the two
entries are at opposite ends of the panel, and *both* ends must be clear for a
tool that cannot plunge. There is no way to ask for one side.

The overrun itself is already directional in the sense W1 landed: `pass_span`
gives the entry overrun to whichever end the pass starts from. What is missing
is control over *which* end that is across the operation.

### 1.3 "Outside the stock" is asserted, not explained

The entry verdict is a distance test against `setup.stock.xy`
(`face.rs::entry_clear_of_stock`), and that rectangle is the only material the
planner knows about. The assumption itself is the right one — the stock
rectangle is where the cut depth is measured from, so it has to be the material
— but nothing in the panel, the preview or the issue text says which rectangle
the clearance was measured against, so "clear of the stock" reads as "clear of
whatever is on the table".

The decision here is to **state** the assumption rather than widen it: no
second material rectangle, no "blank bigger than the stock" mode. A blank
larger than the modelled stock is a job that does not describe the setup, which
CAM cannot see and will not guess at; what it can do is say which rectangle the
verdict used, in the panel, in the help and beside the entry.

### 1.4 The refusal is accurate but arrives late and reads vaguely

With the same 200 × 100 stock and rectangle, a non-plunge-capable tool and both
overruns at 0, the planner reports `FACE_ENTRY_UNSAFE: the tool is marked as
unable to plunge and the pass entry at (45.000, 30.000) still overlaps the
stock (the Ø10.000 cutter is 5.000 mm short of clearing it): add entry
clearance with an entry overrun of at least 6 mm, or mark the tool as able to
plunge`. That is a good message — but it only exists after **Generate**, it does
not name the side, and the number it gives is the clearance to *touch* the
stock, not the travel that puts the cutter sweep clear of it.

### 1.5 Language and preview

Six numbers (four `FaceMargins`, `entry_overrun_mm`, `exit_overrun_mm`) are
described by two captions, with no resolved values: no user-facing statement of
what the coverage rectangle is, how big the allowed sweep envelope is, where
the passes start, or what the overrun buys. The preview has no facing overlay
at all (`scene.rs` skips `OperationSettingsV5::Face` when collecting
references), and because the display frame is `stock ∪ artwork ∪ toolpath`, a
large overrun still reads as the whole scene zooming out.

---

## 2. The model to write down

Five rectangles and two decisions, all in setup space, all derived from
settings alone:

| Name | Definition | Where it comes from |
| --- | --- | --- |
| Stock | The material the job claims. The entrance test is against **this** rectangle. | `setup.stock.xy` |
| Face area | The region to flatten. | `FaceArea::EntireStock` or an explicit rectangle |
| Coverage | Face area expanded per side by the margins. What must end up flat. | area + `FaceMargins` |
| Sweep envelope | Coverage given the overruns at each end and dilated by the cutter radius. The **only** region the plan may travel in. | coverage + overruns + radius |
| Entry corridor | The band outside the stock, at the entry, where a descent is clear. | derived from the stock and the entry |

The two decisions the panel must state and the UI must show resolved:

* **Overrun is measured to the cutter centre**, along the pass direction, and it
  is travel *beyond the coverage boundary* (margins already included) — never an
  addition to the face offset. This is the decision W1 recorded and the
  field-testing plan's Q3 leaves open; keeping it is what makes "entry travel
  60 → the tool centre starts at Y = −60" a true sentence.
* **The stock is the material.** Clearance means the cutter's sweep does not
  overlap the stock rectangle. Tangent contact counts as clear (zero overlap),
  which is why a whole-stock face at zero overrun still plans. Nothing else is
  modelled: the panel says which rectangle the verdict was measured against,
  and there is no second extent for a blank that is bigger than the stock.

Invariants to enforce with tests:

1. **I1 · Chain continuity.** Within a stage, every motion starts where the
   previous one ended. The first motion starts at the stage entry the bridge
   drives to. No implicit travel, ever.
2. **I2 · No unauthorized descent.** A motion that lowers Z below the surface
   its stage cuts from does so only where the cutter is clear of the stock,
   where the stage already cut that space, or with a capability that authorizes
   it; a rapid never is. Already landed — but it must be measured on the *real*
   chain (I1), not on claimed positions.
3. **I3 · One entry.** Every descent into uncleared material in one face
   operation happens at the entry the user chose. Deeper layers re-entering
   their own corridor keep the same entry.
4. **I4 · Nothing else moves.** Changing any facing parameter leaves the stock
   rectangle and every artwork coordinate identical. The preview draws the
   coverage, the envelope and the entry corridor as separate things.
5. **I5 · G-code equals the plan.** Every emitted block corresponds to one
   planned motion endpoint, in order, and the readback reproduces the planned
   descent positions and their sides.

---

## 3. One entry, chosen by the user (the new requirement)

**Requirement.** A face operation enters at **one place the user chose**, at
every depth. Alternating ends is the current behaviour and it is not wrong in
the abstract — but it is unpredictable in the workshop: the user has usually
verified one side of the setup (the clamped edge, the cut-off side, the side
facing the operator, the side with the vacuum zone), and a program that plunges
at the other end on the next layer defeats that. Where the tool cannot plunge,
the entry stops being a convenience and becomes the whole safety argument.

**Model.** A new document field with four modes:

```
entry_side: EntrySide   // "min" | "max" | "coordinate" | "alternate"
```

* `min` / `max` are the ends of the **pass travel axis**: for a 0° pass the X−
  and X+ edges of the coverage, for a 90° pass the Y− and Y+ edges. The GUI
  names them by the resolved axis (`Start at X−`), never as "low" or "left",
  because the axis moves with the pass angle. Each uses the entry travel
  (overrun) as it does today.
* `coordinate` is the user's third option: **the entry end is an explicit
  position on the pass axis**, in setup coordinates — "start at `X = -60.000`",
  not "start 60 mm past the coverage". The panel keeps both views visible
  (position and the travel it implies) and lets either be edited, but the
  coordinate is the authority in this mode. It carries two rules:
  * the coordinate must be at or beyond the pass's own travel limit
    (`coverage.min − radius` on a 0° pass, or the mirror at the other end); a
    coordinate inside the coverage would leave the strip behind it unswept and
    is refused with a located issue naming the nearest allowed position;
  * the side is *derived* from the coordinate rather than chosen, so the panel
    states it (`entering at X = -60.000 → X− side`) instead of asking twice.

  This is also the mode a preview drag writes: dragging the entry marker to a
  position sets the same field.
* `alternate` reproduces today's per-layer flip. It is kept, because it is the
  only variant that never adds a return travel between layers, and a face mill
  that can plunge has no reason to prefer one side. It is never the default and
  its help text states what it costs: **both** ends need clearance.
* Default for a new operation: `min`.

**How the side is expressed in the panel** (four radio choices, one number
that changes meaning with the mode):

```
Pass entry      [ Start at X− ] [ Start at X+ ] [ Start at… ] [ Alternate ]
Start at…                     X = -60.000
Every pass enters at  X = -60.000   ·   the Ø10 cutter clears the stock by 55.100 mm
```

`Start at…` is the coordinate mode; the travel field beside it stays visible and
shows the resolved `coverage → entry` distance, so "60 mm of travel" and
"X = −60" are never mistaken for two independent settings.

**Consequences, stated per pattern in the panel and in §11 of the CAM plan:**

* *One-way*: one entry selects where every row plunges. Rows already retract,
  travel at clearance and re-enter; only the corridor's end changes.
* *Zig-zag, one entry*: one plunge per layer, always at the chosen entry.
  Within a layer the passes still alternate and link at depth. Between layers
  the tool retracts at the exit end, travels at the clearance plane back to the
  entry (a real, emitted motion — I1), and descends there. The travel is added
  only when the layer's last pass does not already end at the entry.
* *Zig-zag, alternate*: unchanged from today, including the envelope reserving
  the larger overrun at both ends.

**Envelope.** With one entry the envelope becomes asymmetric and honest: the
entry end reaches the chosen position (or `coverage + entry travel`), the exit
end reaches `coverage + exit travel`, each still adding the cutter radius and
the numerical reserve. It stays a pure function of the settings, so a generated
path still cannot authorize itself (a path outside it is
`FACE_ENVELOPE_EXCEEDED`).

**Verification of the side, not just of the number.** The entrance test asks the
same question it asks today (is the cutter sweep clear of the stock at that
position), but now for the chosen entry, and the panel shows the answer before
generating:

```
Pass entry      [ Start at X− ] [ Start at X+ ] [ Start at… ] [ Alternate ]
Passes run along X; stock spans X 0.000 … 200.000
Every pass enters at  X = -65.100   ·   the Ø10 cutter clears the stock by 60.100 mm
```

or, when it cannot:

```
Every pass enters at  X = 45.000    ·   the Ø10 cutter overlaps the stock by 5.000 mm
This tool cannot plunge: add 6 mm of entry travel, or enter from X+.
```

With `Alternate` the same line reads `entries alternate between X = -65.100 and
X = 215.100` and carries the warning that both ends must clear.

---

## 4. Slices

Ordered; each is a commit-sized slice with its own tests. S1 is the P0.

### S1 — A truthful motion chain, and one entry (cam-core)

1. `face.rs`: every transition between motions is emitted, never implied. The
   between-layer transition becomes retract → travel at the clearance plane →
   descent, as real motions; the first motion of a layer no longer *claims* a
   position the tool is not at.
2. `face.rs`: `pass_span` and the raster loop honour `entry_side`. The
   `Alternate` variant is the current parity rule; `min` / `max` make every
   layer start at that end; `coordinate` places the pass end at the given
   position on the pass axis and refuses a position inside the coverage
   (`FACE_ENTRY_INSIDE_COVERAGE`, naming the nearest allowed position).
3. `checks.rs`: require I1 as a basic check (`PLAN_MOTION_DISCONTINUITY`,
   naming the two motions, the gap and the tool), and measure I2's descent from
   the previous motion's end when the chain is inconsistent, so a discontinuous
   plan cannot hide a descent behind a claimed start.
4. `checks.rs` / `face.rs`: the entry text names the side and the travel that
   puts the **sweep** clear, not the travel that merely avoids contact.

### S2 — The field, the labels and the resolved numbers (document + GUI)

1. `FaceSettings.entry_side` in the v5 document, the schema-4 mirror, the
   resolver, and the "add a face operation" defaults. `coordinate` adds one
   numeric field (`Face entry at`, the next free id) whose help says it is a
   position along the pass direction in setup coordinates.
2. **No field renames.** The user's decision: `Face entry overrun`, `Face exit
   overrun` and the four `Face margin …` labels stay exactly as they are, so
   `state.rs::FIELDS`, the GUI tests and the probe contract are untouched. The
   work is entirely in the `(?)` help and in what the panel shows resolved
   beside the numbers.
3. Help text, per facing field, written as definitions rather than reminders:
   coverage = area + margins; travel = cutter-centre travel *beyond* the
   coverage boundary, measured to the centre, applied at the entry end and the
   exit end; the envelope is coverage + travel + cutter radius and is the only
   region the tool may travel in; `Alternate` needs both ends clear; and
   **clearance is measured against the stock rectangle**, so a piece larger
   than the stock rectangle is out of scope — enter the piece as the stock and
   the region to flatten as the face area.
4. Panel readout, computed from settings with no plan needed: the coverage
   rectangle, the sweep envelope, the entry in axis terms, the resolved entry
   position, the clearance there, and the minimum entry travel that would clear
   the stock. One click applies that minimum (in `min` / `max` mode it writes
   the travel; in `coordinate` mode it writes the position).

### S3 — Preview: request, coverage, envelope, corridor (cam-gui)

1. Draw, distinctly: the face area (request), the coverage (area + margins), the
   sweep envelope, the entry corridor and the exit band, the planned descent
   points (marked, coloured by the verdict), and the stock rectangle exactly
   `setup.stock.xy`.
2. Frame: default `stock ∪ artwork` (the plan already normalizes every vertex
   against one frame, so nothing inside can rescale); the envelope and the
   travel extent join the fit only when the overlay is shown. `scene_frame.rs`
   keeps the invariance assertions.

### S4 — Verification (tests and evidence)

The tests of §5, plus a post-readback evidence artifact under
`flat-v-carve/artifacts/` for a two-layer zig-zag face and a two-layer one-way
face with a non-plunge-capable tool.

### S5 — Documentation write-back

`2.5d-cam-plan.md` §11.1 (vocabulary, entry side, envelope, the
stock-is-the-material assumption) and §11.2 (the raster order with a fixed
entry side); the field-testing plan's W1/W3 status; a `gui*-progress.md`-style
note and a rebuilt review package (`scripts/build-gui.ps1`) because the panel
changes.

### Follow-on, not in this batch

**Ramped entry.** A tool marked `ramp_capable` may descend along the row inside
the entry corridor instead of plunging at a point (`Ramp angle`, `Entry length`
already exist as fields 14/15 and facing ignores them). That is the natural
second answer for a non-plungeable face mill and it reuses the same corridor;
it needs its own decision about what stops the ramp from passing the stock edge.

---

## 5. Tests

Core (`crates/cam-core/tests/face_core.rs`, plus a readback test):

1. **Chain continuity.** For a two-layer face at both patterns and both angles,
   assert every `motions[i].start == motions[i-1].end` and that the first motion
   starts at the stage entry. Fails today at every layer boundary. Include the
   parity pair (10 and 11 rows per layer at zero overrun) so the test that
   passes today and the one that gouges 1.997 mm are pinned next to each other.
2. **The gap cannot come back.** Build the pre-fix chain by hand (a retract
   followed by a motion claiming a start elsewhere) and assert `check_plan`
   reports `PLAN_MOTION_DISCONTINUITY` and that the descent is judged from the
   previous end.
3. **Entry choice.** Entry × pattern × angle × plunge capability × overruns:
   every descent below stock top is (a) at the declared side and (b) either
   clear of the stock or authorized. Includes the regression the user reported:
   zig-zag, both layers, entry travel 60 / exit travel 0, non-plunge-capable —
   today two entries at opposite ends, after S1 one side only.
4. **Zero overrun, whole stock, non-plunge-capable** still plans (tangent
   clearance) — the W1 acceptance line, kept.
5. **Minimum travel.** One reserve below the reported minimum → a refusal
   naming the side and the number; at the minimum → plans.
6. **Coordinate mode.** An explicit entry position produces the same motion as
   the equivalent travel (`X = -60` ⇔ `coverage.min − radius − 60`); a position
   inside the coverage is refused with `FACE_ENTRY_INSIDE_COVERAGE` and the
   nearest allowed position; the derived side appears in the issue text.
7. **Envelope.** One entry makes the envelope asymmetric; a motion outside it
   is `FACE_ENVELOPE_EXCEEDED`; an `Alternate` plan still reserves the larger
   overrun at both ends.
8. **Readback.** Emit the program for (3) and assert one block per planned
   motion, in order, with the descent endpoints and their sides reproduced.

GUI (`crates/cam-gui/tests/scene_frame.rs`, `face_ui.rs` probes):

9. Changing each margin, each overrun, the pattern, the angle and the entry
   leaves the stock rectangle and the artwork geometry byte-identical.
10. The panel readout for the field report's example (stock Y 0…100, face offset
   20, entry travel 60, Ø51 cutter → entry at `Y = -60`) — the same numbers the
   planner uses, asserted against the plan rather than typed twice.
11. The entry, the corridor and the envelope are present in the preview
    payload for a two-layer face, and the descent markers carry the verdict.
12. The four entry modes keep their fields bound: switching modes never
    transfers text between the travel field and the coordinate field.

---

## 6. Decisions to take

Answered on 2026-09-15:

1. **Blank bigger than the stock — not supported.** The stock rectangle is the
   material. No "material extent" field, no second rectangle; the help and the
   panel state the assumption instead.
2. **One-side entry is the user's choice**, and it gains a third mode: an
   explicit entry position (`Start at…`) beside `min` / `max` / `alternate`.
3. **Field labels stay as they are.** All the clarity work goes into the `(?)`
   help and the resolved readout next to the fields.

Still open, smaller:

4. **Default mode for a new operation.** `min` is proposed; `alternate` keeps
   today's output byte-for-byte.
5. **Travel measured to the cutter centre** (field-plan Q3) — kept from W1; the
   panel's resolved sentence depends on it.

---

## 7. Sequencing and gates

* S1 first and alone: it is the P0, and it changes the emitted program for every
  existing multi-layer face.
* Gate after S1: `cargo test` + `cargo clippy` clean in `flat-v-carve/`; the
  continuity and entry-side tests pass; the two-layer zig-zag fixture's readback
  contains no block that lowers Z below stock top outside the entry corridor.
* S2 + S3 together produce the visible half and the review package; S4/S5 close
  the evidence and the documents.

## 8. Acceptance

* No G-code block lowers the tool into material outside the declared entry
  corridor, for any pattern, angle, overrun or layer count — and no planned
  motion escapes that check because the chain skipped a position.
* A non-plunge-capable tool faces a rectangle inside the stock by entering
  where the user chose, on every layer — a side, or an explicit position — or
  is refused with a located reason that names the entry and the travel needed.
* Before generating, the panel says where the passes enter, how much clearance
  the entry has, and how big the coverage and the sweep envelope are, and it
  says which rectangle that clearance was measured against.
* Changing any facing parameter never moves the stock or the artwork. The
  preview draws the coverage; the request, the travel and the entry are stated
  with numbers in the panel, not drawn — a decision taken with the tester on
  2026-09-15 after the first overlay set proved unreadable.
