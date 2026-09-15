# Machine-motion simulation — plan

Source: review discussion 2026-09-15, immediately after `0fe0a8d` *Animate
playback at a rate the transport controls*. The tester's finding: the Speed
control is "not really an animation, more of a jumps between motions". It was
"not as apparent during simulation of v-carve, which had hundreds and thousands
of motions, but for facing job with only 20 motions — it is a stop-motion
between states". The requested direction is to "move simulation towards proper
machine work visualization — with feed rates, with models of the tools
(generated from tool geometry we have) and spindle to check for collisions and
so on".

**Status: S1–S3 implemented, S4 planned.** The timed playback slice (§6), the
machine-definition rapid rate (D13), the cutter on screen and the assembly with
its two machine warnings have landed; what they changed, and the points where
the implementation learned something the plan did not know, are recorded in
[gui10-progress.md](gui10-progress.md). S4 — the authoritative check in
`cam-core` — is not started.

The tool-assembly questions are answered (§2, D7): the shaft diameter and the
stickout are optional tool-library parameters, and the holder is an optional
selection in the machine definition; the rapid rate comes from the machine
definition too, and the collision warnings stay display-only for this slice
(§12). The one remaining question (whether tool-change and spin-up time joins
the reported total) has a working recommendation, so nothing blocks a start.

This plan re-opens a documented deferral rather than extending a committed
contract: [architecture.md](architecture.md) excludes "general fixture/holder
collision simulation", and
[2.5d-cam-plan.md](2.5d-cam-plan.md) §15.3 lists "holder/fixture models" under
*optional later*. Everything else the simulation already promises — the tiled
heightfield, motion-indexed checkpoints, paged paths, the 250,000-motion display
bound, "a raster or screenshot never becomes evidence for planning or export" —
stays as it is. Compatibility is not an input: the project is pre-release and
does not preserve saved jobs
([field-testing-fixes-plan.md](field-testing-fixes-plan.md) §0), so a schema
change is allowed when it is the better design.

---

## 1. The complaint in the code's terms

Today's transport is a **step clock**, not a machine clock:

| Piece | What it does | Where |
|---|---|---|
| Playback rate | `PLAYBACK_SECONDS = 20`; the playhead advances `(motions / 20).max(1) × speed × dt` | `viewport.rs` — `PLAYBACK_SECONDS`, `Viewport::show` |
| Step | only whole motions: `playback_progress.floor()` then `stock_seek(prefix + step)` | `viewport.rs` — `Viewport::show` |
| Speed control | 0.5× … 15×, a multiplier on that normalization | `viewport.rs` — transport bar |
| Stock state | each step is requested from the worker and returns one raster frame | `viewport.rs` → `app.rs` → `session.rs` → `Playback::seek` |
| Motion record | kind, tool, stage, six coordinates; **56 bytes, no time information** | `sim.rs` — `Motion`, `MOTION_BYTES` |
| Plan motion | already carries `interpolation` (rapid / linear feed) and `feed_mm_min` | `cam-core/toolpath.rs` — `PlannedMotion` |
| From plan to display | both fields are dropped when the simulator stream is built | `scene.rs` — the `motions.push(Motion { … })` loop |
| Cutter on screen | none: the viewport draws stock, paths, artwork and the inspection marker | `viewport.rs`, `scene.rs`, `stock_render.rs` |

Every motion therefore costs the same wall-clock time whatever its length and
whatever its feed, and a job is played as a sequence of states.

### 1.1 The facing job the tester used

`real_data/facing_job.json` as it stands: a 50.2 mm endmill (12 mm cutting
length), 200 × 100 × 18 mm stock, 1 mm stepdown, 25 mm stepover, 2400 mm/min
cutting feed, 600 mm/min plunge feed, 50 mm entry overrun at each end of every
pass. The tester reports twenty motions; the job's 18 mm depth at 1 mm stepdown
with a 25 mm stepover implies roughly ten feed passes of about 300 mm and ten
plunges of 14 mm. The numbers below are the shape of the problem, not a
measurement of the generated plan — S1's test measures the real plan, and it is
the first thing that should be recorded.

| | Today | Program time |
|---|---|---|
| One 300 mm pass | 1 s at 1× (all motions are equal) | 7.5 s |
| One 14 mm plunge | 1 s at 1× | 1.4 s |
| Whole job | 20 s at 1×, 1.3 s at 15× | ≈ 89 s of motion |

The stop-motion is not a frame-rate problem: the model has no notion of motion
duration, so the only thing the transport can animate is the index. The same
numbers are the acceptance criterion for S1 — the feed pass must take ~5.4× the
plunge, not the same second.

---

## 2. How FreeCAD CAM and Fusion define a tool

Researched 2026-09-15 by fetching the projects' own sources and real exported
libraries (listed at the end of this section) rather than working from memory.
The point is to take the parts that answer our question and to name the parts
we are deliberately not copying.

### 2.1 FreeCAD CAM

| Piece | What it holds |
|---|---|
| A tool is a *shape* plus values | `toolbitshape` + `toolbit` assets: the shape class declares the schema, the tool bit stores the values |
| Endmill shape | `Diameter`, `ShankDiameter`, `CuttingEdgeHeight`, `Length` ("Overall tool length"), `Flutes` |
| V-Bit adds | `CuttingEdgeAngle`, `TipDiameter` |
| Holder | **none.** The CAM asset types are `toolbit`, `toolbitlibrary` and `toolbitshape`; the machine model has a *toolhead* (type, min/max RPM, tool change mode, wait) and no holder geometry |
| Simulation | not a heightfield: `PathSim::SetToolShape(shape, resolution)` sweeps the tool's solid through the stock (`ApplyLinearTool`, `ApplyCircularTool`) |

So FreeCAD already keeps a **shank diameter** beside the cutting geometry, and
it simulates the *whole tool body* — whatever the shape contains is what
collides. What it has no concept of is the free length below the holder.

### 2.2 Fusion 360

A real exported library carries two-letter mnemonics **plus** an `expressions`
block that spells them out, so the meanings are not folklore:

| Key | Meaning |
|---|---|
| `DC` | `tool_diameter` |
| `RE`, `TA` | corner radius, taper angle |
| `SFDM` | `tool_shaftDiameter` |
| `LCF` | `tool_fluteLength` |
| `shoulder-length`, `shoulder-diameter` | shoulder length and diameter |
| `LB` | `tool_bodyLength` |
| `OAL` | `tool_overallLength` |
| `NOF`, `HAND`, `CSP` | flutes, hand, centre-cutting flag |
| `assemblyGaugeLength` | the free length from the tool tip to the holder — the stickout |
| `shaft`, `holder` | `{type, segments: [{height, lower-diameter, upper-diameter}], vendor, description, unit}` — a stack of truncated cones |

Measured on the libraries inspected: **113 of 113** tools carry both a `shaft`
block and an `assemblyGaugeLength`, the shaft segment's height equals the gauge
length in every one of them, and where a holder is assigned it is the same
segment representation (`Langmuir Collet`: one 1.328 in diameter × 1.5 in
cylinder). An independent converter maps `Lb → stickout`, `Sfdm → Shank_Dia`,
`Lcf → Flute_Len`, `ShoulderLength → Shoulder_Len`.

The consequence that matters here: Fusion treats the **stickout as a property
of the tool**, stamped on every tool in its geometry block, and the **holder as
a separate reusable object** with a segment-stack body. That is exactly the
split this plan takes.

### 2.3 What we take, and what we leave

Adopted:

* `shaft_diameter_mm` and `stickout_mm` as optional tool parameters — FreeCAD's
  *shank diameter*, and Fusion's *assembly gauge length*, which an independent
  converter also calls "stickout";
* the holder as an optional selection on the machine definition, resolved from a
  small built-in catalogue of segment stacks (ER11 / ER16 / ER20 / ER32 …);
* one profile-revolution mesh builder over a segment stack plus the cutter's own
  geometry — the representation Fusion uses for its shaft and holder;
* names that state what they measure (`shaft_diameter_mm`, `stickout_mm`,
  `holder`), not two-letter codes.

Left out for now: necked or shouldered cutters (a step in the shank diameter),
flute-count rendering, and Fusion's wider parameter set (corner radius, taper,
tip length, thread fields). None of them is needed to answer the collision
question, and each is a small addition to the same segment stack later.

Neither project's definitions cover our **drag knife**: FreeCAD's shape list has
no blade shape, and the Fusion libraries inspected hold mills only. The knife
keeps our own geometry (`blade_offset_mm`, `max_cut_depth_mm`) and its existing
display model, which already derives the tip from the heading; the segment stack
applies to its holder side like any other tool.

Sources (fetched 2026-09-15): `FreeCAD/FreeCAD`
`src/Mod/CAM/Path/Tool/shape/models/{base,endmill,vbit}.py`,
`.../toolbit/models/{base,endmill,vbit}.py`, `.../toolbit/mixins/rotary.py`,
`.../camassets.py`, `src/Mod/CAM/Machine/models/machine.py`,
`src/Mod/CAM/PathSimulator/App/PathSim.cpp`; `ckoval7/fusion360-tool-library`
(`Langmuir-Flycutter.json`, `Langmuir-Non-Ferrous-DLC.json`);
`verkstaden5/verkstaden5-fusion360-tool-library` (the 113-tool hobby library);
`dgjohnson/F360LibraryConverter` (`F360ToolLibrary.cs`, `Form1.cs`);
`ntc490/fusion360` (`ttable.py`).

---

## 3. What exists today that this plan builds on

| Piece | What it gives us | Where |
|---|---|---|
| Fractional cut window | `Field::apply(motion, start, end)` already cuts the part of a motion between two fractions; **every caller passes `0..1`**, so the window is implemented but unexercised | `sim.rs` |
| Exact state rebuild | `Field::from_packed(stock, tools, cell, packed, versions, allocated, stats)` | `sim.rs` |
| Frame metadata | `FrameMeta { prefix, stats, checksum, versions, allocated }` travels with the cells, so a receiver can rebuild the exact field | `stock_preview.rs` |
| Motion stream for the display | `Scene::sim_input()` decodes the transported motions; the cells, versions and allocation mask are already in the payload | `compute.rs` |
| Per-tile upload | the renderer compares tile versions and copies only what changed | `stock_render.rs`, `preview-display-progress.md` |
| Local playback precedent | the retired GUI1 experiment built its own field and `Playback` from exactly this transported data | `experiments/gui1/src/app.rs` — `build_stock` |
| Tool geometry | endmill (diameter, cutting length), V-bit (angle, tip, cutting diameter, cutting height), drag knife (blade offset, max cut depth) | `cam-core/project.rs`, `cam-core/model.rs` |
| Cutter normalization | `ToolSpec::normalize` computes the same radius the field removal uses (V-bit reach = `min(height × tan(θ/2), D/2 − tip/2)`) | `sim.rs` |
| Knife orientation | `blade_heading_deg` per motion, tip derived through `knife_tip`, published per motion in `report["knifeMotions"]` | `cam-core/toolpath.rs`, `scene.rs` |
| Machine facts that exist | clearance, work offset, spindle direction/RPM, `spindle_spinup_seconds`, M6 contract, path control | `project/v5` — `AppliedMachineConfiguration`, `post/profile.rs` |

What does **not** exist anywhere: a rapid rate, a shank diameter, a stickout, a
holder, or a spindle body. `ToolGeometry` has no field for any of them.

---

## 4. The model

### 4.1 One duration per motion

```
linear feed : seconds = |end − start|₃ / feed_mm_min × 60
rapid       : seconds = |end − start|₃ / rapid_rate_mm_min × 60
zero length : no time at all: no state changes across the move
```

`|·|₃` is the 3D length: a plunging or ramping move is timed by its real path
length, not by its XY projection. The rapid rate is **machine data** (D13): the
machine definition carries an optional `rapid_rate_mm_min`, the applied machine
configuration copies it into the job, and the clock uses it when it is there.
A machine that does not state one is still simulated: the clock uses a stated
display fallback and the time readout says the total contains an assumed rapid
rate, rather than quietly presenting an invented machine value as fact.

**Implemented, with one correction.** A per-motion floor (this section first
said 0.05 s) would have been worse than useless: a V-carve's motions are
fractions of a millimetre, so a floor would have added an invented minute for
every 1,200 segments of a long job. A zero-length move now costs nothing and is
skipped when a time is converted back to a position, which is both honest and
enough to keep the arithmetic finite. The rest of this section landed as
written; see [gui10-progress.md](gui10-progress.md) for the geometry-frame fix
the fractional window needed and for the seeding rule the display clock uses.

The scene publishes a prefix time table: `time_at(prefix)` by binary search and
`prefix_at(seconds)`. Eight bytes per motion — about 2 MB at the 250,000-motion
display bound — built once per execution from the same rule on both sides, with
one test pinning the rule.

### 4.2 The playhead becomes a pair

The position is `(prefix, fraction)`, and the drawn state is the exact field at
`prefix` plus `Field::apply(motions[prefix], 0.0, fraction)`. That single
sentence covers every case in this plan:

* **play** advances the fraction and rolls it into the prefix as it reaches 1.0;
* **scrub to a time** restores the prefix at or before that time, then applies
  the fraction — the same two lines;
* **the tool position** is `start.lerp(end, fraction)`, in setup coordinates;
* **the drawn path** ends where the tool is, so the picture never shows material
  removed ahead of the cutter.

Two properties make this safe, and both must be tests rather than hopes:

1. **A partition is exact.** Splitting a motion into fractions and applying the
   pieces in order must produce the same field as applying it once — the cell
   depths are a union of tool envelopes over sub-segments of one straight move,
   so this should hold, but the fractional window has never been used and the
   V-bit candidate search (`coverage`, the tip circle, the stationary point) is
   subtle enough to deserve evidence (D5).
2. **Stats count a motion once.** `Stats::applied_motions` must not grow per
   fraction; the removed volume already accumulates as `level − previous` and
   stays correct under repeated application.

### 4.3 Only the display can own the clock

The compute worker is a persistent, disposable process reached through a
private mailbox that the supervisor polls every 5 ms, and a response carries the
whole scene payload (`worker.rs`). A worker-side clock would mean a serialized
round trip per displayed frame — the wrong answer at any frame rate. The
display already holds everything needed to run the clock itself: the motion
stream, the packed cells, and the per-frame `versions`, `allocated`, `stats` and
`checksum` needed by `Field::from_packed`. This is what the GUI1 experiment
did before the field was moved into the worker, so it is a re-instatement of a
measured design, not a new one.

The division of labour:

* **the worker keeps authority.** `Playback::seek` stays the only way the
  displayed position becomes a retained state again; checkpoints, the seek
  report and the transfer accounting are unchanged;
* **the display owns interpolation.** While playing it advances locally and
  re-uploads only the tiles whose version changed;
* **they reconcile on stop.** Pause, scrub, a stage jump or a preset change asks
  the worker for one authoritative seek to the new prefix; the fraction is
  applied locally on top. The probe keeps publishing `stockPrefix` (what is
  drawn) next to `requestedStock` (what was asked for), exactly as the current
  build does.

**Implemented, with one simplification.** The display seeds its clock from two
transported frames — the pristine state at prefix 0 and the state it opens on —
not from the whole ladder, which would have doubled the resident raster the
payload already holds. Any position those two cannot reach (a rewind, or a jump
beyond `LOCAL_ADVANCE_LIMIT` motions in one frame) becomes one seek against the
retained execution, and the response reseeds the clock. Forward playback inside
the current state never leaves the display process.

### 4.4 Time is derived, never stored

No time, duration or rate enters the job document or the plan. The job already
states the feeds that the machine will use; the simulation is a viewer of those
values and must not become a second authority for them. If a duration readout
ever disagrees with the emitted program, the program is right.

### 4.5 Knife, tool changes, spin-up

Knife stages animate like any other stage, with the heading interpolated between
the motion's start and end headings (shortest way round), so the blade swings
through a corner swivel instead of snapping. The tip stays the derived
`knife_tip(pivot, heading, offset)` point.

Tool changes are the one place where program time is not motion time. The only
known value is `spindle_spinup_seconds` in the applied machine configuration;
the M6 macro's internal motion is explicitly outside the model
([2.5d-cam-plan.md](2.5d-cam-plan.md) §14). The transport therefore reports
**modeled motion time** and marks tool changes as untimed boundaries, rather
than presenting a total that pretends to be a cycle time.

---

## 5. Decisions

**D1 — Real time becomes the transport's domain.** 1× means one second of
program per second of wall clock. The speed control becomes a time scale
(extended well past today's 15× for hour-long carves), and gains one **Fit**
option that maps the whole program into about 20 seconds while keeping every
motion's relative duration. Fit is how a V-carve with 137,000 motions stays
watchable; 1× is how the 20-motion facing job becomes a machine instead of a
slideshow.

**D2 — Time is derived from the plan, not stored in it** (§4.4).

**D3 — The display owns the animation clock; the worker keeps authority**
(§4.3).

**D4 — State is `prefix + partial motion`.** The authoritative prefix only ever
moves in whole motions; the fraction is display-only and always exact, because
it is re-derived from the prefix state rather than accumulated.

**D5 — Fractional application must be proved, not assumed.** A partition-equality
test (one motion applied in ten pieces equals one shot, cell bytes and volume)
and a motion-counting test land with the first use of `Field::apply(start, end)`.

**D6 — The cutter mesh is generated from the same normalized geometry the
removal uses.** A mesh profile test asserts the mesh radius at each depth equals
the simulator's cutting radius at that depth for an endmill, a V-bit with a
truncated tip, and a knife. Two independent models of one cutter is the defect
this decision exists to prevent.

**D7 — The assembly has two homes, both optional, and nothing is invented.** The
**tool** carries `shaft_diameter_mm` and `stickout_mm` (tip → holder bottom) in
the tool library, copied into the job with the rest of the tool's geometry; the
**machine definition** carries an optional holder selection (ER11/ER16/ER20/
ER32 …) resolved from a built-in catalogue of segment stacks (§2). A job with
no stickout, or a machine with no holder selected, is a valid job: the display
draws the cutter alone and the readout says which parts were modeled, so the
warning list can never imply a collision test that did not run.

**D8 — The cutter is drawn through the stock by default.** A cutter hidden under
the surface is precisely the state the user is looking for, so the tool pass
ignores stock occlusion by default and offers the depth-tested view as an
option, reusing the existing x-ray/path toggles rather than adding a second
appearance system.

**D9 — Only two collision families are checked in this slice**, because only
these two are honest on a heightfield (§7): the **shank/holder below the local
surface**, and a **rapid that passes through remaining material**.

**D10 — Display-side warnings are labeled and never gate output.** Each warning
names the motion, the local depth, and the raster it was computed on, and the
panel says the check is a display estimate at *N* mm cells. The authoritative
check belongs in `cam-core` with diagnostic codes and an export gate (S4).
Confirmed by the tester on 2026-09-15: warnings stay display-only for this
slice, so no job is refused because of a raster estimate.

**D11 — The spin is drawn at a labeled visual rate, not at the real RPM.** At
16,000 rpm and 60 fps the rotation aliases into noise or into a lie; the tool
spins visibly slowly, or shows a rotation indicator, and the readout states the
programmed RPM as a number instead of pretending to show it.

**D12 — Checkpoints stay motion-indexed.** A time-domain timeline is fine, but
every seek lands on `(prefix, fraction)` and the checkpoint ladder, the seek
report and the transport accounting keep working unchanged.

**D13 — The rapid rate is machine data with a labeled fallback.** The machine
definition (`SequenceProfile`) gains an optional `rapid_rate_mm_min` beside its
clearance and spindle fields, `apply_machine_configuration` copies it into the
job's `machine_configuration`, and the animation clock uses it for G0 moves.
When a machine does not state one, the clock uses a stated display default and
the readout flags the total as containing an assumed rapid rate. The export
profile does not need the value — G0 carries no feed, so this is timing data
only, and it never changes the plan, the checks or the emitted program.

---

## 6. Increments

### S1 — Timed, continuous playback

*Goal:* the twenty-motion facing job plays as one continuous pass of the cutter,
at the ratio the plan commands. **Implemented** — see
[gui10-progress.md](gui10-progress.md) for the measured numbers, the tests and
the two corrections (zero-length moves, and the geometry frame the fractional
window needs).

| | |
|---|---|
| Wire | the motion record grows `56 → 64` bytes: the pad byte becomes the interpolation flag, and one `f64` carries `feed_mm_min`. Per-motion transfer becomes 120 bytes (2 × 28-byte vertex + 64); the display bound and the refusal message are recomputed from that. |
| Simulator | `Motion` gains `interpolation` and `feed`; `encode_motions` / `decode_motions` carry them; the prefix time table is built beside the motion stream. |
| Display | `StockView` keeps a local `Field` + `Playback` seeded from the transported frame (`FrameMeta` + cells + `sim_input`), advanced per frame with `Field::apply(motions[prefix], from, to)`; only dirty tiles re-upload. |
| Transport | Play / Pause / Start / step-back / step-forward, the speed scale with **Fit**, a time slider beside the motion slider, and the readout `elapsed / total (modeled motion time) · feed · tool · stage`. When the applied machine configuration carries no rapid rate, the total is marked as containing an assumed one (D13). |
| Rendering | the path range ends at the tool, not at the next motion boundary; the in-flight motion is clipped to the fraction. The tool itself arrives in S2 — S1 may show the position with the existing inspection marker so the increment is reviewable on its own. |

*Acceptance:* the facing job at 1× takes ≈ 89 s of modeled time; a 300 mm pass
and a 14 mm plunge differ by the ratio of their length and feed, not by one
step each; scrubbing to a time shows a mid-motion state whose field equals a
cold replay to that fraction; the flower batch still plays with only the tiles
that changed uploaded per frame; a machine with a stated rapid rate times its
G0 moves from that value, and a machine without one still plays and says so.

### S2 — The cutter on screen

*Goal:* the modelled cutter is visible, orientated, and cannot disagree with the
simulation. **Implemented** — see [gui10-progress.md](gui10-progress.md) for the
two deviations: the body is drawn through the existing overlay pass rather than
a new `tool.wgsl` (same camera uniform, same depth behaviour, one less pipeline),
and nothing above the cutting portion is drawn until a tool states a shaft
(S3's field).

| | |
|---|---|
| Geometry | one profile-revolution mesh builder driven by `ToolSpec` + `ToolGeometry`: endmill cylinder, V-bit tip flat + frustum, knife holder + blade, and the shaft above the cutting length as a segment stack using `shaft_diameter_mm` when it is set. Reuse `ToolSpec::normalize` so the displayed reach is the reach that cuts. |
| Placement | per frame: tip at `start.lerp(end, fraction)`, knife heading from the interpolated `blade_heading_deg`; work-zero is setup coordinates, as today. |
| Draw | a small `tool.wgsl` pass after the stock pass sharing `Camera::uniform`, drawn over the stock by default (D8), with the depth-tested alternative behind the existing appearance toggle. |
| Readout | current tool and stage (already published in `report["stages"]`), and the programmed RPM as a number. |

*Acceptance:* a mesh-versus-removal test (D6) passes for all three tool kinds; a
knife corner swivel shows the blade turning; the cutter's tip sits exactly on
the last removed cell in the top, elevation and section views at every display
preset.

### S3 — The assembly, and the collision warnings

*Goal:* the user can see the holder and be told where it would hit.
**Implemented** — see [gui10-progress.md](gui10-progress.md) for the catalogue's
provenance rule (standard nut diameters, nominal everything else), the two
checks and their evidence, and the three deliberate omissions: no segment
editor, no timeline markers (the list has a **Show** seek instead), and
`state.rs::FIELDS` / `help.rs` untouched because the assembly is edited in the
tool library rather than the operation inspector.

| | |
|---|---|
| Input | per tool: `shaft_diameter_mm` (optional) and `stickout_mm` (optional, tip → holder bottom) from the job's copied tool geometry, edited in the tool library. Per machine: one optional holder selection from the built-in catalogue (§2, D7). |
| Model | the mesh extends from the cutter, through the shaft, to the holder, positioned so the holder's bottom face sits at `stickout_mm` above the tip; the cutter end is still generated from the job's tool geometry, and a missing stickout or holder simply omits that part of the mesh. |
| Check 1 | the assembly's lowest non-cutting surface against the local remaining surface over its footprint: warn with the motion index and the intersection depth. |
| Check 2 | every rapid against the remaining material along its path: warn with the motion index and the depth. |
| Surface | a warnings list beside the transport — one row per event, each seeking to its motion; timeline markers; the raster and cell size named in the header (D10). Results are cached per execution identity, not recomputed per frame. |

*Acceptance:* a fixture with a deliberately short stickout over a deep pocket
warns at the right motion and stays warned after seeking away and back; a
correct assembly on the same job does not warn; a rapid drawn through a wall
warns and a rapid at clearance height does not; the facing job (whose 50.2 mm
cutter overhangs the stock by design) warns about nothing except what the
assembly genuinely hits.

### S4 — Later: machine truth (not this slice)

Acceleration and junction-limited feeds (a corner does not change speed
instantly), tool-change time from a machine source, the same two checks as
`cam-core` plan checks with diagnostic codes and an export gate, the holder
promoted to authoritative job data, and detaching parts. Each is a separate
increment with its own plan entry; S1–S3 are designed so none of them needs a
rewrite: the time table, the collision samples and the assembly geometry are
already the interfaces.

---

## 7. What a heightfield can and cannot tell us

The display field is a 2.5D removal raster on a display grid. It knows the
remaining **surface height** under any XY, at a cell size the preset chooses
(0.1 mm at Fine down to ~1 mm at Coarse over a large plate).

| Check | Honest on this model | Why |
|---|---|---|
| Holder/shank below the local surface | yes | the surface height over the assembly footprint is a direct query; a wrong stickout is orders of magnitude larger than a cell |
| Rapid through remaining material | yes | sampling the surface along the rapid's path answers it; a rapid that grazes a wall by less than one cell can be missed, so the warning names the cell size |
| Cutter envelope agrees with the removed cells | yes | it is the same computation, and S2 pins it with a test |
| Sub-cell walls, a thin remaining web, a tab thinner than a cell | **no** | the plan already forbids this inference for the display; the resolved contour overlay stays the authority |
| Is the *cut* itself correct (depth, stock left, gouges) | **no** | that is `cam-core`'s bounded stock verification, which stays the authority |
| M6 macro internals, probing, sensor corridors | **no** | unknown machine motion, already outside every model |

So the warnings in S3 are a viewing aid with a stated resolution, and the checks
that may refuse output stay where they already are.

---

## 8. Data and wire changes

| Change | Kind | Notes |
|---|---|---|
| Motion record `+ interpolation + feed` (56 → 64 bytes) | display protocol | both sides build together; `MOTION_BYTES`, `encode_motions`, `decode_motions` and the transport accounting move in one commit |
| Prefix time table | display protocol | derived on both sides from the motion stream; nothing new is stored |
| Tool geometry `+ shaft_diameter_mm + stickout_mm` | **tool library, copied into the job document** | optional, and carried exactly as `diameter_mm` / `cutting_length_mm` are today, so the display reads them from the job and a library edit never changes a saved job. New labelled fields in the library editor, the inspector, `state.rs::FIELDS` and `help.rs` (that list is a probe contract, not a cosmetic detail) |
| Holder selection | **machine definition** | optional, on the machine record itself (`resources.rs` — `SequenceProfile`, stored in the resource catalogue's `machines` and applied as `machine_configuration`), so every job that applies that machine inherits it and the segment numbers are not copied into each job. The geometry then resolves from the built-in catalogue. A per-tool holder reference is the later extension if one machine ever needs two |
| Holder catalogue | built-in constant | ER11 / ER16 / ER20 / ER32 segment stacks (collet nut + body), each a few truncated cones, with a custom option when none fits. The collision-relevant surface of an ER holder is its nut, so the segment diameters and lengths come from the collet standard's published nut and body dimensions and are cited in the code next to the table — not estimated by eye |
| Rapid rate `rapid_rate_mm_min` | **machine definition** | optional, on the same `SequenceProfile` record as the holder selection, copied into `machine_configuration` by `apply_machine_configuration`; a machine without one still plays, and the readout names the assumption (D13). The export profile does not need it |
| Collision warnings | display only | reported in the probe and in the warnings panel; never in the job |

[tool-library.md](tool-library.md) is the authority for the library record, so
its field list and its schema number move with the first row; the holder row
touches that document's rule that machine data (tool numbers, work offsets, the
M6 contract) stays out of the library, which is exactly why the holder goes
into the machine definition rather than onto the tool.

---

## 9. Budgets

* **Per-frame apply.** Playing at a fitted speed crosses a bounded number of
  motions per frame. The display keeps a per-frame ceiling (the existing
  bounded-replay idea, generalised): while the work it must do this frame stays
  under the ceiling it advances locally; above it — a stall, a very high time
  scale, a huge job — it asks the worker for one authoritative seek instead.
  This is the same coarse/fine split the checkpoint ladder already uses.
* **Upload.** Only tiles whose version changed are copied; the probe keeps
  reporting the last transfer and the replayed motion count, now including the
  per-frame local work.
* **Memory.** The display's local field duplicates the packed cells in a second
  layout (4 bytes per cell each). At the Fine preset that is ~4 MiB extra for a
  1024 × 1024 raster; the alternative is to derive the uploaded bytes from the
  field and drop the duplicate.
* **Transport.** `MOTION_LIMIT` is a measured display bound, not a machining
  limit; the refusal message and the recorded ceiling are recomputed when the
  record grows.

---

## 10. Verification

| Level | What it proves |
|---|---|
| `sim.rs` unit tests | duration math (feed, rapid, plunge, zero-length floor); a rapid timed from the machine's stated rate and from the labeled fallback when there is none — D13; a monotone time table; `prefix_at(time_at(p)) == p`; **partition equality** (one motion in ten pieces equals one shot, cell bytes and removed volume) — D5; `applied_motions` counts a motion once; the mesh radius equals the cutting radius at depth — D6; warnings fire on a synthetic deep pocket and stay silent on a correct assembly |
| `cam-gui` integration tests | the facing job's 20 motions produce a modeled duration within a stated tolerance of its feed-time and a per-motion time proportional to length/feed; the flower batch keeps a bounded per-frame apply; a seek to a mid-motion time equals a cold replay to that fraction; the probe reports the drawn position next to the requested one |
| Browser scenario — **landed** | `crates/cam-gui/web/gui10-scenario.mjs`: the facing job carries a machine clock (164.03 s over 63 motions, a rapid rate the machine does not state), 1× advances *inside* one pass (eight samples, fraction 0.084 → 0.510, feed 2400 mm/min), Fit runs the program in 20 s at 8.2×, a tool that states no assembly is not invented, and the tight-holder fixture reports the ER20 nut 0.7 mm inside the material from motion 83 on a 0.4 mm raster with a row that seeks to it. Run with `node crates/cam-gui/web/smoke.mjs --gui10`; the recorded numbers and two screenshots land in `artifacts/gui/browser-smoke/`. It found two defects: a seek dropped the display's clock, and the assembly readout was empty at the end of the program |
| Review package | `scripts/build-gui.ps1`, a `gui10-progress.md` with the measured numbers (modeled time, per-frame motions, tile uploads), and a review recipe that starts from the tester's own complaint: the facing job at 1× must stop looking like a slideshow |
| Probe fields | `simulation: { clock, prefix, fraction, elapsedSeconds, totalSeconds, feedMmMin, tool, holderModeled, warnings }` beside the existing `stockPrefix` / `requestedStock` / `stockTransferBytes` |

---

## 11. Kept deferrals

Unchanged by this plan: arbitrary section planes, detaching stock, material
appearance, insert face-mill catalogues, native G2/G3, physical detached-part
simulation, and simulation of the M6 macro's interior. S1–S3 add time, a
cutter, an assembly and two checks; they do not turn the display raster into a
verifier, and they do not make the simulated stock an input to planning.

---

## 12. Questions for the tester

**Answered — Q1: where does the tool assembly live?** Shaft diameter and
stickout go into the tool library as optional parameters (copied into the job
with the rest of the geometry); the holder style goes into the machine
definition as an optional selection. Both are settled in D7 and grounded in
§2, where Fusion keeps the stickout on the tool and the holder as a separate
reusable object. Two consequences worth noting: the holder selection is one per
machine for now, so two tools on one machine share it — fine for a single
spindle, and a per-tool holder reference is the extension if that ever
changes; and the *tool's* cutter geometry stays authoritative for what the
simulation removes, while the shaft and holder are additional bodies that only
take part in the collision picture.

**Answered — Q2: how far should the collision warnings go in this slice?**
Display-only, settled in D10. Each warning names the motion, the depth and the
raster it came from; nothing refuses a job on the strength of a display
estimate. The `cam-core` check with diagnostic codes and an export gate stays in
S4, where it can be built on authoritative values.

**Answered — Q3: what rapid rate should the clock assume?** The machine
definition carries it, settled in D13. A machine with no stated rate still
plays — the readout names the assumption instead of presenting an invented
number as machine truth, and the value never reaches the plan or the G-code.

**Q4 — Should the reported total include tool-change and spin-up time?**

*Recommended:* no — report modeled motion time, and show tool changes as
untimed boundaries, because the macro's motion is unknown and the only known
value is the spin-up dwell.
