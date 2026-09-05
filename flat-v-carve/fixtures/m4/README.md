# M4 combined planner fixtures

All tool dimensions, feeds, spindle speeds, and capability flags are **synthetic software-test inputs**, not machining defaults or recommendations. These schema-3 jobs embed their SVG artwork and extend the M3 cases with explicit V-bit cutting settings, ridge/detail limits, and planning budgets.

| Fixture | Expected result | Focus |
| --- | --- | --- |
| `wide-floor.json` | complete | Endmill + pointed V-bit, floor lanes and retained wall finish. |
| `island.json` | complete | Both outer and island boundaries, actual combined stock. |
| `disconnected.json` | complete | All connecting travel at clearance. |
| `narrow-channel.json` | complete | Empty endmill stage; V-bit depth passes and rising terminals. |
| `finite-tip.json` | complete | 1 mm flat tip, 0.5 mm allowed cutter-limited detail. |
| `ramp-roughing.json` | complete | M3 ramps contribute to combined stock before V-bit finishing. |
| `exact-fit.json` | complete | M3's exact-fit endmill case completed by the V-bit. |
| `curved-medial.json` | complete | L-shaped region, 4.5 mm cap, curved branches and cap transitions. |
| `contact-line.json` | complete | Pointed V-bit centerline at the nominal cap; zero ridge needs no area clearing. |
| `contact-point.json` | complete | Isolated cap contact retained as an explicit plunge execution. |
| `unsupported-entry.json` | incomplete | V-bit explicitly does not support M4's plunge entry. |
| `resource-limit.json` | inconclusive | Motion budget exhausted; complete excursions retained. |
| `zero-ridge.json` | rejected | A pointed tool cannot clear remaining floor area with zero ridge. |

Most fixtures use a 2 mm cap, 90° V-bit, 1 mm V-bit stepdown, 0.5 mm maximum V-bit stepover, 0.15 mm allowed floor ridge, 0 mm detail limit, 0.005 mm geometry/motion tolerance, and 0.05 mm verification tolerance. The planner limits actual lane spacing using the ridge formula and reserves explicit numerical budgets. A zero nominal tip is an idealized cutter used to test the model.

`vbit_planning` explicitly sets candidate/motion/curve/pass limits, at most two cleanup iterations, a 1 mm sample lattice, 10,000 quality samples, 4,096 reachability cells, and three nominal stock slices. Actual motions also contribute sample witnesses. Samples and fixed slices do not prove global finish quality; that is M5 work.

```sh
cargo build --release --locked -p cam-app
target/release/cam plan fixtures/m4/curved-medial.json --output artifacts/m4/curved-medial/plan.json
target/release/cam inspect artifacts/m4/curved-medial/plan.json \
  --output artifacts/m4/curved-medial/preview.svg --report artifacts/m4/curved-medial/report.json
target/release/cam verify artifacts/m4/curved-medial/plan.json --output artifacts/m4/curved-medial/verification.json
```

With `vbit_planning` configured, `plan` selects the combined pipeline. `--stage endmill` still generates the M3 stage alone; `--stage combined` explicitly requires M4 settings. Complete/empty stages exit 0, incomplete/inconclusive stages exit 1 while retaining their plan, and rejected inputs write failure diagnostics with exit 1. Argument/I/O errors exit 2. Check status before consuming output.

The integration tests additionally remove floor lanes, depth passes, and final finishing, alter tool order/feeds, inject gouges and stale artifacts, tighten detail limits, and compare analytic ridges, tapered capsule area, continuous curve approximation, and point-removal extrema.
