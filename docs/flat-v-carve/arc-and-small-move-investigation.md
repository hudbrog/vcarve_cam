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
| V-carve finish, V-bit | **2 893** | 1 626 | **0.85 mm** | 2 544 mm (unchanged) |

Nothing in either stage is now shorter than 0.05 mm (46 % of the finish
stage's moves were, before), and the program is written at the machine
profile's own three decimal places instead of the five the writer had to
escalate to.

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

### What actually limited the V-carve finish fan

The first fitted flower plan was 17 780 moves, and the finish stage was still
17 113 of them at a 0.024 mm median. Measuring the *run structure* — the
maximal stretches the fit is allowed to merge — found the reason, and it was
not geometry: `flat-v-carve` emits each V-bit *execution* (one medial branch or
boundary path) with its own `pass_id`, and the fit treated a change of `pass_id`
as a machining boundary. The finish stage's first 10 386 motions were **4 459
runs, median length 1**, and 4 078 of them descended (the medial fan's
tapered branches), with no gap between runs at all: the branches are
continuous in space, and only bookkeeping separated them.

`pass_id` is now a *label* rather than a break: the fit merges across it when
stage, tool, purpose, effect, feed, depth layer, contour and endpoint
continuity all agree, the emitted primitive carries the pass of its first
motion, and the plan's evidence ranges are recomputed against the motions that
actually carry each label (they addressed the planner's numbering, which a
rewrite invalidates). Result: 34 707 → **3 560** moves, the finish stage
8.7× smaller, feed path unchanged at 3 730 mm, measured deviation unchanged at
0.00499998 mm of a 0.005 mm budget.

Raising the tolerance is the wrong lever, and the measurement says so:

| `arc_fit_tolerance_mm` | Total moves | Finish moves | Median finish move | Measured deviation |
| --- | ---: | ---: | ---: | ---: |
| 0.005 (default) | **3 560** | 2 893 | 0.85 mm | 0.00499998 mm |
| 0.010 | 3 066 | 2 492 | 1.04 mm | 0.00999833 mm |

Doubling the deviation budget buys 14 %, because what remains is genuinely
short-branched geometry, not approximation slack. The same measurement also
shows the next real lever: of the finish stage's 2 893 moves, 663 are
clearance, approach and entry motions — one retract-and-re-enter cycle per
execution — so the fan's *transit* structure, not its sampling, is what is
left.

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

It is **per stage, by role, not per operation**: every enabled operation's
stages are fitted at plan assembly, so a Face pass, a Flat V-carve and a
Profile all get it, and only knife stages are exempt (their arcs are the
planner's own). A profile pass around a 40 mm circle goes 1 261 motions → 6,
two of them arcs; the `arc_fit` evidence is published per operation, so a job
with several milling operations reports each one separately.

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
| 4 | Bounded arc/line fitting of the motion stream, emitted as `G2/G3` | bounded by the declared `arc_fit_tolerance_mm` | high: new plan interpolation, writer, reader, stock sweeps, simulation | **done**: flower V-carve 34 707 → **3 560** moves (rough 14.4×, finish 8.7×), 2 145 arcs, path length unchanged, nothing left under 0.05 mm, written at the profile's own precision |
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

Two things are worth doing next, outside this objective:

* **The fan's transit structure.** After the pass-label fix the finish stage is
  2 893 moves, of which 663 are clearance, approach and entry — one
  retract-and-re-enter cycle per execution. That is the planner's link
  policy for the fan, not its sampling.
* **The medial sampling budget.** `vcarve/medial.rs` subdivides medial branches
  at `motion_tolerance * slope.min(1) / 8`, an eighth of the tolerance, which
  is eight times finer than the plan promises. The fit now collapses that
  density in the program, so the remaining cost is planner time and memory
  rather than emitted moves; loosening it would need the engine's own depth
  verification to show the headroom, which this investigation did not measure.

### Fan travel: ordering (options A and B of the follow-up)

Measured on the flower V-carve after the arc fit:

| measure | finish (V-bit) | rough (endmill) |
| --- | ---: | ---: |
| cutting excursions (lift → cut → retract) | 156 | 23 |
| cut length | 2 526 mm | 1 186 mm |
| air travel | 3 720 mm | 761 mm |
| transit between excursions | 1 764 mm (max 67.2) | 485 mm |
| plunges, summed Z | 279 mm | 46 mm |

**B (ordering) was measured and rejected, and is reverted.** The prototype
refined the walk the planner already computes — bounded 2-opt stretches,
single-path reversals, and every closed ring re-entered at the vertex nearest
where the tool arrives. It did work on the program: finish transit
**1 764 → 1 696 mm**, longest transition **67.2 → 60.3 mm**, cut length
unchanged at 2 524 mm. But a 68 mm saving on a 4 900 mm program costs
**0.4 s of plan time on every run** (7.9 → 8.3 s), for about 0.7 s of machine
time at the programmed rapid rate. That trade is not worth a permanent
increase in planning cost, so the ordering code is gone and only the
measurement stays. The rough stage was never in question: its 23 features are
about 22 mm apart and already visited once each, so its 485 mm is geography,
not order.

### Where the finish stage's remaining time actually goes

Modelled from `02-vcarve-finish-T2.ngc` at the programmed feeds (cut 1 600,
plunge 600 mm/min) with G0 at 5 000 mm/min:

| | released (full retract) | with the short lift below |
| --- | ---: | ---: |
| cutting | 2 576 mm / 97 s | 2 576 mm / 97 s |
| Z at plunge feed (approach + 2 mm plunge, 332 moves) | 1 110 mm / 111 s | 778 mm / 78 s |
| rapid (retract + transit) | 2 915 mm / 35 s | 2 583 mm / 31 s |
| total | **5 601 mm / 243 s** | **4 937 mm / 206 s** |

The up-and-down is not the XY transit: 156 cycles cost 1 111 mm of descent at
plunge feed and 35 s of rapid. Cutting is 101 s of the 247 s. That is why the
lift height, not the tour, is the first lever.

### D: what the 156 cycles are, and what an order could do

The follow-up asked whether the fan is walked feature by feature. Measured on
the surviving excursions of the flower finish stage:

| measure | value |
| --- | ---: |
| lift cycles | 156 |
| transit hops between them | 166 |
| hops inside one artwork component | 139 (1 290 mm) |
| hops between components | 27 (503 mm) |
| hop length | median 7.0 mm, p90 23.5 mm, max 67 mm |
| the same excursion ends, nearest-neighbour tour | 983 mm |
| the same excursion ends, 2-opt tour | **838 mm** |

The tour figures come from the same helper reading the same program, which
delimits an excursion slightly differently (170 rather than 156 — it does not
fold a surface contact point into the excursion it belongs to) and reports the
programmed transit as 1 793 mm. So the order is roughly **2× the optimal
tour**: a perfect walk would take the 1 764 mm down to about 840 mm, worth
~11 s at the programmed rapid rate — not
the up-and-down itself. The 7 mm median hop is real geography: the medial fan
of a petal is a tree whose branches meet at junctions but *end* near different
walls, so leaving a branch at its far end and entering the next one is a real
7–20 mm move unless the tool walks back along the branch it just cut. Retracing
that spine costs cut-feed time (1 600 mm/min) against a 0.5–0.9 s lift cycle,
so the trade only pays on short spines; deciding it per spine needs a walk over
the medial graph rather than a tour over branch ends.

One related experiment was measured and dropped: ordering only the paths that
survive air pruning (rather than every candidate, so that omitted paths cannot
act as waypoints) left the flower's transit unchanged (1 764 → 1 794 mm) and
removed the recorded proof for each pruned path, which is the audit trail
`analysis.pruned_air_paths` and `vcarve_planning` report. It is not worth the
lost evidence.

**A (feature-local emission order) is deferred into the option below.** Its
premise — that the tour is bad — did not survive measurement.
`vcarve/verify.rs` enforces `BOUNDARY_FINISH_ORDER`: once the final
boundary/detail family has run, nothing else may cut, and with one depth cap
that family *is* the whole detail sweep. So the artwork is swept once per
family, and that is what the 1 764 mm is: a global nearest-neighbour tour over
the same 170 excursions costs 983 mm only because it visits each feature
*once*, which the rule forbids. Sharing one feature sequence across both sweeps
instead made the total *worse* (1 764 → 1 804 mm), because each sweep is
already ordered best from its own start. The rule exists for the reason the
operator gives — a later pass over already-finished geometry can leave a mark —
so feature-local ordering belongs with the selectable behaviour below, behind a
separation proof (no floor pass within the V-bit's cone reach of another
feature's finished wall) rather than becoming the default.

### C: the selectable finish behaviour, as built

`vbit_planning.transit` chooses what the V-bit does between two cutting
excursions. Every mode keeps every cut; they differ only in how much clearance
the transit takes.

| mode | between two excursions |
| --- | --- |
| `retract` (released behaviour, still the default for a job that has no such field) | lift to the global clearance plane above the stock top |
| `short_lift` (default for a new combined carve) | lift to the same clearance measured from the pass depth |
| `route` | keep the bit down and cut across, while that is quicker than lifting |

A join below the stock top has to be argued for, and the argument is the shape
itself: wherever the V-bit's cone at pass depth still fits inside the target,
the cut it makes is one this stage already owes, because the target's own depth
there is at least that deep. The planner decides that with
`variable_radius_margin_mm` — the same continuous sweep bound the verifier
applies to every recorded cut — and only keeps the join while it is quicker than
the lift it replaces. Rapid moves are machine-side and not an input the planner
owns, so the comparison uses the two feeds it does know: the lift's descent at
the plunge feed against the join's travel at the cutting feed. That makes the
test conservative in one direction only — a join is taken only if it wins even
if the machine's rapids were instantaneous.

Two details the verifier had to learn. A lifted transit enters and leaves at a
height that is no longer the clearance plane, so `RapidXY`, `RapidRetract` and
`Approach` are bounded by the stock top below and the clearance plane above
instead of being pinned to the clearance plane. And an entry below the stock
top is a join, which is now exactly the case whose shape bound has to hold.

Measured on the flower, same cuts, same 156 cycles:

| | released | `short_lift` |
| --- | ---: | ---: |
| finish air travel (`z >= 0`) | 3 720 mm | **3 056 mm** |
| finish Z at plunge feed | 1 110 mm | **778 mm** |
| finish rapid | 2 915 mm | **2 583 mm** |
| finish total (modelled) | 243 s | **206 s** |
| plan time | 7.9 s | 7.9 s |

So the selectable behaviour is worth **37 s, 15 % of the finish stage**, at no
planning cost and with no change to a single cutting move. The plane it stops
at is never below the stock top: a pass deeper than the configured clearance
leaves no room above it, and the released clearance plane stands.

The first attempt at a join rule asked for something stronger: that the transit
remove *nothing*, proved by rasterizing the corridor the cone sweeps and
demonstrating that the rough-stage sweeps had already cleared every cell of it
(`StockQuery::covered_transit_depth`). On the flower that proof held for **0 of
166** connections, and it held at the first bisection step, which means the
straight line between two excursions crosses ground the rough stage never
touched: the lane ends sit in the leftovers a 3 mm endmill physically cannot
reach, and the fan's hops are chords across the pocket rather than paths along
its spine. Because every join that rule accepts is also a join the shape bound
accepts, and because `route` additionally accepts the joins that *do* cut shape
material, the stricter rule was redundant: **`route` supersedes it**, and it and
its corridor rasterizer were removed rather than shipped as a mode that does
nothing on real jobs.

What that gives up is one guarantee: no mode now promises that a transit cuts
nothing. `route`'s joins cut material this stage removes anyway, at pass depth,
on the shape's own terms — but they are cuts, and they engage the cone
sideways.

### D, answered: the transit does not have to be a straight line

The follow-up question was the right one. A lift costs the Z descent at the
plunge feed, so on this job a cycle is worth about
`6 mm / 600 mm·min⁻¹ = 0.6 s`, which is the same time as **16 mm of travel at
the 1 600 mm·min⁻¹ cutting feed**. A detour is therefore free as long as it is
under about 16 mm — and a chord that leaves the shape is not the only way to
get there.

Measured on the flower's own corner cluster (the five floor lanes around
`x 155…170, y 25…38`, motions ≈1733…1822 of the lagging program):

| | released | `route` |
| --- | ---: | ---: |
| lifts inside that one cluster | 15 | **6** |
| the five corner lanes themselves | 5 separate lifts | **cut as one run** |

Two things make that possible. First, a join straight through the middle is
allowed whenever the shape itself allows it: the V-bit's cone at pass depth has
to stay inside the target, which the same `variable_radius_margin_mm` bound the
verifier applies to every recorded cut decides. Where the cone fits, the cut is
one this stage already owes — the shape's own envelope there is at least that
deep — so a join changes the order of removal and nothing else. Second, where
the straight line cannot be used (a corner, or a chord that would leave the
shape), the planner tries waypoints either side of the line and along it, keeps
the shortest route whose every segment passes that same bound, and takes it
only if it is quicker than the lift it replaces. Otherwise the short lift
stands. `FinishTransit::Route` is that rule, and it is the only one this stage
needs for joins: every join the stricter "removes nothing" rule accepted, this
one accepts too.

Measured on the whole stage, same cuts otherwise:

| finish stage | flower (released → `route`) | flower_lagging (released → `route`) |
| --- | ---: | ---: |
| lift cycles (Z-feed moves) | 329 → **211** | 329 → **205** |
| excursions | 156 → **109** | 167 → **105** |
| Z at plunge feed | 1 110 → **486 mm** | 1 277 → **578 mm** |
| rapid | 2 915 → **2 015 mm** | 3 082 → **2 053 mm** |
| cutting | 2 576 → **2 946 mm** | 2 576 → **3 012 mm** |
| modelled stage time | 243 → **183 s (−24 %)** | 261 → **195 s (−25 %)** |

The join budget is what keeps the cycle count here at 211 rather than 191: joins
between 16 mm and ~21 mm are still wins on a real machine, because the lift's own
XY travel is at the rapid rate, but the planner cannot see that rate and refuses
to assume it. If the ~20 extra cycles on the flower annoy more than the
possibility of a join that is a tenth of a second slower, the budget is one
comparison to drop.

The cutting grows by about 550 mm because the routes are real cuts: they travel
through material this stage removes anyway, but they do it with the bit engaged
at pass depth, which is the same kind of cut the floor lanes already make. That
engagement is the one mechanical cost of the option, and it is why `retract`
and `short_lift` are still there: the hierarchy is exactly how much cutting on a
transit the operator is willing to accept.

The verifier needed no new rule. A route is emitted as the *same excursion*,
with the detour prepended to its own path, so it is one continuous cut: the
depths, feeds, entry, retract and per-move continuous sweep bound of the
existing checks all apply unchanged. `VBIT_SWEEP_CLEARANCE` is what makes each
metre of a route shape-safe, and a route that failed it would fail the plan.

The export dialog's **Technical details** section now draws the move-length
histogram the report published — one chart per stage and one for the whole
program, square-root scaled so a dominant bucket cannot hide the tail, with the
buckets at or under 0.05 mm flagged and the threshold marked. `cargo test -p
cam-gui --lib export_ui` checks both the mapping and that the chart paints one
bar per bucket.

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
cargo test --locked -p cam-gui --lib export_ui                # the export dialog's charts
```
