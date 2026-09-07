# Tiny motions and endmill stay-down links

Engine **0.7.6** implements these optimizations. Regenerate older plans from
their saved jobs; job settings and schemas are unchanged.

## Implementation and validation, 2026-09-07

Endmill cleanup removes tiny offset edges within the remaining motion-error
budget and at most one quarter of the geometry tolerance. It measures every
original vertex against the final replacement chord, checks continuous cutter
clearance, preserves closed loops and protects the full ramp-entry edge.

Endmill links include bounded construction-error slack. Larger corner gaps use
an already-cut contour to reach a suitable departure, or split a destination
edge to enter between its vertices. The full contour is retained. Alternate
entry searches and retracing distances are bounded. Deeper links additionally
require the complete cutter footprint to fit inside lower-bound sweeps from
earlier layers at `depth - stepdown`. Unproved links retain their retracts.

V-bit medial endpoints reconcile to immutable nearby boundary vertices before
candidate identities and execution records are generated. Endpoint depths and
point features are retained, displacement is bounded in both XY and cone-height
error, and changed sweeps retain clearance. One-grid-tick link comparisons now
account for floating-point reconstruction error.

The unchanged flower job produces the following results:

| Measurement | 0.7.5 | 0.7.6 |
| --- | ---: | ---: |
| Endmill retracts | 64 | **29** |
| V-bit retracts | 221 | **214** |
| Movements below 0.00001 mm, both stages | 22 | **0** |
| Endmill XYZ travel | 3,112.009 mm | **2,704.494 mm** |
| Endmill feed-only time | 80.765 s | **67.461 s** |
| Endmill motion records | 12,522 | 12,590 |
| V-bit motion records | 15,882 | 15,843 |
| Required export decimal places | 6 | **5** |

Endmill records increase because some connections explicitly retrace previously
cut contour edges. Travel falls by 407.515 mm and retracts by 54.7%. Feed-only
time excludes rapids, acceleration and controller behavior.

Two fresh flower generations are byte-identical. Saved-plan authentication and
execution replay pass. Both original and emitted stock verification pass, as
does independent G-code readback. The rectangle fixture now clears both depth
layers with one excursion per layer. Tests also cover reverse/outward entry,
island crossings, disconnected regions, full-cutter stock clearance, ramp entries,
tiny loop closures, bounded chained merges, immutable endpoint witnesses and
motion-budget rollback through both layers.

Validation passed: **255 default Rust workspace tests**, the additional opt-in
flower regression, strict workspace/all-target Clippy, formatting, **100 frontend
tests**, schema/display contracts, and **27 packaged CLI/HTTP tests**. Four opt-in
packaged tests were skipped. The new flower regression is independently runnable:

```powershell
cargo test --release --locked -p cam-core --test planner_optimizations -- --ignored --nocapture
```

The rebuilt portable executable is retained locally at
`flat-v-carve/artifacts/planner-small-optimizations/portable/cam.exe`.
Its SHA-256 is
`f8860d4237b99186cebeb333a47f7890985fbb8d234788683582b791070d3c06`.
Run this executable after closing the older app, then regenerate the job.
Final plans, comparison metrics, export report and G-code readback are under
`artifacts/planner-small-optimizations/implemented/`; validation logs are in
their parent directory. These local build artifacts are ignored by Git.

## Initial investigation

Investigation on 2026-09-06, source revision `4841e0a`, engine 0.7.5.
Both reported behaviors are reproducible. Tiny endmill offset edges can be
collapsed with a bounded geometry change. Many endmill retracts are caused by
an overly strict link-length heuristic. V-bit routing also has a grid-threshold
comparison that turns some microscopic gaps into full entry cycles.

The initial investigation left production planning unchanged. The measurements below
come from a freshly generated plan and isolated modifications to copies of its
raw motions. They establish candidate optimizations, not authenticated replacement
plans or exported machine programs.

## Input and baseline

Measured [the saved flower job](../../real_data/flower_box-svg.job-real.json),
SHA-256 `80dd0208cec4d2622d5c5719a96e16ef85d4c2a82e8e72dae8c2c1ecaa552ae7`.
The screenshot illustrates the symptom; its exact job/settings were not supplied.
This available snapshot uses a 3 mm endmill, 1.5 mm stepover, 2 mm maximum
stepdown, 1 mm total depth, plunge entry, and 5 mm clearance. Its motion tolerance
is 0.01 mm and verification tolerance is 0.05 mm.

Fresh planning reports `Complete`: 12,522 endmill motions and 15,882 V-bit motions.
Independent full-stock verification of the baseline reports `Passed`.

| Stage | Motions shorter than 0.00001 mm | Motions shorter than 0.001 mm | Retracts |
| --- | ---: | ---: | ---: |
| Endmill | 4 cuts | 23 cuts | 64 |
| V-bit | 11 cuts + 7 XY rapids | 57 cuts + 7 XY rapids + 2 plunges | 221 |

Lengths are full XYZ distances. These are different kinds of motion, so a global
"delete short records" filter would not preserve the motion contract.

## 1. Microscopic movements

### Endmill: offset edges reach the output unchanged

`Region::refine_construction_grid` multiplies the integer construction scale by
16 while preserving the normalized source geometry. Here the resulting quantum
is **0.00000625 mm**. `pocket::plan_endmill` takes the offset ring vertices and
`make_loop` emits each edge; its only length filter is exact `position != end`.
There is no bounded endmill contour simplifier.

All four endmill cuts below 0.00001 mm are one grid tick long, either axially or
diagonally. Original motion 122 is the previously documented example:

```text
(39.13473125, 30.979125, -1) -> (39.1347375, 30.979125, -1)
length = 0.00000625 mm
```

The experiment merges a tiny cut into its preceding cut only at the same depth,
layer and feed. It checks all original intermediate vertices against the final
replacement chord, preventing accumulated simplification error, and checks the
whole replacement segment against cutter clearance with a reserve. It then
rebuilds endmill coverage and runs full-stock verification with the existing
V-bit motions.

| Merge threshold / deviation budget | Cuts removed | Maximum chord deviation | Endmill / full stock |
| --- | ---: | ---: | --- |
| 0.00001 mm | 4 | 0.000008838835 mm | Complete / Passed |
| 0.001 mm | 22 | 0.000755963715 mm | Complete / Passed |

The one remaining sub-0.001 mm cut was outside this experiment's eligible
preceding-cut pairing. Both experiments have zero missing-floor area beyond
tolerance, zero possible-overcut area in the endmill slice, and no full-stock
findings. The four smallest merges save only about 0.000031 mm of travel: this
is mainly useful for cleaner motion records and output precision, not travel time.

### V-bit: independently reconstructed endpoints and a strict threshold

All 18 sub-0.00001 mm V-bit motions occur at the beginning of a boundary
execution following a medial execution. They connect independently constructed
endpoints one grid tick apart in one or both XY axes; all are at Z = -1 mm.
They are routing connectors, not tiny internal contour features.

`weld_endpoints` rounds each candidate endpoint independently. Nearby endpoints
can still land on adjacent grid points. In `routing::can_link`, the condition
`distance < 1. / grid.scale()` rejects short connections. The seven XY rapids
have a computed gap of **0.000006249999998431122 mm**, just below the nominal
0.00000625 mm quantum. Other one-tick gaps evaluate just above it and become
cutting links instead. This is a floating-point threshold artifact on top of the
adjacent-grid endpoint mismatch.

Replacing those seven excursions with continuously checked cutting links passes
full-stock verification and reduces V-bit retracts from **221 to 214**. Merging
the resulting 18 tiny connectors into their following constant-depth cuts also
passes, with **15,843 motions instead of 15,882** and no remaining movements
below 0.00001 mm. The seven links retain approximately 0.00499 mm clearance after
the additional V-bit geometry reserve.

For production, do the bounded endpoint reconciliation before candidate identity
and execution records are finalized. Update replay consistently. Raw motion
deletion would otherwise invalidate recorded candidate cuts, IDs and hashes.
Keep depth-changing features, point cuts and mandatory entry motions unless
their own cutter-envelope and execution contracts are preserved. Existing
adaptive G-code precision remains useful for small motions that must survive.

## 2. Endmill lifts within a clearing component

The existing stay-down condition in `pocket::plan_endmill` requires all of:

- Plunge entry and depth no greater than one stepdown from stock top.
- The preceding retract belongs to the same layer.
- Distance to the chosen next contour vertex is at most one stepover, exactly.
- Whole-segment cutter clearance retains half the geometry guard.

The route selects existing vertices by nearest XY distance before checking links.
It does not optimize for the availability of a stay-down connection.

The flower plan has 63 between-path transitions plus its final retract:

| Classification | Transitions |
| --- | ---: |
| Between distinct filled components | 17 |
| Within one component, but direct link fails clearance | 10 |
| Within one component, same layer, shallow and clear | 36 |

All 36 otherwise eligible links exceed the exact 1.5 mm cutoff. Examples are
**1.5000004053 mm**, **1.5000061595 mm**, and **1.5013321556 mm**. An allowance of
0.0001 mm admits 16 of them; 0.00125 mm admits 19; 0.015 mm admits 21. These are
candidate counts, not a justification for an arbitrary epsilon.

Corner geometry produces larger gaps even without numerical noise. In the
rectangle fixture, contours spaced 1.5 mm apart have chosen corner vertices
**sqrt(2) × 1.5 = 2.12132034 mm** apart. Every such first-layer connection retracts
despite having continuous clearance. Changing only numerical slack cannot fix
this class of transition.

The following experiments preserve the route and all original contour cuts.
They replace only same-component, same-layer, first-stepdown excursions whose
complete connecting segment passes the reserved clearance check. The enlarged
distance caps are sensitivity measurements, not proposed production settings.

| Maximum candidate link length | Endmill retracts | Motions | Total XYZ travel | Feed-only time |
| --- | ---: | ---: | ---: | ---: |
| Current 1.5 mm | 64 | 12,522 | 3,112.009 mm | 80.765 s |
| 1.515 mm (1.01 × stepover) | 43 | 12,459 | 2,860.009 mm | 72.261 s |
| 2.25 mm (1.5 × stepover) | 37 | 12,441 | 2,788.009 mm | 69.882 s |
| 3.0 mm (2 × stepover) | 35 | 12,435 | 2,764.009 mm | 69.142 s |

All three report endmill `Complete`, full-stock `Passed`, no findings, and zero
missing-floor/possible-overcut area in the endmill slice. Feed-only time sums
distance/feed for programmed feed motions; it excludes rapid time, acceleration
and controller behavior. These tests verify modeled geometry, not cutting load.

The same shallow, checked 1.5× experiment also passes the existing rectangle,
retained-island and disconnected-region fixtures. Their retract counts change
9→5, 14→12 and 12→8 respectively. Cross-component travel, failed-clearance
connections and deeper-layer excursions remain present.

## Recommended implementation boundary

1. Add bounded endmill contour cleanup before motion emission, preserving loop
   closure and entry behavior. Tie its deviation budget to the existing motion
   and geometry budgets, check every original vertex against the final chord,
   and retain full replacement-sweep clearance checks.
2. Reconcile nearby V-bit candidate endpoints with an explicit envelope bound,
   and make the one-grid-tick routing decision stable under floating-point
   reconstruction. Keep generation and execution replay consistent.
3. Make the endmill stepover comparison aware of bounded construction error.
   For larger corner gaps, use contour/stock-aware links or traverse an already
   cleared contour to a suitable exit before crossing to the next contour.
   Prefer feasible stay-down routes when choosing starts.
4. Treat deeper layers separately: establish swept-stock clearance above the
   permitted fresh stepdown before enabling deeper links. Merely removing
   `depth <= stepdown` does not establish that intervening stock was cleared.

"Same island" must not replace continuous clearance checking: ten actual
same-component flower transitions fail that check. Regression coverage for a
production change should include concave/hole crossings, multiple layers,
non-plunging ramp entries, tiny closing edges, chained merges, motion-budget
rollback, authenticated replay and G-code readback.

## Evidence and reproduction

Local experiment source and JSON reports are retained in
`flat-v-carve/artifacts/planner-small-optimizations/` (ignored by Git):
`flower.json`, `rectangle.json`, `island.json`, `disconnected.json`, and
`vbit-gaps.json`. The fresh unchanged baseline is
`flower.json.baseline.plan.json`. No modified plan or machine program is emitted.

From `flat-v-carve`, rerun the locally retained harness:

```powershell
cargo run --release --offline --manifest-path artifacts/planner-small-optimizations/Cargo.toml --target-dir target --bin planner-small-optimizations -- ../real_data/flower_box-svg.job-real.json artifacts/planner-small-optimizations/flower.json combined
cargo run --release --offline --manifest-path artifacts/planner-small-optimizations/Cargo.toml --target-dir target --bin vbit-gaps -- artifacts/planner-small-optimizations/flower.json.baseline.plan.json artifacts/planner-small-optimizations/vbit-gaps.json
./target/release/planner-small-optimizations.exe fixtures/m3/rectangle.json artifacts/planner-small-optimizations/rectangle.json
./target/release/planner-small-optimizations.exe fixtures/m3/island.json artifacts/planner-small-optimizations/island.json
./target/release/planner-small-optimizations.exe fixtures/m3/disconnected.json artifacts/planner-small-optimizations/disconnected.json
```

Production code locations: `geometry/polygon.rs` construction refinement;
`pocket/mod.rs` offset-loop collection, linking and `make_loop`;
`pocket/verify.rs` continuous clearance; `vcarve/routing.rs` endpoint welding and
`can_link`; `vcarve/prune.rs` existing V-bit envelope-bounded simplification;
`vcarve/verify.rs` execution replay. Paths are relative to
`flat-v-carve/crates/cam-core/src/`.
