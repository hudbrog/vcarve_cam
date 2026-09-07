# Real flower job: below five seconds

Measured on 2026-09-07 in an isolated worktree from `main` at `e6c274f`, using the pinned Rust 1.95.0 native Windows x64 release CLI on an AMD Ryzen 9 8945HS (16 logical processors).

The benchmark is `real_data/flower_box-svg.job-real.json`, SHA-256 `80dd0208cec4d2622d5c5719a96e16ef85d4c2a82e8e72dae8c2c1ecaa552ae7`. This is the current real job: 0.005 mm geometry tolerance, 1 mm depth, 3 mm endmill, 90-degree V-bit with a 0.1 mm tip, 0.01 mm motion tolerance, 0.05 mm verification tolerance, and eight stock slices. Artwork, machining settings, required paths, and verification requirements were unchanged.

This is a different input from the older 2 mm pointed-bit job in the [40-second report](flower-performance-40s.md). These results do not establish a five-second time for that older snapshot or the wood presets.

## Result

| Run | Complete combined CLI time |
| --- | ---: |
| Unmodified main baseline | 5.213 s |
| Optimized 1 | 4.352 s |
| Optimized 2 | 4.112 s |
| Optimized 3 | 4.153 s |
| Optimized 4 | 4.114 s |
| Optimized 5 | 4.151 s |

All five optimized runs exited zero with status `Complete`. Mean time was **4.176 s**, about 20% below baseline; the slowest was 0.648 s below the five-second target. Each measurement starts a new CLI process and includes job import, endmill and V-bit planning, continuous motion checks, stock reconstruction, sampled finish analysis, serialization, writing the plan, and process shutdown. Builds and tests were not run alongside these measurements. There is no persistent plan cache or excluded warmup run. These are observed local timings, not a guarantee under arbitrary hardware or system load.

Every saved plan is byte-identical to the baseline: SHA-256 `34715752ad93f504d063e239f521d5d26b966aca14c372f9d7d5027da9691185`, 7,620,116 bytes, 12,590 endmill motions, and 15,843 V-bit motions. Sampled peak working sets across the five runs were 134,340,608–143,081,472 bytes. Executable SHA-256: `092b900fa0995c0027a511326ab50be539a08339658df893a36dc3cd047eaffc`.

## Changes

- Transfer the freshly planned endmill target into the combined stage. Both contexts derive the same selected geometry, depth, and V-bit angle from the same borrowed job. This keeps the prepared Voronoi diagram and bounded access caches available for the second tool; validation order and the standalone endmill API are preserved.
- Retain up to 131,072 exact boundary-clearance samples from the first successful center-set query. Their values depend on immutable target geometry, independently of cutter radius. Later center-set queries, medial paths, and candidate depth queries can reuse them. Initialization uses `OnceLock`; concurrent readers need no mutex. Missing samples still run the independent boundary predicate. Saved-plan loading rebuilds its own geometry and evidence.
- Use bounded local traversal stacks in the balanced spatial index instead of allocating a vector per query. Median splits bound deferred siblings by the machine word size. Nearest searches carry the already computed child-distance bounds into the next traversal step, preserving branch order, exact predicates, and numerical reserves while avoiding duplicate distance calculations.
- Extend the benchmark harness with repetitions, a strict elapsed-time threshold, completion status, and plan hashes. The default input is now the current real job. An over-budget, incomplete, missing-plan, or timed-out run makes the harness fail; CLI `Empty` status cannot pass as a completed flower job.

The engine remains 0.7.6 and the release compiler profile is unchanged. No geometry simplification, tolerance relaxation, sample reduction, skipped verification, or motion removal was introduced.

## Validation

- `cargo test --workspace --release --locked`: 258 tests passed; the two existing opt-in flower tests were initially ignored.
- The opt-in `planner_optimizations` flower regression then passed separately. It checks deterministic repeated generation, saved-plan authentication/reconstruction, absence of grid-tick motions, full-stock G-code export verification, and readback verification. Export and readback both returned `passed`; the existing postprocessor increased output precision from three to five decimals to preserve all motions. The replayed plan still matches the baseline hash.
- New regressions compare retained and fresh targets for islands, finite tips, and exact-fit geometry; concurrent cache initialization against independent clearance queries; cached range errors; and nearest spatial queries against full scans at large coordinates. Existing analytic stock, clearance, geometry topology, and artifact authentication tests passed.
- Strict Clippy across the workspace and all targets, formatting, and `git diff --check` passed.
- Harness checks rejected a completed run above its threshold, an inconclusive resource-limited job, and a one-second timeout. A subsequent real-flower run passed in 4.114 s, confirming success exits zero after earlier failures.

## Reproduce

From the worktree's `flat-v-carve` directory, choose a fresh artifact directory:

```powershell
cargo build --release --locked -p cam-app
./scripts/benchmark-flower.ps1 -Stages combined -Repetitions 5 -MaxSeconds 5 -OutputDirectory artifacts/flower-under-five
cargo test --workspace --release --locked
cargo test --workspace --release --locked --test planner_optimizations -- --ignored --nocapture
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```

`-MaxSeconds 5` requires **every** run to complete strictly below five seconds. The time gate is opt-in, so ordinary CI is not tied to this machine's speed. Use `-Job <path>` to measure another snapshot explicitly.

Recorded logs are under `flat-v-carve/artifacts/plan-optimization/`: `baseline-real/`, `final-runs/`, `tests.txt`, `clippy.txt`, `flower-verification.txt`, `verified-export/`, and `harness-checks.json`. Generated artifacts are ignored by Git.
