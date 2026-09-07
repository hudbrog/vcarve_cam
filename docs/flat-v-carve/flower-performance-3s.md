# Real flower job: below three seconds

Engine **0.7.7**, measured on 2026-09-08 in `codex/plan-optimization`, the worktree created from `main` at `e6c274f`. This continues the [five-second optimization](flower-performance-5s.md) at `58fd075`.

## Benchmark

The input remains `real_data/flower_box-svg.job-real.json`, SHA-256 `80dd0208cec4d2622d5c5719a96e16ef85d4c2a82e8e72dae8c2c1ecaa552ae7`. Artwork and job settings are unchanged: 0.005 mm geometry tolerance, 1 mm depth, 3 mm endmill, 90-degree V-bit with a 0.1 mm tip, 0.01 mm motion tolerance, 0.05 mm verification tolerance, and eight stock slices.

Five fresh native Windows x64 release CLI processes ran on the same AMD Ryzen 9 8945HS with 16 logical processors and pinned Rust 1.95.0. Timing includes startup, import, both planning stages, continuous motion checks, stock reconstruction, sampled finish analysis, serialization, file writing, and shutdown. Compilation and tests did not run concurrently. No warmup run was excluded and no plan cache persists between processes.

| Run | Complete combined CLI time |
| --- | ---: |
| 1 | 2.713 s |
| 2 | 2.724 s |
| 3 | 2.731 s |
| 4 | 2.710 s |
| 5 | 2.710 s |

All runs exited zero with `Complete` status and passed the strict three-second gate. Mean **2.718 s** is 35% below the preceding 4.176 s mean; the slowest run has 0.269 s of margin. These are local measurements for this input, rather than a guarantee on different hardware, system loads, the older pointed-bit flower job, or other presets.

Every output is byte-identical: SHA-256 `6bb14924ed19f419bde46bdc6d3be1a443c4b58a92ba346da95f9bdcd1464e72`, 6,487,536 bytes. The independently replayed and exported plan has the same hash. Executable SHA-256: `1f72c854ff8c36be8284a1bdef383bdf6c7ded76f6cd0fded5006534a1a78e9d`. Sampled peak working sets were 150,806,528–160,370,688 bytes.

## Implementation and output changes

- Extend endmill contour simplification to ordinary tessellation edges, using the existing 0.00125 mm cleanup budget for this job. Every replacement is measured against all original vertices in its span and independently checked for continuous cutter clearance. Preserve ramp endpoints, at least three loop vertices, and the bounded work limit. The source artwork is unchanged.
- Reconstruct exactly connected constant-radius sweep groups as round strokes with explicit inner and outer envelopes. Clipped sweep endpoints remain unchanged; gaps and varying radii split groups. Preserve every contributing motion ID and use the existing capsule path for ineligible inputs. Groups have at most 64 sweeps and a bounded circle-resolution estimate. Coarse grids also retain capsules: stroke grouping requires a snap bound no greater than one sixty-fourth of geometry tolerance.
- Reserve eight grid-snap bounds for stroke construction, including intersection rounding and the two checked output-normalization passes. Round joins use an arc sagitta of one eighth of geometry tolerance. Both stock bounds and the reported radial error include these reserves. Polygon topology and coordinate validation remain enabled.
- Process deterministic groups of eight footprints with bounded workers, preserving merge order across CPU counts. Overlap independent inner/outer unions and stock comparisons. Finish each V-bit slice and immediately compare it with the endmill and nominal target, while other depths are still being reconstructed. Preserve the original construction-before-comparison error priority and input depth order.
- Prefetch up to four independent floor offset levels, retaining level order and resource errors. Share the freshly constructed target between tool contexts, removing duplicate import while retaining validation and the existing bounded caches.
- Infer containment from a nearest edge only when its closest point lies strictly inside the oriented edge with a numerical reserve. Vertex and ambiguous cases retain ray casting. Compare conservative squared spatial bounds without changing exact distance evaluation, and avoid distance searches where callers need only containment.

| Output | Previous | 0.7.7 |
| --- | ---: | ---: |
| Endmill motions | 12,590 | 7,048 |
| Endmill retracts | 29 | 29 |
| V-bit motions | 15,843 | 15,845 |
| V-bit retracts | 214 | 215 |

The endmill path has 44% fewer motions. Its bounded simplification slightly changes residual-floor clipping, and the resulting V-bit route uses one additional retract. The flower regression now permits 215 retracts; continuous clearance, required finishing paths, stock coverage, and output verification still have to pass. No grid-tick motions remain. Saved plans from earlier engine versions must be regenerated from their jobs. The release compiler profile is unchanged.

## Verification

- `cargo test --workspace --release --locked`: **264 tests passed**; two existing opt-in tests remain excluded from the ordinary suite.
- The opt-in `planner_optimizations` flower test passed separately: repeated deterministic generation, authenticated saved-plan reconstruction, motion/retract checks, full-stock G-code export verification, and numeric readback. Export and readback both returned `passed`, using the existing adaptive five-decimal output precision. This separate full-stock/output check is not included in the three-second planning measurement.
- New regressions compare boundary signs with independent ray crossings and distances with exhaustive scans; test stroke bounds against analytic disks at sharp turns, crossings, holes, retraces and collapsed centerlines; check gaps, radii, coarse-grid fallback and worker-count determinism; verify contour error accumulation and ramp preservation; and compare streamed slices/prefetched offsets with sequential results and error priority.
- Strict workspace/all-target Clippy, formatting, and `git diff --check` passed. Temporary profiling instrumentation was removed.

## Reproduce

From the worktree's `flat-v-carve` directory, choose a fresh output directory:

```powershell
cargo build --release --locked -p cam-app
./scripts/benchmark-flower.ps1 -Stages combined -Repetitions 5 -MaxSeconds 3 -OutputDirectory artifacts/flower-under-three
cargo test --workspace --release --locked
cargo test --workspace --release --locked --test planner_optimizations -- --ignored --nocapture
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```

The harness requires every run to finish strictly below three seconds with `Complete` status. Evidence is under `flat-v-carve/artifacts/plan-under-three/`: `final-runs/summary.json`, per-run plans/timings, `tests-final.txt`, `clippy.txt`, `flower-verification.txt`, and `verified-export/`. Generated artifacts remain ignored by Git.
