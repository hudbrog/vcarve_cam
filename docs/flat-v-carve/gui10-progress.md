# GUI10 — machine-time playback

Status: **S1, S2 and S3 landed and checked**; S4 (the authoritative check in
`cam-core`) is not started. This batch implements S1 of
[machine-simulation-plan.md](machine-simulation-plan.md) — timed, continuous
playback — the machine-definition rapid rate that decision **D13** requires, S2
— the cutter on screen — and S3 — the assembly and the two machine warnings.

Everything below is in the uncommitted worktree that starts from `0fe0a8d`
*Animate playback at a rate the transport controls*.

## What the user can now do

* Play the facing job as one continuous pass instead of a slide show. Its
  generated plan has **47 motions** and **151.1 s** of modeled motion time; the
  longest feed move (a 300.2 mm pass at 2400 mm/min) takes **7.505 s** and the
  shortest (a 14.8 mm stepover) takes **0.370 s**. Under the old transport every
  motion cost the same step, so the whole job was one 20-second pass at 1×.
* Read where the program is in machine terms: `Program time · 0:37 / 2:31
  modeled motion · 42% into motion 12 · feed 2400 mm/min`.
* Scrub the program by time, including inside a single long pass, and watch the
  cutter marker travel while the material is removed under it.
* Play long jobs at 60×, 300× or 1000×, or press **Fit** to map the whole
  program into a 20-second window at the same ratios.
* Set a machine's rapid rate in the machine definition (optional). A machine
  that states none still plays, and the readout says the total rests on a stated
  assumption instead of presenting it as machine truth.
* See the cutter itself: a body revolved from the tool's own geometry, sized to
  scale, travelling with the clock. The readout names the tool and stage it is
  in (`· tool endmill Flat V-carve`), and a drag knife's blade turns with the
  heading the plan modeled, so a corner swivel reads as a turn.
* State how the tool is held — shaft diameter and stickout — in the tool
  library, and name the holder the machine uses (ER11 … ER40, optional) in the
  machine definition. The viewport then draws the shaft and the holder standing
  on the stickout, to scale, moving with the tool.
* Be told when the assembly or a rapid would hit the job: the transport lists
  each warning with its motion and depth, on the raster the estimate rests on,
  and a **Show** button seeks to it.

## Contracts and implementation

**The motion stream carries machine execution** (`crates/cam-gui/src/sim.rs`,
`scene.rs`, `knife.rs`). `sim::Motion` gained `interpolation` and
`feed_mm_min`, and the wire record grew from 56 to 64 bytes — the pad byte
became the interpolation flag and one `f64` carries the feed. One mapping
(`scene::sim_motion`) turns a `PlannedMotion` into the display record for both
the milling and the knife scene, so the knife's cutting, plunge and swivel feeds
reach the clock too.

**Program time is derived from the plan** (`sim::TimeTable`). A linear feed
costs its length at its own `feed_mm_min`; a rapid costs its length at the
machine's rapid rate, or at `DEFAULT_RAPID_RATE_MM_MIN` when the machine states
none; a zero-length move costs nothing, because a per-segment floor would invent
minutes on a job made of micro-segments. The table converts between a program
time and `(prefix, fraction)` in both directions.

**A position is `(prefix, fraction)`** (`sim::Playback`). `Playback` now holds
the fraction of the in-flight move it has applied, so `advance_to` can move
inside one motion, `seek_position` can restore an exact state at a time, and
`Playback::seek` forces a restore when the field holds an unfinished move — a
raster cannot be subtracted, so a rewind replays from an exact state.

**One geometry frame per motion** (`sim::Field::apply`). The fractional window
used to be computed from the window's own endpoints, which re-derived the
covered interval per piece; cells exactly tangent to the swept envelope could
then flip between a one-shot application and a partitioned one, and the
animation's own frames disagreed with a cold replay. The window now restricts
the **whole motion's** covered interval, so a partition is exact: the same
`disc`, the same candidate set, intersected with the window. For a whole motion
(`0..1`) every expression is unchanged, which is what keeps the reference
parity comparison valid. `Stats` counts a motion when a window starts at the
beginning of that motion, so progressive frames count once.

**The display runs the clock** (`crates/cam-gui/src/sim_clock.rs`).
`LocalClock` seeds from the frames that travelled with the scene — the pristine
state at prefix 0 and the state the display opens on — and advances locally with
`Field::apply`. A frame that removes material returns the dirty tiles, and the
viewport patches those tiles of the raster it hands the renderer
(`sim::Field::pack_tile`) instead of re-packing the whole grid. A target behind
the display, or further than `LOCAL_ADVANCE_LIMIT` motions ahead, becomes one
exact seek against the retained execution rather than a long frame: the compute
process keeps authority over every position the display lands on.

**The transport is machine time** (`crates/cam-gui/src/viewport.rs`). The speed
buttons scale real time (1× is the machine's own time), **Fit** maps the program
into 20 seconds, a **Program time** slider sits beside the existing motion
slider, and the probe publishes `display.simulation` (`prefix`, `fraction`,
`elapsedSeconds`, `totalSeconds`, `tip`, `feedMmMin`, `rapidRateMmMin`,
`rapidRateAssumed`, `fastForward`, `fit`). The cutter marker interpolates along
the in-flight move and the overlay draws the part of the move already cut, so
the drawn path ends where the tool is.

**A plan whose timing cannot be derived still plays.** A linear feed with no
feed rate is refused by `TimeTable::build`, not by the scene: the display keeps
showing the plan, falls back to stepping whole motions, and says so in the
transport line. The plan's own checks already refuse such a plan for output.

**The rapid rate is machine data** (`cam-core`, decision D13).
`SequenceProfile` and `AppliedMachineConfiguration` gained an optional
`rapid_rate_mm_min`, the GUI's machine editor exposes it, and
`apply_machine_configuration` / `resolve_sequence_profile` carry it into the
job, which the scene publishes with the motion stream. It is never required: a
program does not need it (G0 carries no feed) and the export profile does not
carry it.

## S2 — the cutter on screen

**The display spec now describes the whole cutter** (`sim::ToolSpec`). An
endmill carries its `cutting_length_mm` and a knife its `max_cut_depth_mm`
(the V-bit already carried its cutting height), so the drawn body needs no
second source of geometry: `ToolSpec::profile` returns the revolved profile from
the tip upward, and `ToolSpec::radius_at_depth` returns the cutting radius at a
depth. Both are derived from `ToolSpec::normalize`, which is the same function
the removal uses, and both lengths come from the job's tool geometry through
`scene::sim_tool`.

**The body is a surface of revolution** (`overlay.rs`). `revolve` turns the
profile into quads between consecutive rings with an outlined silhouette and a
flat bottom face, so an endmill reads as a cylinder of its cutting length, a
V-bit as flanks up to its widest diameter plus its collar, and a truncated tip
still shows its flat face. The tool axis is drawn up to the stock top, so a
short cutter still reads as held.

**A knife is a blade, not a revolution.** `blade` stands a plate from the
cutting tip to the holder pivot along the modeled heading, `max_cut_depth` tall,
and marks the pivot with a ring. The heading comes from
`report["gui2"]["knifeMotions"]`, which the scene already publishes, and the
viewport interpolates it the short way round between the two ends of the move.

**D8 comes from the appearance toggle that already exists.** The overlay is
drawn before the stock and depth-tested against it: in the opaque appearance a
cutter inside the material is hidden, and in the x-ray appearance the stock
stops writing depth, so the body and the paths inside a cut stay visible. The
plan asked for a separate `tool.wgsl` pass; the overlay pass already draws 3D
triangles with the same camera uniform, so the outcome is the same with one
less pipeline to maintain.

**Not drawn yet:** anything above the cutting portion. The shaft diameter and
the stickout are S3's tool-library fields; until a tool states them, the axis
line is the honest representation of the rest of the tool assembly.

## S3 — the assembly, and the two machine warnings

**The assembly is tool data, not cutter geometry** (`cam_core::project::ToolAssembly`).
A job tool and a library tool each carry an optional `shaft_diameter_mm` and
`stickout_mm` (tip → holder bottom face, Fusion's *assembly gauge length*).
Capture copies them into the library, "use in operation" copies them into the
job, and the library's unchanged-snapshot comparison includes them, so editing a
tool's stickout in the library re-snapshots rather than silently reusing the old
record. Both values are optional and validated positive when present.

**The holder belongs to the machine** (`cam_core::post::holder`).
`SequenceProfile` and `AppliedMachineConfiguration` gained an optional
`holder: HolderSelection` — a catalogue id (`er11`, `er16`, `er20`, `er25`,
`er32`, `er40`) or the machine's own `segments`. `HolderSelection::body()`
resolves it into a stack of truncated cones, the same representation Fusion uses
for its holder and shaft blocks. The catalogue's **nut diameters are the
published DIN 6499 / Rego-Fix ER series sizes** (19, 22, 25, 32, 40, 50 mm); its
nut heights and chuck bodies are nominal and the module header says so, because
chuck bodies differ by manufacturer and a machine that needs exact geometry
carries its own segments. A name nobody knows resolves to nothing rather than to
an invented body.

**The display draws what the tool states, and nothing else.** The per-tool
assemblies and the machine's holder travel in `SimMeta`; the viewport resolves
the holder body once per scene and passes it into the marker. The shaft is drawn
from the cutter's top to the holder's bottom face, and each holder segment
stands on the stickout in turn. With no stickout there is no holder drawn at
all: nothing says where one would be, and the axis line is honest where a body
in the wrong place would not be.

**Two checks, run once with the execution** (`crates/cam-gui/src/sim_checks.rs`).
The pass walks the program in the worker at generation time over its own coarse
raster (0.4 mm, or coarser for a large plate) and publishes
`report["gui2"]["warnings"]` plus `warningCellMm`. Each motion is judged against
the material that is actually there when the machine starts it, so a rapid into
an already-cut trench is not a crash:

* **assembly below the surface** — for a feed move, each non-cutting body (the
  shaft from the flute top up, and every holder segment) is tested over its own
  footprint: the centre plus a ring at its radius, which is what catches a wall
  beside the tool;
* **rapid through material** — a lateral rapid or a descent whose path passes
  below the surface. A pure retract is exempt: it starts where the tool already
  is and leaves along its own path.

One warning per motion and kind, each with the motion index, the tool and how
far inside the material the part reaches. The transport lists them with the
raster named in the header and a **Show** button per row that seeks to the
motion through the same path as any other jump; the probe publishes the count,
the cell size and the first few entries. Nothing here refuses a job: the
`cam-core` check with diagnostic codes and an export gate is S4.

**One bug the tests found and fixed:** the shaft body was first checked at the
holder's underside rather than at the flute top, which is the lowest point a
shaft can touch material with. Caught by
`a_holder_that_would_run_into_the_job_is_reported` reporting 2 mm instead of the
4 mm the geometry predicted.

**Deliberate omissions in this slice.** The machine form offers the catalogue
series and shows a machine's own segments read-only (a segment editor is a
library-file job for now). The assembly is edited in the **tool library**, not
the operation inspector, so `state.rs::FIELDS` and `help.rs` are untouched. The
plan also asked for timeline markers per warning; the list with its **Show**
seek is what landed, because egui's slider has no marker API and a custom track
is a review-recipe-sized change of its own.

## Verification

`cargo clippy --workspace --all-targets --locked -- -D warnings` is clean, and
`cargo test --locked --workspace` passes every target except the one
pre-existing failure recorded below.

| Evidence | What it proves |
|---|---|
| `sim::tests::a_feed_move_is_timed_by_its_feed_and_a_rapid_by_the_machine_rate` | 300 mm at 2400 mm/min is 7.5 s, 14 mm at 600 mm/min is 1.4 s, a rapid follows the machine rate and not the feeds, and a zero-length move costs nothing |
| `sim::tests::a_partitioned_motion_removes_the_same_material_as_one_shot` | a flat pass, a dive and a diagonal ramp, for an endmill and a truncated V-bit: ten pieces equal one shot in cell bytes, stage/tool identity, removed volume, and motion count |
| `sim::tests::advancing_into_a_motion_matches_a_cold_replay` | the animated field equals a cold replay at `(prefix, fraction)`; a backward target asks for a restore; a long jump asks for an exact seek |
| `sim::tests::the_time_table_moves_between_time_and_position` | position → time → position round trips, zero-length moves are skipped, and a position never carries a fraction of exactly one |
| `sim::tests::a_feed_without_a_rate_is_refused_by_the_clock_not_by_the_display` | the stream still decodes and the clock refuses to invent a feed; a machine with no rapid rate uses the stated fallback and says so |
| `sim_clock::tests` | an advance inside a move moves the tip and returns seconds; a target behind the clock asks for a restore; the advanced frame equals a cold replay; a raster patch touches only the tiles that changed |
| `scene_frame::the_facing_job_is_timed_by_its_own_feeds` | the tester's own job: 2400 mm/min reaches the stream, rapids carry none, the longest feed move takes more than 3× the shortest (measured 7.505 s against 0.370 s), the total is the program's own 151.1 s rather than a fixed playback window, and every motion is timed by its own length and rate |
| `knife::` (the GUI6 knife tour) | a knife stage is timeable: its 150, 50 and 75 mm/min feeds reach the stream and the table builds |
| `overlay::tests::a_cutter_body_reaches_exactly_as_far_as_the_removal` | **S2/D6**: for an endmill and a truncated V-bit at 0.5, 1.4 and 3.0 mm, a single plunge's removed footprint equals the profile's radius at that depth within one cell, and every profile ring agrees with the cutting envelope at its own height |
| `overlay::tests::the_knife_blade_stands_along_the_modeled_heading` | the blade reaches `blade_offset` along +X at heading 0 and along +Y at heading 90°, and stands `max_cut_depth` above the tip |
| `overlay::tests::markers_draw_the_declared_cutter_bodies` | a body is drawn for both mill kinds, the axis reaches the stock top, and an endmill's body stops at its cutting length |
| `overlay::tests::the_assembly_is_drawn_only_where_the_tool_states_it` | the holder's bottom face lands on the stickout, its widest ring is the catalogue nut, and neither shaft nor holder is drawn when the tool states no stickout |
| `cam_core::post::holder::tests` | every catalogue name resolves to a body whose nut is the published diameter, no body is narrower than its own nut, and a machine's own segments are used instead when it carries them |
| `sim_checks::tests` | a holder inside the job is reported with its motion and depth; an unstated assembly is not checked; a rapid at clearance height is clean and the same rapid at depth is reported; a rapid inside an already-cut trench is not; a retract is exempt while a lateral move from the same place is not |
| `scene_frame::a_holder_that_would_hit_the_job_is_reported_with_the_execution` | end to end through the worker: the gui4 lettering carving with a 0.8 mm stickout and an ER20 holder publishes an assembly warning whose deepest report is 0.7 mm, and the same job at a 40 mm stickout publishes none |
| `knife::` (the GUI6 knife tour) | the knife scene publishes its warnings too, and this fixture's are empty |

### Browser scenario

`crates/cam-gui/web/gui10-scenario.mjs` drives the shipped browser build
(`node crates/cam-gui/web/smoke.mjs --gui10`, with `pkg/` built and
`node web/serve.mjs` running) through both halves of this batch. It drops the
tester's own `real_data/facing_job.json`, generates, and reads the display's own
probe; then it drops `fixtures/gui10/tight-holder.job.json` and checks the
assembly warning. Recorded in `artifacts/gui/browser-smoke/<timestamp>/`
alongside two screenshots:

| Step | Recorded |
|---|---|
| a machine clock | 63 motions, **164.03 s** of modeled motion, rapid rate 5000 mm/min *assumed* (the job states none) |
| 1× is continuous | eight samples inside **one** pass: fraction 0.084 → 0.510, program time 0.58 s → 3.14 s, feed 2400 mm/min, 3.14 s of program over 2.8 s of wall clock |
| Fit | 8.202× — 164.03 s / 20 s, the program in one window at its own ratios |
| an unstated assembly | `{shaftDiameterMm: null, stickoutMm: null}` and no warnings: the display does not guess |
| the holder warning | ER20 nut **0.7 mm** inside the material from motion 83, on a **0.4 mm** raster, one row |
| a warning seeks | the row's **Show** puts the display at motion 83, 4.27 s into the program |
| walls move with the floor | six samples inside **one** pass (fraction 0.072 → 0.324) carry six different wall revisions (261 → 368) and 143 → 158 → 156 wall instances |

### A report from testing: the walls stood still inside a motion

The tester's own words: on the facing job with the 50 mm cutter, "I can see how
the material is being removed during a single step, but the walls were being
left in place until the end of the move".

The cause was the wall cache key. Walls are derived display geometry: the stock
pass draws floors straight from the cell bytes, while `stock_walls` builds the
vertical faces once and caches them against `(identity, prefix, threshold,
section)`. `prefix` is the *motion index*, and the animation now advances
**inside** a motion — so the floor moved every frame while the walls waited for
the next boundary. It is the one piece of the display that was still keyed on
the old step clock.

`StockView` now carries a `raster_revision`, bumped wherever the displayed bytes
change (a local advance with dirty tiles, a transported seek, a preset change),
and the wall cache keys on that instead of on the prefix. The browser scenario
pins it: two samples inside the same motion must not share a wall revision, and
the recorded run shows six revisions across one pass. `display.walls` publishes
the revision, the instance count, the dropped-step count and the threshold, so a
review can see the derived geometry keep up with the floor.

Cost, stated plainly: the walls are rebuilt in any frame that removes material,
because the whole set is derived from the raster. On this job that is a few
hundred instances; on a fine-raster, hundred-thousand-motion carving it is a
full-grid sweep per cutting frame. Incremental wall updates (only the cells
inside dirty tiles) are the follow-up if a measurement shows it matters.

The scenario earned its keep: it found two defects no unit test could.

1. **A seek dropped the clock.** A seek response carries cells, not a motion
   stream, and the reseed treated that as "no clock" — so after any scrub, stage
   jump or `Start` the transport fell back to stepping whole motions and showed
   "Program time · unavailable". The reseed now takes the tool geometry from the
   scene that is already displayed. This affected every historical playback
   path, not just the new one.
2. **The assembly readout was empty at the end of the program**, because there
   is no in-flight move there to read a tool from; `LocalClock::tool` now falls
   back to the last completed move.

It also changed the warnings list for the better: the first version reported one
row per motion, so a holder that is inside the material for forty consecutive
passes became forty near-identical rows and the **first six** (the shallowest)
were all the panel showed. A row is now one **problem**: the first motion that
shows it, the depth there, and the worst depth the program reaches
(`depthMm` / `maxDepthMm`).

### State that was already broken before this batch, and is now closed

* `scene_frame::the_scene_publishes_the_facing_request_coverage_and_entry`
  failed in the working tree because `real_data/facing_job.json` had changed
  under it (margins of 20 mm and a 50 mm entry overrun against a test that
  expects the coverage to be the stock rectangle). The tester asked for the
  fixture to be restored to the committed version, and the test passes again.
* `cargo fmt --all -- --check` failed at `HEAD` on
  `cam-core/src/checks.rs`, `cam-core/src/operations/face.rs`,
  `cam-core/tests/face_core.rs`, `cam-gui/src/face.rs`, `cam-gui/src/face_ui.rs`
  and one hunk of `cam-gui/src/scene.rs`. The tester asked for the formatter to
  be run across the workspace, and those files landed as their own commit
  (`Reformat the files rustfmt had drifted from`) so the feature diff stays
  readable. `cargo fmt --all -- --check` is clean.

## Not in this batch

S4 (the authoritative `cam-core` check with diagnostic codes and an export
gate), the visual check that the drawn tip sits on the last removed cell in
every view and preset, a holder segment editor, warning markers on the timeline
track, acceleration, and tool-change time. The browser scenario and its evidence
landed with S3. S1–S3 built the interfaces the rest needs: the time table, the
`(prefix, fraction)` position, the per-tile raster patch, the tool's own display
geometry, the assembly fields, and one place where the holder's numbers live.
