# M5 verification fixtures

`cases.json` reuses the explicit synthetic machining settings from M4. It specifies verification arguments and expected M5 results; it does not supply machine defaults.

| Cases | Expected result | Evidence |
| --- | --- | --- |
| narrow-channel, wide-floor, island, finite-tip, curved-medial, disconnected | passed | Original and six-place formatted coordinates satisfy bounded stock/quality checks. |
| contact-line, contact-point | failed | Six-place cap witnesses leave about 0.01 mm against a strict zero-ridge request; M4's preview allowance is not an M5 pass. |
| cell-limit | inconclusive | One cell cannot establish whole-surface coverage. |
| rounding-coarse | failed | Zero decimal places collapse required movements and change removal. |

From the Rust workspace in PowerShell:

```powershell
cargo build --release --workspace --locked
.\scripts\benchmark-m5.ps1
```

Each case creates `plan.json`, `verification.json`, `verification.svg`, and process logs under `artifacts/m5/<case>/`. `artifacts/m5/benchmark.json` records timings, observed process memory, segment counts, artifact sizes, and whether the expected status/exit code matched. The script exits unsuccessfully if an expectation changes.

The core tests additionally inject continuous gouges, remove one floor lane while preserving valid motion continuity, lower reachable paths, tighten finite-tip detail limits, change tools, and challenge depth/area bounds against analytic cases. See the [M5 capability report](../../../docs/flat-v-carve/m5-capability-report.md) for formulas, measurements, and limits.
