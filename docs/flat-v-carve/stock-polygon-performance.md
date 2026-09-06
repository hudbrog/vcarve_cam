# Stock-polygon construction

The V-bit stock builder now distributes work within each depth slice as well as
across slices. The previous planner used up to eight slice workers, but a shallow
slice's large polygon union was still serial and could dominate the final wait.
Cleanup also constructs a single floor slice, which benefits from this change.

## Geometry contract

The original spatial order, duplicate-footprint filtering, capsule construction,
eight-footprint batches, and balanced Boolean tree are retained. Changing batch
sizes changed rounding in experiments, so those variants were rejected.

Each worker constructs an aligned subtree of 512 footprints (64 original
batches). A variable capsule has at most 2,048 vertices, so eight capsules cannot
reach the accumulator's 32,768-vertex batch limit. The parent appends unfinished
accumulators at their original tree levels; it does not finalize or re-normalize
intermediate subtrees. A final partial block retains its unfinished batch.
Consequently every successful Boolean operation has the same operands and order
as before, independently of worker count.

Jobs interleave depths to keep both capsule construction and larger merges busy.
At most eight workers are active, at most eight slices have prepared sweeps, and
each wave contains at most twice the worker count in blocks. Completed geometry
is merged before the next wave starts. Errors from V-bit slices are returned in
input-depth order, including when a later slice fails during preparation.

The planner obtains all requested V-bit slices through this scheduler, then does
the existing endmill/nominal-section comparisons. The single-depth API uses the
same scheduler. Slice counts, bounds, contributing motion IDs, tolerance and
resource limits, cleanup decisions, plan status, and continuous verification
criteria are unchanged. This is a scheduling optimization: it does not claim to
remove Boolean operations or make M4 stock polygons a formal interval proof.

## Planner measurements

Measured on 2026-09-06 on a Windows host with 16 logical processors. Three
sequential, interleaved pairs per job use the same inputs and build settings;
pair 2 runs optimized first. No benchmark is run concurrently with another
benchmark or a build/test started by this task.

| Job | Pair | Baseline elapsed | Optimized elapsed | Baseline CPU | Optimized CPU |
| --- | --- | ---: | ---: | ---: | ---: |
| Original | 1 | 27.162 s | 24.937 s | 82.141 s | 84.547 s |
| Original | 2 | 28.991 s | 24.817 s | 85.672 s | 85.297 s |
| Original | 3 | 20.247 s | 24.902 s | 59.297 s | 84.875 s |
| Original | Median | 27.162 s | 24.902 s | 82.141 s | 84.875 s |
| Newer | 1 | 8.321 s | 6.502 s | 25.688 s | 21.063 s |
| Newer | 2 | 7.480 s | 6.467 s | 21.172 s | 20.641 s |
| Newer | 3 | 6.837 s | 6.385 s | 19.859 s | 20.844 s |
| Newer | Median | 7.480 s | 6.467 s | 21.172 s | 20.844 s |

The newer job improves in every pair, with a 13.5% reduction in median elapsed
time. Original-job results are mixed: the median is 8.3% lower, but pair 3 is
slower after the change. That baseline run also has much faster unchanged
candidate construction (4.207 s versus 6.709 s), so these observations do not
establish a reliable original-job end-to-end speedup. Both comparisons retain
all runs rather than selecting the fastest observations.

Parallel scheduling retains the original polygon work and adds coordination.
Median CPU time increases 3.3% for the original job and falls 1.5% for the newer
job; CPU reduction is not the claimed mechanism. Sampled peak working sets are
192.9–200.7 MiB baseline versus 225.9–228.6 MiB optimized for the original job,
and 115.2–117.6 MiB versus 128.6–133.8 MiB for the newer job. The bounded extra
subtrees and retained V-bit slices trade memory for concurrency.

All twelve runs report `Complete` and preserve the complete serialized plan
bytes. Original plan SHA-256:
`BC166699C4C57F29100FF81BFDB66B62EC1DE90375B4F0374DAC2AC7F80B015F`
(14,054,720 bytes, 17,063 endmill and 28,501 V-bit motions). Newer plan SHA-256:
`34BA8A77E9E38BF58E8A19E42602B0FC12E9E8EF49ED0207470DDC6F6DB9AFC1`
(7,614,481 bytes). Stock analysis is not serialized in those plans, so polygon
equivalence is checked separately below.

Raw plans, timings, hashes, and resource measurements are under
`flat-v-carve/artifacts/stock-union-study/paired-{original,current}-{baseline,optimized}-{1,2,3}/`.
`comparison.json` contains the measurements and top-level stage timings.

Original-job median final analysis falls from 11.278 to 9.028 s and cleanup
from 1.661 to 1.323 s. Newer-job cleanup falls from 1.473 to 0.582 s, while
final analysis is essentially unchanged (2.470 versus 2.480 s). The existing
parallelism across depths already handles much of the newer final-analysis
work; the single cleanup slice benefits more directly.

## Direct stock comparison

All nine original-job slices and all eight newer-job slices were reconstructed
from the same saved motions with both implementations. The full serialized
`SliceRemoval` matches byte for byte at every depth, including lower and upper
polygons, contributing-motion IDs, grid, depth, and radial-error bound. This
checks real artwork in addition to the sequential-reference regression matrix.

Representative single observations from the diagnostic example:

| Job | Depth | Baseline stock construction | Optimized stock construction |
| --- | ---: | ---: | ---: |
| Original | 0.250 mm | 5.624 s | 1.514 s |
| Original, cleanup floor | 1.875 mm | 1.783 s | 0.527 s |
| Newer | 0.125 mm | 1.671 s | 0.501 s |
| Newer, cleanup floor | 0.875 mm | 1.208 s | 0.300 s |

These isolated slices benefit much more than the full planner, which also
performs endmill planning, candidate generation, stock/tool comparisons, and
other work. They are single observations, not repeated latency guarantees.
Full output and per-depth timings are under
`flat-v-carve/artifacts/stock-union-study/slices-{original,current}-{baseline,optimized}/`;
`slice-comparison.json` records all seventeen hashes and timings.

## Validation and provenance

Both native Windows x64 release builds use static CRT, bundled UI, and committed
base `dd2847393c7bc8838d32e749fdf9efa88f243739`. The optimized snapshot adds only
the stock builder, accumulator append, quality integration, and diagnostic
example described here. Separate target directories prevent build artifacts
from crossing between the two source snapshots.

- Baseline app SHA-256: `15ED5ED8BCB725B9E59DB22BE1934ED1E4AEF7537C7516B516ABB44AF38AA874`.
- Optimized app SHA-256: `E03F609B7BEB959827D0C133068738B98EE9289B2F09F8D084A97699D64F55AF`.
- Original flower input SHA-256: `59E4C9DEB37CC3F0A335EAB1F4C53FD86257263B476B96260DA560223C6EA693`.
- Newer flower input SHA-256: `80DD0208CEC4D2622D5C5719A96E16EF85D4C2A82E8E72DAE8C2C1ECAA552AE7`.

The original job uses 0.001 mm geometry tolerance, a 2 mm depth cap, and a
pointed tip. The newer job uses 0.005 mm geometry tolerance, a 1 mm depth cap,
and a 0.1 mm tip diameter. Their settings are preserved.

The full release workspace suite with bundled UI passes, as does strict
workspace/all-target Clippy. New regressions compare complete serialized stock
against the former sequential algorithm with 1, 2, and 8 workers. They cover
pointed and finite tips, varying Z, holes and disconnected components, duplicate
sweeps, plunges, partial blocks, more than eight depths, unsorted depth order,
empty input, and preparation/construction error ordering. Invalid accumulator
alignment is rejected. Existing planner, continuous-clearance, stock, artifact
replay, API, and verification tests also pass.

The packaged HTTP service and browser adapter generate the original flower
plan and then perform three fresh continuous verifications. All pass in 2.209,
2.203, and 2.190 s, with 519,395 evaluated cells, zero unresolved cells, no
findings, and maximum-error uncertainty `0.04675631823681522 mm`, below the
unchanged 0.05 mm tolerance. Input and motion fingerprints match the prior
conclusive reports. Measurements and full reports are under
`flat-v-carve/artifacts/stock-union-study/web-verification/`. This validates
compatibility; the base revision already includes separate M5 parallelism, so
these times are not attributed to this stock optimization. No new continuous
verification result for the newer flower settings is claimed here.

Build hashes are recorded in
`flat-v-carve/artifacts/stock-union-study/build-info.json`. Early runs named
`final-*` used an incorrectly reused build artifact and are explicitly excluded;
`discarded-runs.json` records that issue. Only the separately built `paired-*`
runs are used for the final comparison, and each run's timing output is checked
for the expected old or new implementation.

## Reproduction

From `flat-v-carve`, run each planner measurement in a new output directory:

```powershell
./scripts/benchmark-flower.ps1 `
  -Job ./artifacts/planner-verification-backport-baseline/baseline.job.json `
  -Cam ./artifacts/stock-union-study/cam-optimized.exe `
  -Stages combined -OutputDirectory ./artifacts/stock-union-repeat
```

The diagnostic example reconstructs stock directly from saved motion records,
without replanning or treating those records as authenticated verification
evidence. It writes complete slice JSON and reports geometry hashes, bounds,
vertex counts, contributing-motion counts, and elapsed time:

```powershell
cargo run --release --locked -p cam-core --example benchmark_vbit_slices -- `
  ./artifacts/planner-backport-ab-optimized-3/combined.plan.json `
  ./artifacts/stock-slice-repeat '0.25,0.5,0.75,1,1.25,1.5,1.75,1.875,2'
```

The example processes requested depths individually. Planner timings measure
the actual scheduler across depths and include planning, serialization, and
file writing.
