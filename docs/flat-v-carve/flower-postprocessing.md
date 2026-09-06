# Flower G-code generation

The `real_data/flower_box-svg.job-real.json` failure is reproduced with
`real_data/machine-profile.json`: original stock verification passes, then
the emitted-program reader rejects motion 122 with `POST_ROUNDING`.

That endmill cut runs from X39.13473125 to X39.1347375 at Y30.979125,
Z−1: its length is 0.00000625 mm. Both X coordinates become 39.135 at
the profile's three-decimal precision. Visualization and default verification
use original coordinates, so their success does not establish that this formatted
program preserves its motions.

## Output behavior

Profile `decimal_places` now specifies the minimum coordinate precision.
Export chooses the first precision through nine decimal places that preserves
every segment's direction, cutting XY travel, and required approach/plunge/ramp/
retract Z travel. Selection happens after stock-datum translation. Both flower
jobs need six decimal places. The source profile remains unchanged; the report
contains `output_decimal_places` and a `POST_PRECISION_INCREASED` warning, and
the output panel shows the effective precision.

Every source motion still has an output block. The independent numeric reader
checks the actual endpoints, feeds, modal state and tool changes. Failure at the
maximum precision still returns no programs. Both original and emitted stock
verification must pass; machining tolerances and verification budgets are unchanged.
Safety planes, stock thickness and declared machine positions must still be
representable at the profile precision.

## Performance changes

- Browser export authenticates its retained plan using the existing service-owned
  planning receipt. The receipt comes from the private task ledger, not the HTTP
  request or imported artifact. Job settings, both motion lists, stage transition,
  execution records and generation issues remain fingerprint-checked. Portable
  plan imports retain full reconstruction. CLI export avoids doing that reconstruction twice.
- Independent verification cells run in bounded batches on at most eight workers.
  Spatially adjacent cells are interleaved between workers to distribute expensive
  boundary queries. The coordinator preserves deterministic result order and
  accounts for all pending cells in the original global refinement budget.
- Signed-distance bounds identify cells that are entirely exterior or on the deep
  flat floor before running exact edge/rectangle searches. Other cells retain those
  searches. Box-distance queries avoid general hypotenuse calculations for overlapping
  boxes and axis-aligned gaps, retaining the same arithmetic reserves.
- Validated cutter angles cache their immutable slope, avoiding repeated tangent
  calculations in analytic stock queries. Serialized angle values remain degrees.
- G-code XYZ values are formatted once instead of formatting, parsing, and formatting again.

## Measurements and reproduction

Measurements use the native Windows x64 portable release build, engine 0.7.5.
The timer starts at export submission and ends after the browser HTTP adapter
has fetched and checked the report and all program bytes. Planning is excluded.
Each job is exported twice as one combined program and once as per-tool files.

The final packaged build, using the same 200 ms completion polling as the UI:

| Saved job | Combined 1 | Combined 2 | Per-tool | Checked motions |
| --- | ---: | ---: | ---: | ---: |
| job-real | 4.74 s | 4.65 s | 4.63 s | 28,404 |
| Original flower | 4.92 s | 4.91 s | 4.88 s | 45,564 |

All six outcomes passed at six decimal places with zero unresolved cells and
every original motion retained. Raw results are in `release-job-real/` and
`release-original/`. The UI previously polled at 700 ms.
The executable SHA-256 is
`7c6668213a5cf028d34f6daaa75f6e25c07c69038c8c6d77f783f9b23560366f`.

An earlier run of the final computation build measured **4.60, 4.92, and
7.11 seconds** for job-real; the last exceeded the benchmark's five-second
assertion even though geometry and program verification passed. Raw captures
are retained in `accepted-job-real/` alongside the successful repeat in
`job-real-repeat/`. These are local wall-clock measurements, not a latency
guarantee on every machine or under other workloads. Before parallel verification,
the retained-plan export path took 13.18–16.30 seconds in three runs. The old
export additionally rebuilt planning previews before eventually failing on rounding.

The machine profile supplied for job-real uses 5 mm clearance. The original
flower snapshot uses **30 mm** clearance, so its benchmark uses a separate copy
of that profile with clearance set to 30 mm. Export correctly rejects using
the 5 mm profile with that original job. Neither job's settings nor the supplied
machine-profile file are edited.

Detailed timings, input hashes, reports and checked G-code are retained under
`flat-v-carve/artifacts/postprocessing-flower/`. The original snapshot is recovered
from the tracked `flower_box-svg.job (2).json`; the benchmark copy has normalized
line endings. The rebuilt portable application is in `portable/cam.exe` under that
artifact directory.

From `flat-v-carve/web`, run the actual portable service regression:

```powershell
$env:CAM_FLOWER_EXPORT = '1'
$env:CAM_TEST_EXE = 'D:\proj1\flat-v-carve\artifacts\postprocessing-flower\portable\cam.exe'
$env:CAM_FLOWER_OUTPUT = 'D:\proj1\flat-v-carve\artifacts\postprocessing-flower\new-measurement'
node node_modules/vitest/vitest.mjs run integration/flower-export.test.ts
```

The default regression uses job-real and the supplied machine profile and requires
each export to finish in under five seconds. `CAM_FLOWER_JOB` and
`CAM_FLOWER_PROFILE` select another saved job and matching profile;
`CAM_FLOWER_EXPORT_SECONDS` changes only the benchmark assertion, never a machining
or verification setting. Native CLI stage profiling remains available with
`CAM_TIMINGS=1`.

Regression coverage includes the actual motion-122 coordinates, required Z travel,
rejection when nine decimals cannot preserve a move, retained-versus-reconstructed
export equivalence, changed-artifact rejection, deterministic serial/parallel
cell results, resource exhaustion, controller-state/readback mutations, both
program layouts, and matching CLI/HTTP reports. Tests verify modeled geometry
and output bytes; they do not run a controller or machine.

Validation passed: 242 Rust tests (one opt-in benchmark ignored), strict Clippy
across the workspace/all targets, formatting, 100 frontend tests, TypeScript and
schema/display contract checks. The portable CLI/HTTP suite passes 27 tests
(two unrelated opt-in large-artwork cases skipped), in addition to the two
three-export flower performance runs above.
