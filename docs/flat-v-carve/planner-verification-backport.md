# Verification optimizations reused by the planner

Measured on 2026-09-06 with native Windows x64 release builds, using the same static CRT and bundled UI configuration for both executables.

## Changes

- Boundary queries expose containment separately from nearest-boundary distance. Continuous variable-radius clearance checks retain their containment checks and quadratic proof, but no longer perform unused endpoint distance searches. Whole-segment distance queries retain coordinate and empty-region validation without first calculating two unused point samples.
- Medial construction retains exact boundary samples for repeated subdivision endpoints and branch radii. The cache belongs to one immutable planning context, uses the exact floating-point coordinate bits, and stops inserting after 131,072 entries. It never rounds query coordinates or caches errors. Safe-depth calculation consumes the same sample instead of querying it again.
- A successful continuous clearance proof can satisfy the medial accuracy check when its checked disk radius is at least as large at both endpoints. Both radii interpolate linearly along the chord, so containment holds throughout the move. Otherwise the original second proof still runs. Subdivision budgets, tolerances, and arithmetic reserves are unchanged.
- Cleanup retains the floor stock slice it has already constructed. Final analysis reuses it only when the complete motion records have exactly the same length and fingerprint. Changed records invalidate reuse. Even appended duplicate cuts require new polygons and contributing-motion IDs, although the pre-existing sample cache can still reuse their point evidence. Final slice order is unchanged.

These caches stay inside fresh planning. Saved-plan reconstruction does not accept them as external evidence. M4 still constructs stock polygons and sampled quality evidence; M5 remains the separate continuous coverage certification.

## Original flower job

The original job was frozen from the committed input because the working copy was subsequently renamed and its settings changed. Its SHA-256 is `59E4C9DEB37CC3F0A335EAB1F4C53FD86257263B476B96260DA560223C6EA693`. The geometry tolerance is 0.001 mm, maximum depth 2 mm, pointed V-bit tip diameter 0 mm, motion tolerance 0.01 mm, and verification tolerance 0.05 mm.

Three sequential, interleaved baseline/optimized pairs measured end-to-end combined planning, serialization, and file writing:

| Pair | Baseline elapsed | Optimized elapsed | Baseline CPU | Optimized CPU |
| --- | ---: | ---: | ---: | ---: |
| 1 | 32.571 s | 20.408 s | 92.328 s | 59.375 s |
| 2 | 30.146 s | 33.743 s | 89.953 s | 88.578 s |
| 3 | 24.697 s | 19.923 s | 70.531 s | 59.922 s |
| Median | 30.146 s | 20.408 s | 89.953 s | 59.922 s |

The observed median elapsed reduction is 32.3%. Timing variability is substantial: other work was active on this machine, and pair 2 was slower after the change. These results do not establish a fixed speedup or a latency guarantee. The benchmark process was never run alongside another benchmark or a build/test started by this task.

All six runs report `Complete` and produce byte-identical plans: SHA-256 `BC166699C4C57F29100FF81BFDB66B62EC1DE90375B4F0374DAC2AC7F80B015F`, 14,054,720 bytes, 17,063 endmill motions, and 28,501 V-bit motions. No settings, motion coordinates, or tolerances changed. Optimized sampled peak working sets were 193.4–194.7 MiB.

The last optimized run spent 5.017 s in endmill planning, 4.276 s preparing candidates, 1.646 s in cleanup, 0.261 s checking executions, and 8.159 s in final analysis. Nested worker timings overlap. Stock-polygon unions remain the main remaining cost. The M5 surface bound can certify residual depth without constructing all those polygons, but it does not supply the polygons needed for path generation and stock previews. Replacing or deferring that work requires a separate planner/inspection change.

## Newer flower settings

The renamed working job was frozen separately as `flat-v-carve/artifacts/planner-verification-backport/current-flower.job.json`, SHA-256 `80DD0208CEC4D2622D5C5719A96E16EF85D4C2A82E8E72DAE8C2C1ECAA552AE7`. It uses 0.005 mm geometry tolerance, 1 mm maximum depth, and a 0.1 mm V-bit tip diameter. Those settings were not changed by this work.

| Pair | Baseline elapsed | Optimized elapsed | Baseline CPU | Optimized CPU |
| --- | ---: | ---: | ---: | ---: |
| 1 | 9.414 s | 13.893 s | 29.766 s | 30.094 s |
| 2, optimized first | 9.656 s | 8.949 s | 30.125 s | 27.359 s |
| 3 | 7.810 s | 15.020 s | 23.516 s | 25.922 s |
| Median | 9.414 s | 13.893 s | 29.766 s | 27.359 s |

Median CPU time is about 8.1% lower, but elapsed time is worse in these observations. Other builds and CAM processes were active; the slower runs also slow down context construction and unchanged polygon work. These measurements cannot isolate a wall-time regression from contention, and **do not establish an elapsed-time improvement for this newer job**. A controlled rerun on an idle host is needed to resolve that question. The tighter original geometry tolerance also allows more medial accuracy proofs to be subsumed by the existing clearance proof; the newer setting retains the second check when required.

All six plans are byte-identical, SHA-256 `34BA8A77E9E38BF58E8A19E42602B0FC12E9E8EF49ED0207470DDC6F6DB9AFC1`, 7,614,481 bytes. These runs measure M4 planning only; no continuous M5 pass for the newer job is claimed here. Raw evidence is under `flat-v-carve/artifacts/planner-backport-current-{baseline,optimized}[-2,-3]/`.

## Validation and build provenance

The native release workspace suite with bundled UI passes, as does strict workspace/all-target Clippy. Regression tests compare reused and fresh analysis including polygons, check exact-motion invalidation and appended-duplicate fallback, and cover containment at holes, boundaries, subgrid coordinates, and invalid query coordinates. Existing continuous-clearance, stock, finite-tip, artifact-replay, and verification tests remain in the suite. Timing instrumentation confirms that original-flower planning constructs nine combined stock slices in total, including cleanup, instead of ten.

The packaged HTTP service and browser adapter generate the original flower plan and then conclusively verify it on three fresh requests in **7.271, 7.311, and 7.287 seconds**. All reports pass with 519,395 evaluated cells, zero unresolved cells, and maximum-error uncertainty `0.04675631823681522 mm`, below the unchanged 0.05 mm tolerance. Input and motion fingerprints match the earlier conclusive verification. Reports and measurements are in `flat-v-carve/artifacts/planner-verification-backport-web/`.

Validation uses an isolated source snapshot of `f14368477be363facfc90d4defd20fa9431494d8` plus only these planner changes and the performance test's explicit input-path option. Concurrent export edits in the shared working tree are excluded from this snapshot and are preserved in place. A subsequent workspace/all-target `cargo check` also passes in the shared working tree with those concurrent edits present.

- Baseline executable: `flat-v-carve/artifacts/portable/cam.exe`, SHA-256 `EBACE452A7C15D33C35DBA987723C934358092960DC6BBC096698C284963EDFB`.
- Optimized executable: `flat-v-carve/artifacts/planner-verification-backport/cam.exe`, SHA-256 `40954C1C81CFCB49B2C3E9F51C1DDDFFBF97C283DA1E359AE45116CF2EB7587C`.
- Frozen original job: `flat-v-carve/artifacts/planner-verification-backport-baseline/baseline.job.json`.
- Raw timings, plans, hashes, and resource measurements: `flat-v-carve/artifacts/planner-backport-ab-{baseline,optimized}-{1,2,3}/`.
- Source-file hashes and aggregate measurements: `flat-v-carve/artifacts/planner-verification-backport/{build-info,comparison}.json`.

From `flat-v-carve`, choose a new output directory for each measurement:

```powershell
./scripts/benchmark-flower.ps1 `
  -Job ./artifacts/planner-verification-backport-baseline/baseline.job.json `
  -Cam ./artifacts/planner-verification-backport/cam.exe `
  -Stages combined -OutputDirectory ./artifacts/planner-backport-repeat
```

The opt-in browser-adapter test accepts `CAM_FLOWER_JOB` so the original frozen input can be selected explicitly after the user's working job is renamed. From `flat-v-carve/web`:

```powershell
$env:CAM_TEST_EXE = 'D:\proj1\flat-v-carve\artifacts\planner-verification-backport\cam.exe'
$env:CAM_FLOWER_VERIFY = '1'
$env:CAM_FLOWER_JOB = 'D:\proj1\flat-v-carve\artifacts\planner-verification-backport-baseline\baseline.job.json'
$env:CAM_FLOWER_OUTPUT = 'D:\proj1\flat-v-carve\artifacts\planner-backport-web-repeat'
node ./node_modules/vitest/vitest.mjs run integration/flower-verification.test.ts
```
