# M3 endmill fixtures

These portable schema-2 jobs embed simple SVG artwork. **All dimensions, feeds, spindle speeds, and capability flags are synthetic software test inputs.** They are not machining defaults or cutting recommendations.

The main cases use a 4 mm endmill, a 90° target angle, 2 mm depth in 1 mm steps, 1.5 mm stepover, 0.5 mm horizontal allowance, 5 mm clearance Z, 0.005 mm geometry/motion tolerance, and 0.05 mm slice coverage tolerance. Only the ramp fixture enables ramp capability; its entry explicitly requests 5° and 100 mm/min. V-bit cutting settings remain unset because M3 generates only the endmill stage.

| Job | Expected status | Purpose |
| --- | --- | --- |
| `rectangle.json` | complete | Broad floors and depth-dependent upper clearing. |
| `island.json` | complete | Preserved hole, round offsets, independent segment checks. |
| `disconnected.json` | complete | Two regions with all connecting XY travel at clearance. |
| `ramp.json` | complete | Non-plunge tool with explicit ramp capability and feed. |
| `deepest-region.json` | complete | Same deepest center region used at every depth pass. |
| `no-access.json` | empty | Channel narrower than the endmill even at shallow depth. |
| `exact-fit.json` | inconclusive | A positive-area shallow pass and zero-margin final centerline. |
| `narrow-margin.json` | inconclusive | Positive center area disappears under the numerical guard. |
| `unsupported-entry.json` | incomplete | Plunge requested for a non-plunge tool. |
| `resource-limit.json` | inconclusive | Partial loops retained when the loop budget is exhausted. |

Run from the workspace root:

```sh
cargo build --release --locked -p cam-app
target/release/cam plan fixtures/m3/island.json --output artifacts/m3/island/plan.json
target/release/cam inspect artifacts/m3/island/plan.json \
  --output artifacts/m3/island/preview.svg --report artifacts/m3/island/report.json
target/release/cam verify artifacts/m3/island/plan.json --output artifacts/m3/island/verification.json
```

Complete and empty stages exit 0; incomplete and inconclusive stages write their artifacts and exit 1. Check exit status before consuming output. `inspect` displays up to 16 slices; the JSON retains every slice. The integration tests also inject island crossings, allowance violations, malformed moves, deleted loops, invalid settings, and stale fingerprints.
