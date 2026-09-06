# Saved flower job: CLI performance, engine 0.7.3

Measured on native Windows x64 with the pinned Rust 1.95.0 release build on 2026-09-06. The input is `real_data/flower_box-svg.job (2).json`, SHA-256 `59e4c9deb37cc3f0a335eab1f4c53fd86257263b476b96260da560223c6ea693`. Its embedded SVG and machining settings were left unchanged.

This is a different setup from the older scalability report: 0.001 mm geometry tolerance, 15 selected components, 22,039 normalized boundary vertices, a 3 mm endmill, one 2 mm roughing layer, a pointed 90° V-bit, 0.1 mm permitted floor ridge, and eight stock slices plus the floor-check slice. The saved snapshot's SVG matches `real_data/flower_box.svg` (ignoring surrounding whitespace). The standalone SVG's SHA-256 is `817743c4f77f644fdf703fa9301eba8a52f35df66503f5b0a953c822f6d63154`.

## Measured result

The unchanged saved job now completes combined CLI planning in **54.195 seconds**, followed by a **51.681-second repeat**, both with status `Complete` and exit code 0. This includes endmill planning, V-bit planning, motion/stock/finish checks, serialization, and writing the saved plan. It is a native release CLI measurement on this 16-logical-processor Windows machine, not a browser/UI timing or a guarantee for other hardware.

| Stage | Original 0.7.2 | Optimized 0.7.3 |
| --- | ---: | ---: |
| Endmill | 60.256 s, complete | 6.919 s, complete |
| Combined, including endmill | Still running when stopped at 904.909 s | 54.195 s and 51.681 s, both complete |

The combined artifact contains 17,072 endmill motions and 367,173 V-bit motions and is 110,745,718 bytes, below the unchanged 128 MB reload limit. The two runs produced byte-identical plans (SHA-256 `b1979b992f547b2938762d4f1868934b80df2b8171d74d5eeeff9c2a5db790c5`). Sampled peak working set was 430,833,664 bytes (411 MiB), and 427,204,608 bytes on the repeat. All required finish families remain present. `Complete` retains the existing M4 meaning: continuous cutter-clearance checks, slice coverage, and sampled finish quality; it is not a claim that M5 full-volume verification or M6 controller validation was run.

Raw results and timing logs are in `flat-v-carve/artifacts/performance-flower/guarded-eight-v2/`; the repeat measurement is in `final-repeat/`. These artifacts are ignored by Git. The harness records the executable hash with each measurement.

Normal saved-plan authentication/reconstruction also completed successfully in 40.878 seconds (`final-replay.json`): status `Complete`, no diagnostics or generation issues, all 29,992 required finish paths executed, 175,995 quality samples, zero missing floor beyond tolerance, and zero possible-overcut area in all nine slices. The maximum sampled missed reachable depth was 0.0466473 mm, within the unchanged 0.05 mm verification tolerance.

## Where the baseline spent its time

The original release CLI needed 60.256 seconds for endmill planning. Combined planning was stopped after 904.909 seconds (884.484 CPU seconds), while still reconstructing stock. It had not produced a completed plan. This reproduces the slowdown without the browser.

The instrumented baseline's endmill stage took 60.395 seconds: context/import 0.130 s, motion generation including tool-access geometry 15.397 s, and analysis 44.855 s. Checking the actual cutting motions took only 0.327 s; rebuilding one stock slice took 34.888 s.

Combined candidate generation took 63.717 s, including another 37.960 s rebuild of the endmill stock that had just been computed. Subsequent analysis rebuilt it again for cleanup and each display/quality slice. The first three final V-bit stock slices took 136.883, 187.071, and 88.672 s. Actual depth-pass motion generation took 4.146 s, and assembling final finishing moves took 0.039 s.

These are wall-clock observations, not percentile guarantees. The long baseline overlapped development/profiling work; its near-equal CPU and wall times confirm sustained computation. Final CLI measurements are recorded separately by the reproducible harness below.

## Changes

- Union eight cutter footprints at a time instead of 256, with balanced merges. Hundreds of almost coincident circular polygons create expensive intersection arrangements before their interiors can be discarded.
- Process footprints in spatial order, using the existing bounding-volume hierarchy. Motion order and recorded contributing motion IDs remain unchanged.
- Omit exact repeated footprint construction with bounded memoization, including reversed sweeps. Cutting motions and mandatory final passes remain in the plan.
- Omit a plunge's disk polygon only when a nonstationary sweep has an equally large endpoint disk at exactly the same XY. The retained outer bound still covers the complete disk; a deeper plunge is retained.
- Reuse a rebuilt endmill slice only when **every** motion has exactly the same clipped XY sweep at the requested depth. Partly clipped ramps and different layer coverage fall back to reconstruction. Serialized analysis is still ignored during authentication.
- Cache repeated tool-center sets on the immutable target, bounded to eight entries and 131,072 retained points. Area-only nominal stock sections no longer calculate unused Voronoi contact/witness reports.
- Erode large disconnected regions component by component, retaining each component's holes, with at most four workers. Reconstruct independent combined stock slices with at most eight workers. Results and error selection remain in deterministic input order.
- Refine the planning construction grid by a factor of 16 when coordinate range permits. The power-of-two rescaling preserves every normalized source coordinate exactly, retains the original input snap uncertainty, and leaves geometry/arc tolerances unchanged. It resolves the flower's tiny positive-clearance center-set witness that fell outside the coarser offset polygon. The general `Target::new` fixed-grid contract is unchanged; planning uses the explicit `Target::for_planning` constructor.
- Re-normalize crossed integer backend output with at most two additional polygon unions, validating boundaries after each attempt. Proper crossings first receive a shared vertex computed by exact rational intersection and integer rounding, within the grid snap bound. Successful repair carries `OUTPUT_RENORMALIZED`; exhausted repair still fails. Source-ring validation remains strict. This addresses rounded intersection artifacts exposed by changing union order; it is not permission to accept intersecting output.
- Retain an empty lower bound when a tiny near-apex inner sweep collapses on the grid. The outer bound is still constructed and the radial reserve includes the collapsed radius. This fixes a later slice failure without treating the tiny sweep as absent from the upper stock bound.
- Preserve one geometry tolerance of clearance along medial chords by continuously checking an enlarged cutter during subdivision. Previously safe, nearly tangent chords produced outer stock-bound slivers up to 0.00022 mm outside the nominal target. The stricter construction removes that uncertainty without weakening verification or changing the user's tolerances.

Machining tolerances, feature geometry, sampling requirements, path families, and configured resource limits are unchanged. Polygon operation order can affect grid-level stock boundaries and subsequent rest contours, so engine 0.7.3 invalidates older plans. Saved jobs remain compatible.

## Regression checks

All 184 release tests pass, together with strict Clippy across the workspace/all targets and formatting checks. Added checks cover 1,000 densely overlapping short sweeps against an analytic capsule, duplicate/reversed footprints and retained motion IDs, covered versus deeper plunges, exact slice reuse versus reconstruction for plunges/ramps/multiple layers, cached access contacts and independent returned geometry, parallel slice geometry/error ordering, component erosion with holes within grid rounding, source-preserving construction refinement and its range fallback, reserved medial-chord clearance, checked output normalization with holes and an exhausted repair budget, the actual flower crossing/contact coordinates, and sub-grid apex bounds. Existing clearance, stock verification, artifact authentication, and LinuxCNC readback tests also pass.

## Reproduce

From `flat-v-carve`, build the release executable and use a new output directory:

```powershell
cargo build --release --locked -p cam-app
./scripts/benchmark-flower.ps1 -OutputDirectory artifacts/flower-performance-new
```

The harness runs endmill and combined stages sequentially, records exit codes, elapsed time, sampled peak working set, artifact sizes, input/executable hashes, and stage timing logs. A diagnostic JSON file is not a successful plan: check the exit code. Use `-Job <path>` for another snapshot or `-Stages endmill` for a shorter measurement.

For direct CLI profiling, set `$env:CAM_TIMINGS='1'` and run `cam plan` normally. Timings go to stderr without changing artifacts. Nested stage timings and accumulated capsule/union timings overlap; do not add every printed number together. Remove the environment variable to disable profiling.

The `benchmark_stock` core example isolates a single slice from a combined motion artifact:

```powershell
cargo run --release --locked -p cam-core --example benchmark_stock -- artifacts/flower-performance-new/combined.plan.json 0.25
```

That example measures polygon construction/merging and stock comparisons only. It does not authenticate a plan or certify machining. Production `inspect`/`verify` retain their normal authentication and reconstruction.

`benchmark_pipeline --replay <combined-plan.json>` authenticates and rebuilds a saved combined plan, reporting elapsed time, status, motion/path counts and sampled-quality results without generating a large preview/report. As with the generation benchmark, inspect `combined_status` in its JSON; successful recording of a benchmark report is not itself a finish-quality result.
