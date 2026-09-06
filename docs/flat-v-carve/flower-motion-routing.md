# Flower job: motion routing, engine 0.7.4

The unchanged `real_data/flower_box-svg.job (2).json` now produces **113,192 motions**, down from **384,245**. V-bit plunges fall from **60,485 to 178**. The job's 30 mm clearance plane, feeds, tool dimensions, depth limits, artwork and tolerance settings are unchanged.

| Measurement | Previous 0.7.3 plan | 0.7.4 plan |
| --- | ---: | ---: |
| Endmill motions | 17,072 | 17,063 |
| V-bit motions | 367,173 | 96,129 |
| Total motions | 384,245 | 113,192 |
| Endmill XY rapid distance | 3,154.16 mm | 573.98 mm |
| V-bit XY rapid distance | 1,019,197.47 mm | 1,575.87 mm |
| V-bit approaches / retracts | 60,549 each | 233 each |
| V-bit plunges | 60,485 | 178 |
| V-bit cutting distance, including links | 72,049.85 mm | 38,145.10 mm |
| V-bit time at programmed feeds | 3,194.54 min | 33.30 min |
| Serialized plan | 110,745,718 bytes | 33,216,127 bytes |

The time row sums XYZ distance divided by each motion's programmed feed. It includes approaches, plunges and cuts; it excludes rapid travel, acceleration, controller blending, spindle startup and tool changes. It is a feed-time lower bound, not a machine-cycle prediction. No rapid-speed profile is present in the saved job. The old approaches alone took 3,027.45 minutes at the saved 600 mm/min plunge feed: each started 30 mm above the stock.

## Causes and changes

The old planner made 29,963 medial candidates, of which 14,536 were shorter than 0.1 mm. Almost every candidate required a separate retract, traverse, approach and plunge. It then repeated the boundary and medial families even when the final depth pass had already completed them and no cleanup followed. Endmill contours followed offset/backend order and started at their longest edges, including plunge entries that did not need a long ramp edge.

Both planners now choose the nearest available path entry using a deterministic spatial index. Open V-bit profiles can reverse; closed contours can rotate their start without changing traversal direction. Endmill ramp entries retain the longest-edge start. Depth layers and tool-stage order remain intact. Floor work precedes the boundary/detail group within each V-bit depth pass.

Short connections remain at cutting depth only after continuous whole-segment clearance checks with a geometry reserve. New connections are limited to one stepover and at most one stepdown below stock top. Deeper V-bit passes may join exactly coincident XYZ endpoints, but otherwise retract; the planner does not assume the material between independently roughed paths is cleared. Connections that cross islands or fail clearance retain the complete clearance-plane excursion. Motion-budget failure also retains a complete retracted prefix.

V-bit candidate endpoints are placed on the existing construction grid only when it is sufficiently finer than the motion tolerance and the changed chords pass the reserved clearance check. This resolves independently reconstructed shared vertices without emitting microscopic connecting moves that disappear during decimal output. Curve subdivisions and job tolerances are retained.

When the last full-depth boundary/detail group is followed by no cleanup, and none of its paths were air-pruned, that traversal is recorded as final finishing. Otherwise the explicit final group is retained. Required finishing is checked as a multiset of cuts, retaining multiplicity, path family and source-branch identity while permitting routing changes. The saved-plan verifier reconstructs every execution, checks added links, tracks prior depth by the same traversal-independent identity, and still rejects missing finish families or omitted required depth passes.

This is a nearest-entry heuristic, not a global machining-time optimum. Many short cutting segments remain because they describe the requested geometry. The principal savings come from removing entry cycles, long travel and duplicate finishing, rather than increasing approximation tolerances.

## Validation

All **226 Rust workspace tests** pass in release mode. Strict workspace/all-target Clippy and formatting checks pass. Tests cover indexed routes against exhaustive nearest-entry selection, reversed profiles and rotated loops, unsafe island links, stock-stepdown limits, microscopic connectors, motion-limit rollback, required finishing, independent stock verification, rounded output and G-code readback. Existing tests that removed entire excursions now reconstruct clearance-plane travel across deleted linked paths, preserving their original coverage-check purpose.

Two independent flower generations produced byte-identical plans, SHA-256 `b1a0c46a061826913c3ac40a0fde18f07be886d0acfb4d6949f24593993fe4fc`. Independent saved-plan authentication and stock reconstruction reports `Complete`, no diagnostics or generation issues, all **29,992** required finish paths executed, zero missing floor beyond tolerance, and zero possible-overcut area in every checked slice. The maximum sampled missed reachable depth remains **0.0466473 mm**, within the unchanged 0.05 mm verification tolerance. This is the planner's M4 evidence; full-volume M5 verification and controller validation of the flower job are separate steps.

Local artifacts are under `flat-v-carve/artifacts/flower-routing-*` (ignored by Git), including plans, motion metrics, generation/replay summaries and the test log. The saved-job SHA-256 remains `59e4c9deb37cc3f0a335eab1f4c53fd86257263b476b96260da560223c6ea693`.

A separate release CLI generation completed in **54.838 seconds**, including analysis and serialization. The preceding planning-only optimization reported 36–37 seconds for 0.7.3; this change prioritizes machining travel and does not improve that generation-time baseline. Independent replay took **36.751 seconds**. An earlier generation overlapped compilation and is excluded from the planning-time comparison.

## Reproduce

From `flat-v-carve`, with the app stopped if its executable is locked:

```powershell
cargo build --release --locked -p cam-app
./target/release/cam.exe plan '../real_data/flower_box-svg.job (2).json' --stage combined --output artifacts/flower-new.plan.json
node scripts/analyze-motions.mjs artifacts/flower-new.plan.json
cargo run --release --locked -p cam-core --example benchmark_pipeline -- --replay artifacts/flower-new.plan.json
```

`analyze-motions.mjs` measures stored records without authenticating them; use the replay command for authentication and reconstruction. Engine 0.7.4 invalidates older plan identities. Saved jobs remain compatible. Rebuild/restart the application and regenerate the job to use the new routing. Validation used `--target-dir target/routing-validation` because an existing app instance held the default executable open.
