# Arc and small-move investigation (drag knife and flat V-carve)

Status: investigation, 2026-09-15, engine 0.7.7. No production code was
changed. This answers "is there room for improvement, can movements be
approximated with arcs, and can arcs from the SVG be carried along", and it
supplies the baseline numbers W7 in
[the field-testing fixes plan](field-testing-fixes-plan.md) asks for
(`§7 W7`, "Drag knife motion quality") and the data for its open question 5
("may smoothing move the path by a stated tolerance, or must geometry be
preserved exactly and only the representation change (arcs instead of
chords)?").

Measured inputs are the user's own saved jobs:

| Job | File | Operation |
| --- | --- | --- |
| "slitherin" lettering outlines | `real_data/knife` | drag knife, 17 closed chains |
| flower box outlines | `real_data/knife_flower` | drag knife, 15 closed chains |
| flower box | `real_data/flower_box-svg.job-real.json` | flat V-carve, endmill rough + V-bit finish |

Helpers and the exported programs used here live in
`flat-v-carve/artifacts/investigation/` (git-ignored), with the commands in
[Reproducing](#9-reproducing).

## 1. How big is the problem

Exported as the operator would run it (machine profile `decimal_places = 3`,
millimetre/absolute program):

| Program | Moves | Cut path | Median move | Moves < 0.01 mm | Moves < 0.05 mm | Moves per mm | Path mode |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| knife, slitherin | 23 581 | 2 410 mm | 0.017 mm | 48 % | 55 % | 9.8 | **G61 exact path** |
| knife, flower box | 37 008 | 2 109 mm | 0.018 mm | 48 % | 56 % | 17.5 | **G61 exact path** |
| V-carve rough, endmill | 9 617 | 1 186 mm | 0.095 mm | 0.1 % | 10 % | 8.1 | G64 P0.05 Q0.05 |
| V-carve finish, V-bit | 25 096 | 2 544 mm | 0.061 mm | 23 % | 46 % | 9.9 | G64 P0.05 Q0.05 |

and with option 4's arc fit at 0.005 mm, on the same flower job:

| Program | Moves | Of them arcs | Median move | Feed path |
| --- | ---: | ---: | ---: | ---: |
| V-carve rough, endmill | **667** | 519 | **1.72 mm** | 1 186 mm (unchanged) |
| V-carve finish, V-bit | **17 113** | 1 306 | 0.024 mm | 2 544 mm (unchanged) |

Both knife programs are about 10 to 18 programmed moves per millimetre of cut,
at a median move of 0.017 mm — roughly one millisecond of commanded motion at
the configured 1000 mm/min. The knife replay gate passed with a measured
maximum tip deviation of 0.0039 mm against a 0.01 mm budget, so nothing here is
buying accuracy: there is 0.006 mm of unused budget while the machine is asked
to stop at every one of 23 581 endpoints.

With options 1, 2, 6 and 7 implemented (see §7), the same slitherin export is:

| | before | after |
| --- | ---: | ---: |
| programmed moves | 23 581 | **8 125** |
| median move | 0.017 mm | **0.050 mm** |
| moves under 0.01 mm | 48 % | 41 % |
| moves per mm of cut | 9.8 | **3.2** |
| knife stage path control | `G61` exact path | **`G64 P0.003 Q0.003`**, bounded by the unused tip budget |
| cut length | 2 409.6 mm | 2 409.4 mm |

and the tip replay still reports the same 0.0039 mm of a 0.01 mm budget.

## 2. Where the moves come from

**Not from the swivels.** In the first 20 000 plan motions of the slitherin
job: 19 819 `knife_cut`, 118 `knife_swivel`, 15 `knife_align`. The corner
swivels (radius `d` = 0.25 mm, chorded within 0.0025 mm) are already coarse —
median 0.059 mm — and they are 0.6 % of the program. The micro-moves are the
*continuous compensated cut*.

**Two multipliers, in series.**

1. *Import flattening.* `svg/path.rs::Flattener` turns every cubic and every
   elliptical arc into a polyline at the artwork tolerance (0.001 mm here).
   The slitherin artwork is one path of 34 cubic segments; its filled boundary
   becomes **13 876 vertices**, and the 17 closed rings total 2 363 mm (mean
   segment 0.170 mm). The derived "knife outlines" item the knife operation
   actually selects is a copy of exactly those vertices.
2. *Blade compensation.* For a turn `Δθ` between two tip segments the planner
   inserts a pivot step of `2·d·sin(Δθ/2) ≈ d·Δθ` (plan §12.1). Its ratio to
   the tip segment that follows is `d/ρ` for local radius `ρ`. Lettering has
   curvature radii of a few millimetres against `d` = 0.25 mm, so **each tip
   vertex worth 0.16 mm of drawing becomes a pivot step near 0.016 mm** — a
   10× subdivision — while adding only 1.9 % to the path length.

Replaying that construction over the same outline (helper
`simulate-knife.mjs`) reproduces the exported cut length to 0.07 %
(2 407.8 mm predicted, 2 409.6 mm exported) and predicts 27.8 k pivot moves
against the planner's 23.6 k, so the proportions below are trustworthy even
though the replay over-counts by ~18 % (it does not reproduce every planner
skip rule).

| Simplification applied to the tip polyline | Tip vertices | Pivot moves | Moves < 0.01 mm | Moves < 0.05 mm |
| --- | ---: | ---: | ---: | ---: |
| none (today) | 13 876 | 27 816 | 49 % | 56 % |
| 0.001 mm | 5 977 | 12 089 | 45 % | 52 % |
| 0.002 mm | 4 261 | 8 694 | 43 % | 51 % |
| 0.005 mm | 2 644 | 5 505 | 37 % | 48 % |
| 0.010 mm (= the job motion tolerance) | 1 910 | 4 058 | 30 % | 46 % |

The same computation on the flower-box outlines (22 039 vertices)
gives 44 255 → 8 620 moves at 0.005 mm. Simplifying the *tip* polyline is the
cheapest large win, and it is exactly what W7 step 3 already proposes.

Implemented as `path_simplification_mm` on the drag-knife operation
(`None` = the planner's declared share of the motion tolerance, `0` = follow
the resolved polyline exactly). Measured on the slitherin job:

| `path_simplification_mm` | tip vertices | programmed moves | merge deviation |
| --- | ---: | ---: | ---: |
| 0 (off) | 11 655 | 23 578 | 0 |
| unset (0.0025 = tolerance/4) | 2 740 | **8 122** | 0.0025 mm |
| 0.005 | 2 447 | 6 048 | 0.0050 mm |
| 0.010 (the whole motion tolerance) | 1 888 | 4 392 | 0.0100 mm |

The merge is a Douglas-Peucker pass *between* anchors — the chain start, the
open end or ring seam, and every turn at or above the corner threshold — so a
swivel is never merged away. Every removed vertex is then re-measured against
the segment that replaced it; a merge that moved one further than the declared
tolerance fails the operation with `KNIFE_SIMPLIFY_BOUND` instead of emitting a
tip path the plan does not claim. (A first, greedy implementation was caught by
exactly that measurement, having moved one vertex 0.0059 mm inside a 0.0025 mm
budget; the pass was replaced rather than the check.)

**The V-carve is the same shape of problem.** 94 % of the endmill roughing
moves are turns below 2° — a nearly straight path emitted as 9 617 chords. The
V-bit finish has 23 % of moves below 0.01 mm (p10 = 0.0042 mm, minimum
0.000014 mm); 5 848 of its in-page 10 067 motions sit at one constant depth
(a pure 2-D offset of the same flattened boundary) and the rest are the
tapered medial branches, whose adaptive subdivision spends
`motion_tolerance · slope / 8` of chord error per interval.

## 3. How the import actually spends its tolerance

Measured directly, by rebuilding the import for the real artwork and comparing
counts against the exact curve (`flatten-candidates.mjs`, `vertex-budget.mjs`,
and the `contour_density` test):

* The flattener runs at **a quarter of the artwork tolerance**
  (`svg/mod.rs`: `geometry_tolerance_mm / 4`), and its cubic test — both
  control points within the tolerance of the chord — is within **15 %** of the
  best achievable: 7 155 vertices against 6 202 for a criterion that measures
  the true curve-to-chord distance, on the same 35-cubic path. There is
  nothing worth taking out of `Flattener::cubic`.
* `Flattener::ellipse` (every `<circle>`, `<ellipse>` and `A/a` arc) bounded
  the curve's second derivative by `|u| + |v|` where the exact maximum is the
  largest singular value of `[u v]` — up to twice as large. A 15 mm circle at
  a quarter-tolerance of 0.001 mm flattened to **770** vertices with the loose
  bound and **545** with the exact one, at the same declared accuracy
  (measured chord error 0.00031 mm against the 0.001 mm tolerance).
* Filled region rings come back with exactly the flattened vertex count — the
  fill pass does not re-sample (a circle's chain and its ring are both 545
  points). The earlier "3.4× overspend" reading in this document was an
  artefact of comparing against the *full* tolerance instead of the quarter
  the flattener is given; it is withdrawn.
* `ContourCatalogue::build` spent the merge budget on centreline chains only.
  Closed rings kept the flattener's dyadic overshoot: applying the same
  quarter-tolerance merge to them removes **17 %** of the flower artwork's
  boundary vertices (22 039 → 18 277) and 29 % of a synthetic blob's, with
  corners and straight edges intact (the `contour_density` test pins both).

## 4. Arcs: where they die, and what would be exact

Nothing in the toolpath is an arc today:

* `svg/path.rs` flattens `A/a` arcs *and* `<circle>`/`<ellipse>` through
  `Flattener::ellipse` into points; `ChainPoints` is a `Vec<Point>` and the
  arc centre, radii and sweep are discarded.
* The geometry engine is an integer-grid polygon engine; the V-carve plan's
  coordinates land on a 6.25e-6 mm grid. Offsets, unions and the medial axis
  consume and emit polygons.
* `toolpath::Interpolation` has only `Rapid` and `LinearFeed`, so a plan cannot
  express an arc; the post writer emits `G0`/`G1` only, the numeric reader
  compares decoded motions to planned motions 1:1 *including* interpolation,
  and stock verification builds straight capsule sweeps from
  `Motion::at_depth`.

The user's intuition is right, and it is stronger than "approximation":

**Both operations emit an offset curve of the artwork, and offsets of lines and
circles are lines and circles.**

| Path | Mapping |
| --- | --- |
| milling offset | tip arc of radius `ρ` → arc of radius `ρ ± R` (tool radius) about the same centre; line → line |
| drag-knife compensation | tip arc of radius `ρ` → arc of radius `√(ρ² + d²)` about the same centre, phase-shifted by `atan2(d, ρ)`; line → line |
| drag-knife corner swivel | already a radius-`d` circle centred on the corner (`PathElement::Arc`), currently chorded at ≤ 16.2° |

For the knife, `√(ρ² + d²)` and the constant phase shift are exact, not a
fit: the holder's heading and angular rate are unchanged, so **a circular tip
arc is exactly one `G2/G3`**, and a corner swivel is exactly one more. An
arc-preserving knife pipeline would spend *no* tolerance and would delete the
entire 2× subdivision of §2.

Two caveats worth writing down while the option is still open:

* Both real jobs here contain **no arcs at all** — they are pure cubic
  Béziers (37 cubics for slitherin, 37 for the flower). Carrying source arcs
  end-to-end pays only for artwork that has them; for Bézier artwork the win
  has to come from fitting.
* Flat V-carve at one depth is an offset, so the mapping above is exact there
  too, but the *tapered* medial branches are not an offset of the artwork:
  the branch is a quadratic `Curve` and its depth varies non-linearly. Arcs
  there are a bounded approximation, and the depth/stock consequence has to
  be re-verified.

Cost: `docs/flat-v-carve/2.5d-cam-plan.md` §8.4 and §14 already scope this as
"support native G2/G3 only in a later change that updates the writer, readback,
stock checks, simulation and tolerances together", and W7 calls it "the larger
half of the workstream". A representation change must land in the *plan*, not
in the writer, because readback compares the program to the plan 1:1.

## 5. What fitting buys, measured

Greedy line/arc fitting with every emitted primitive re-verified against every
point it covers (`fit-arcs.mjs`):

| Path | Input moves | Tolerance | Result | Reduction |
| --- | ---: | ---: | --- | ---: |
| slitherin tip polyline | 13 859 | 0.005 mm | 1 959 lines + 45 arcs | 6.9× |
| slitherin pivot path (exported) | 23 396 | 0.005 mm | 3 453 lines + 375 arcs | 6.1× |
| slitherin pivot path (exported) | 23 396 | 0.002 mm | 8 672 lines + 490 arcs | 2.6× |

Two things to read from this. First, a single tolerance-bounded pass over the
motion stream removes 5-7× of the moves without touching any planner
decision — and that pass is easier than a full arc-capable pipeline. Second,
the fitter prefers *lines* (it merges the shallow-curvature runs that the
flattener over-densified); a proper arc/line fitter with tangent continuity
would do better still, so these are lower bounds.

## 6. Two machine-level findings that change the priority

**The knife stopped at every vertex.** `post/sequence.rs` forced
`PathControl::ExactPath` for every knife stage regardless of the profile, and
the emitted stage re-established `G61`. The project had already measured what
that does — the [M6 capability report](m6-capability-report.md) records
"G61 stopped the machine at every micro segment ... exact-path mode
decelerates to zero at each vertex, so the machine never reaches feed and runs
jerky" — and at a 0.017 mm median move no machine reaches its configured feed,
so the jerky motion the tester reported is fully explained.

Implemented as W7 step 2 prescribes. The planner now publishes a
`BlendWithin { tolerance_mm }` intent only when its own emitted-program replay
left verified headroom, taking half of what is left (the tip is derived from
the pivot, so a pivot deviation can move it further than itself). The post
resolves the profile's path control inside that bound, quantized down to the
profile's precision so the prepared state is exactly the number the program
carries. An intent with no verified bound, and an exact-path profile, both stay
`G61`. The slitherin knife now emits `G64 P0.003 Q0.003` — the exported
deviation is a third of the motion tolerance, and the report says so
(`EXPORT_KNIFE_BLENDED_PATH`).

**V-carve already has the opposite problem.** Milling stages take the
profile's `G64 P0.05 Q0.05`, and export only requires the blend tolerance to
stay within the job's *verification* tolerance (0.05 mm) — five times the
job's 0.01 mm motion tolerance. So today's V-carve program is simultaneously
the densest path the planner can produce (0.000014 mm moves) and one the
controller is licensed to round by 0.05 mm. The 25 k-point finish path buys
fidelity the program tells the machine it may throw away.

**Precision escalation is now reported.** The profile declares
`decimal_places: 3`; the knife export was written with 4 decimals and the
V-carve export with **5**, purely to preserve moves finer than the machine's
declared resolution (`motions_preserved` in `post/sequence.rs`).
Every export now carries a `motionProfile` — per-stage move counts, the length
histogram, moves per millimetre and the profile and written precision — and
raises `EXPORT_PRECISION_ESCALATED` when the writer needed more precision than
the machine asked for, `EXPORT_MICRO_MOVE_DENSITY` for a stage dominated by
moves under 0.05 mm, and `EXPORT_EXACT_PATH_MICRO_MOVES` when such a stage also
runs exact path (the combination that stopped the machine). These are
observations, never gates; the CLI prints them after "exported N file(s)", and
they are in `report.json`.

**Native arcs are in the plan (option 3).** `Interpolation::ArcFeed` carries a
centre and a direction; the writer emits `G2`/`G3` with the centre as `I`/`J`
offsets from the start point, the numeric reader reconstructs the same arc and
compares the centre within the output grid (an offset sum does not land on the
plotted double exactly), and the knife replay integrates the no-slip model
along the arc's own tangent instead of a constant chord direction. The
simulator walks arcs as chords bounded by 0.002 mm because it animates a
straight-line envelope. Two gates keep it honest: `checks` refuses a
programmable arc whose endpoint radii disagree by more than 0.001 mm, whose
endpoints coincide, or whose sweep is empty, and it refuses arc motion outside
a knife stage with `PLAN_ARC_UNSUPPORTED` until the removal model sweeps an arc
rather than a segment (option 4).

Measured on both real jobs:

| | motions | of them arcs | replayed tip deviation |
| --- | ---: | ---: | ---: |
| slitherin, before | 8 122 | 0 | 0.0039 mm |
| slitherin, now | 7 887 | **3 793** | **0.00009 mm** |
| flower box, before | 12 656 | 0 | — |
| flower box, now | 12 289 | **5 978** | 0.0018 mm |

**Milling arcs are fitted within a declared tolerance (option 4).** A new job
tolerance, `arc_fit_tolerance_mm` (tolerances tab), lets the planner rewrite a
milling stage's polyline into the fewest lines and arcs that stay inside that
tolerance of it. The pass runs on the *plan* (the readback compares program and
plan one-to-one, so a representation change must exist there first), never
across a semantic breakpoint — same stage, tool, purpose, effect, feed, layer,
pass and contour, an unbroken chain of endpoints, and only the rough/finish
purposes — and it measures every primitive it emits against every vertex *and
every covered chord's midpoint*, in XY and in Z, before emitting it. A
primitive that wraps more than half a turn is refused: past that the same three
points describe the other arc too, and a wrapped primitive is not a local fit.
The resolved tolerance, the motion counts and the measured deviation travel
with the plan as an `arc_fit` named output, and the export report counts the
arcs per stage (`arcFeedMotions`).

Measured on the flower V-carve at 0.005 mm: **34 707 → 17 780 motions**, 1 825
of them arcs, feed path unchanged at 3 730 mm (the fitted stream's own length
is within 0.01 % of the polyline's), and an independent measurement of the
fitted path against the resolved polyline gives 0.0014 mm — inside the declared
0.005 mm. The rough endmill stage collapses 14.4× (9 614 → 667 moves, median
move 0.095 → 1.72 mm); the V-bit finish stage collapses 1.5× because its
medial fan is genuinely short-branched and tightly curved at that tolerance.
The knife jobs are untouched — their stages are never fitted, and their
execution digests are byte-identical before and after this change.

## 7. Options

| # | Change | Geometry effect | Risk | Expected effect (measured) |
| --- | --- | --- | --- | --- |
| 1 | Simplify the tip polyline to a declared tolerance before compensation (W7 step 3, first half) | bounded by the declared tolerance, spent from the motion budget | medium: needs replay evidence and an explicit setting (W7 step 4) | **done**: slitherin 23 578 → 8 122 moves at the default, 6 048 at 0.005 mm, 4 392 at 0.01 mm; cut length unchanged |
| 2 | Spend the tolerance instead of overshooting (exact elliptical-arc curvature bound; the chain merge budget applied to closed rings too) | none — same declared 0.001 mm chord bound | low, local to `svg/path.rs` and `contours.rs` | **done**: circles 770 → 545 vertices (29 %), flower contours 22 039 → 18 277 (17 %); cubics are already within 15 % of optimal |
| 3 | Emit `G2/G3` for arcs the planner already models (knife corner/alignment swivels) | none — exact | medium: touches writer, reader, verification, simulation | **done**: slitherin 8 122 → 7 887 moves with 3 793 of them arcs, and the replayed tip deviation falls from 0.0039 mm to **0.00009 mm** because the chord error is gone |
| 4 | Bounded arc/line fitting of the motion stream, emitted as `G2/G3` | bounded by the declared `arc_fit_tolerance_mm` | high: new plan interpolation, writer, reader, stock sweeps, simulation | **done**: flower V-carve 34 707 → 17 780 moves (rough 14.4×, finish 1.5×), 1 825 arcs, path length unchanged, independent deviation 0.0014 mm inside a 0.005 mm tolerance |
| 5 | Carry source arcs/curves end-to-end through import, offsetting and the plan | none where the artwork has arcs | highest: replaces the flatten-then-polygon pipeline | removes the whole 2× compensation subdivision for arc artwork; no benefit for the two Bézier jobs here |
| 6 | Let the knife stage use a verified blend tolerance instead of forced `G61` (W7 step 2) | changes corner geometry by up to P | medium: needs verification that blending cannot move the tip | **done**: the slitherin knife runs `G64 P0.003` instead of stopping at 8 122 endpoints, bounded by half the replay's unused tip budget |
| 7 | Report per-stage moves, move-length histogram, moves/mm and any precision escalation as export findings | none | low | **done**: `motionProfile` plus `EXPORT_PRECISION_ESCALATED`, `EXPORT_MICRO_MOVE_DENSITY`, `EXPORT_EXACT_PATH_MICRO_MOVES`, `EXPORT_KNIFE_BLENDED_PATH` |

## 8. Recommendation

Implementation status (2026-09-16): **options 1, 2, 3, 4, 6 and 7 are
implemented and verified; option 5 is out of scope.** Evidence: the workspace
test suite, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo fmt --all -- --check`, and the three real jobs planned and exported end
to end (`flat-v-carve/artifacts/investigation/final-*`), with the two knife
digests byte-identical before and after the milling work.

What each option now is, in one line each:

1. **Knife tip simplification** — `path_simplification_mm`, a bounded
   Douglas-Peucker merge between anchors, verified after the fact;
   slitherin 23 578 → 8 122 moves.
2. **Tolerance spending** — exact elliptical-arc curvature bound and the same
   merge budget for closed rings; circles 770 → 545 vertices, flower contours
   −17 %.
3. **Native arcs in the plan** — `Interpolation::ArcFeed` through the writer,
   reader, checks, replay, evidence, motion profile and simulator; the knife
   emits one arc per swivel, 3 793 of its 7 887 moves, and its replayed tip
   deviation falls to 0.00009 mm.
4. **Milling arc fitting** — `arc_fit_tolerance_mm`, measured per primitive
   over vertices and chord midpoints; flower rough 9 614 → 667 moves.
5. **Out of scope** (carrying source arcs through import and offsetting).
6. **Verified knife blending** — `BlendWithin { tolerance_mm }` from the
   replay's unused headroom; the knife runs `G64 P0.003` instead of `G61`.
7. **Export motion profile** — per-stage counts, histogram, moves/mm, arcs,
   precision, and the four observations.

Nothing in the option list remains open. Option 4's last pieces, for the
record:

* `PLAN_ARC_UNSUPPORTED` is gone: `checks` now accepts arc motion in any stage
  and keeps only the geometry gate (endpoint radii within
  `ARC_RADIUS_TOLERANCE_MM`, distinct endpoints, a real sweep).
* The fitter refuses a primitive that wraps more than half a turn, and measures
  the polyline's chord midpoints as well as its vertices — both were caught by
  measurement, not by review: the first version left hairpin arcs through
  loosely-sampled corners that its vertex-only test could not see.
* A fitted milling arc that descends is authorized exactly like the straight
  descent it replaced: the entry check's question is about the *tool's* ability
  to enter material while moving, not about how the move is programmed.

The one thing worth doing next, outside this objective, is raising the V-carve
finish stage's fit tolerance toward the motion tolerance (0.005 → 0.01) or
simplifying that stage's medial fan at the source: at 0.005 mm the finish stage
still emits 17 113 moves, 66 % of them under 0.05 mm, because its short
tightly-curved branches cannot be merged further without spending more of the
declared tolerance.

Option 6 was done after option 1 rather than instead of it — the reason `G61`
was forced is that the path had thousands of corners, so the honest fix was to
stop emitting thousands of corners *and* to stop the controller at the ones
that remain.

For the open question in the field-testing plan, this investigation supports
the "representation only" answer for arcs (options 3 and 5 are exact) *and*
the bounded-smoothing answer for everything else (options 1 and 4), provided
the tolerance is an explicit, visible setting, the flat V-carve tapered
branches are re-verified, and the existing knife replay gate is the acceptance
test for the pivot path.

## 9. Reproducing

Rebuild the CLI first: with a stale `target/release/cam.exe`, the derived
"knife outlines" artwork is rejected by an import rule the current source no
longer has, and both knife jobs plan to zero motions.

```powershell
cd flat-v-carve
cargo build --release --locked -p cam-app
./target/release/cam.exe collection inspect ..\real_data\knife --output artifacts\investigation\knife-inspect.json
./target/release/cam.exe collection apply-machine ..\real_data\knife --profile artifacts\investigation\machine-knife.json --name printnc-knife --output artifacts\investigation\knife.mapped.json
./target/release/cam.exe collection plan artifacts\investigation\knife.mapped.json --output artifacts\investigation\knife-plan.json
./target/release/cam.exe collection export artifacts\investigation\knife.mapped.json --output artifacts\investigation\export-knife
node artifacts\investigation\analyze-gcode.mjs artifacts\investigation\export-knife\01-knife-T3.ngc
node artifacts\investigation\simulate-knife.mjs artifacts\investigation\artwork-knife\artwork-1-knife-outlines.svg 0.25
node artifacts\investigation\fit-arcs.mjs gcode artifacts\investigation\export-knife\01-knife-T3.ngc 0.005
node artifacts\investigation\flatten-efficiency.mjs artifacts\investigation\artwork-knife\artwork-1-knife-outlines.svg 0.001
```

`node artifacts\investigation\extract-artwork.mjs <job.json> <dir>` dumps the
embedded artwork, and the same helpers accept the flower-job programs under
`artifacts\investigation\export-flower-vcarve\`.

The implemented options have their own regression tests:

```powershell
cargo test --locked -p cam-core --test knife_simplification   # option 1
cargo test --locked -p cam-core --test knife_output           # option 6 (bounded blend, mutations)
cargo test --locked -p cam-core --test motion_profile         # option 7
cargo test --locked -p cam-core --test contour_density        # option 2
cargo test --locked -p cam-core --test arc_sweep              # option 4 prerequisites
cargo test --locked -p cam-core --test arc_fit                # option 4 fitting pass
```
