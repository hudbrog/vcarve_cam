# Saved flower job: below 40 seconds

Follow-up to [the original performance report](flower-performance.md), measured on 2026-09-06 with the native Windows x64 Rust 1.95.0 release CLI on the same 16-logical-processor machine.

## Result

The unchanged saved flower job completes combined planning in **37.146 seconds**, followed by a **36.335-second repeat**. Both exit with code 0 and status `Complete`. These are end-to-end CLI times, including endmill planning, V-bit motion generation, verification, serialization, and writing the plan. The previous completed measurements were 51.681 and 54.195 seconds: this follow-up reduces elapsed time by about 30%.

Both final plans are byte-for-byte identical to the previous verified 0.7.3 plan: SHA-256 `b1979b992f547b2938762d4f1868934b80df2b8171d74d5eeeff9c2a5db790c5`, 110,745,718 bytes. They retain 17,072 endmill motions, 367,173 V-bit motions, all required finish passes, and the original geometry and verification tolerances. Sampled peak working sets were 428,441,600 and 433,766,400 bytes (409 and 414 MiB).

The input remains `real_data/flower_box-svg.job (2).json`, SHA-256 `59e4c9deb37cc3f0a335eab1f4c53fd86257263b476b96260da560223c6ea693`. The standalone SVG remains SHA-256 `817743c4f77f644fdf703fa9301eba8a52f35df66503f5b0a953c822f6d63154`. No saved settings or artwork were edited.

## Where time goes now

Non-overlapping top-level stages from the 37.146-second run:

| Stage | Wall time |
| --- | ---: |
| Final stock reconstruction and analysis | 17.488 s |
| Endmill planning and analysis | 6.657 s |
| Cleanup sampling and floor reconstruction, overlapping | 4.379 s |
| Candidate preparation, medial and area paths overlapping | 4.253 s |
| Depth-pass motion generation | 2.180 s |
| Execution and continuous-clearance verification | 1.091 s |
| Final finish motion assembly | 0.022 s |
| Context, fingerprints, serialization, file I/O, and CLI overhead | 1.076 s |

Nested worker timings overlap and must not be added to these totals. Final stock analysis remains the largest opportunity: nine independently reconstructed slices run on up to eight workers, and the shallow slices' polygon unions determine the remaining critical path. Further work should focus on those unions and their partitioning. It must retain conservative stock bounds and deterministic geometry validation.

## Changes

- Evaluate quality samples with up to four workers sharing immutable stock indexes. Preserve every sample location, value, resource limit, and result/error order.
- Remove exact forward duplicate cutting sweeps from analytic query indexes. Reversed or different-depth sweeps remain distinct. This changes the index, not the stored motions.
- Reuse cleanup samples after final finishing only when a full motion-prefix hash matches and every appended cutting sweep is an exact forward duplicate of an already sampled sweep. Any new sweep or changed prefix falls back to full sampling. Saved-plan loading never accepts this cache and recomputes samples.
- Carry the result of complete execution verification directly into analysis, tied to the exact borrowed motion slice. Memoize continuous-clearance calculations for exact duplicate sweeps while still validating every record's identity, continuity, feed, entry, and range.
- Overlap independent cleanup sampling and floor reconstruction. Overlap medial and area candidate preparation for large inputs; retain the original family order and error priority.
- Assign the next stock slice to the first free worker, so the ninth slice does not wait behind the longest-running shallow slice.
- Reuse freshly rebuilt complete endmill accessible-floor geometry. Exact-fit contact and unresolved cases still rebuild their full access geometry.
- Cache the original circle direction values by polygon resolution, avoiding repeated trigonometry without changing polygon coordinates or arithmetic.

Memoization is bounded (131,072 retained entries for sweep caches; at most 1,024 circle resolutions). Parallel work is bounded, with serial paths for small inputs or single-processor hosts. These changes preserve artifact identity and require no additional engine-version change.

## Validation and reproduction

All **160 cam-core release tests** pass, as do strict Clippy for all core targets and the core formatting check. Added regressions compare serial/parallel samples and candidate families, first-error order, analytic indexes against independent full-motion scans, sample reuse against fresh sampling, changed-prefix/new-cut rejection, and endmill access reuse with exact-fit fallback. Existing stock, clearance, resource-limit, and artifact-authentication tests remain in the suite.

Independent saved-plan authentication and reconstruction completed in **34.301 seconds**, with status `Complete`, no diagnostics or generation issues, all 29,992 required finish paths executed, and 175,995 freshly evaluated quality samples. Missing floor beyond tolerance and possible-overcut area in all nine slices were zero. Maximum sampled missed reachable depth remains 0.0466473 mm, within the unchanged 0.05 mm verification tolerance. The replay does not use the generation-time sample cache.

The CLI build includes a separate concurrent packaging refactor; these optimization changes are confined to cam-core. The measurements use the same executable SHA-256 `0c4b2dad09e8768370ddd23f8afaaea955ade99afaf3b1841842993342af1f40`. Build and test work was not run alongside the two final measurements. These are observed times on this host, not a latency guarantee under arbitrary system load. An earlier intermediate implementation measured 38.661 and 43.946 seconds before candidate preparation was parallelized; it was not accepted as the final result.

From `flat-v-carve`:

```powershell
cargo build --release --locked -p cam-app
./scripts/benchmark-flower.ps1 -OutputDirectory artifacts/flower-40s-new -Stages combined
cargo run --release --locked -p cam-core --example benchmark_pipeline -- --replay artifacts/flower-40s-new/combined.plan.json
```

Inspect the CLI exit code and the replay report's `combined_status`. `Complete` retains the existing M4 meaning: continuous cutter-clearance checks, slice coverage, and sampled finish quality.

Recorded artifacts are under `flat-v-carve/artifacts/performance-flower/`: `target40-final/`, `target40-final-repeat/`, `target40-final-replay.json`, `tests-40-fourth.txt`, and `clippy-40-final.txt`. These generated files are ignored by Git.
