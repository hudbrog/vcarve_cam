# Sketcher feasibility harness

Isolated experiment for [the feasibility decision](../../../docs/flat-v-carve/artwork-sketcher-feasibility.md).
No production workspace member, saved schema or application dependency is changed.
`results/qualification.json` contains the measured run; raw outputs and downloaded
inputs live under ignored `../../artifacts/sketcher-feasibility/`.

## Reproduce on Windows

Requirements: Python 3.12+, Node with ES modules, Rust 1.95.0 with
`wasm32-unknown-unknown`, MSVC C++ build tools and Windows SDK, and
`wasm-bindgen-cli` **0.2.128**. Run from this directory:

```powershell
python prepare.py
./run-native.ps1 -WasmBindgen 'path/to/wasm-bindgen.exe'
node serve.mjs
```

`prepare.py` downloads official pinned archives and checks their SHA-256 hashes.
It extracts only the required Boost headers/license, alongside the other source
packages. The native build uses the existing C++ installation; it does not install
toolchains. Cargo dependencies are pinned in this experiment's own lockfile.
The browser uses the published PlaneGCS artifact, not a locally reproduced
Emscripten build. See the report for that provenance limitation.

Open `http://127.0.0.1:5193/` in an actual browser and click **Run native/browser
comparison fixtures**. Wait for **Complete — results saved**. Then, from a second
terminal in this directory:

```powershell
node verify.mjs
cargo fmt --check
cargo clippy --release --locked --no-deps -- -D warnings
```

The server binds loopback only. Its POST route accepts four named experiment
result files. It never sends repository content to an external service.
Stop it with Ctrl+C when finished.

`verify.mjs` rejects missing/duplicate cases, unintended acceptance, excessive
residuals, wrong DOF, determined native/browser branch changes, failed geometry
checks and stale-reply/cancellation regressions. The current PlaneGCS browser
`scale-500` OOM and unrefined Arael coincidence miss are explicit characterizations,
not silently passing qualifications. Change those expectations when retesting
an improved backend; do not mistake the verifier's exit code for a production
release approval. `null` in a residual field means a non-finite residual and is
always accompanied by `accepted: false`.

## Files and scope

- `fixtures.mjs` generates the same dimensional systems for every target.
- `src/lib.rs` maps the neutral protocol to Arael, runs solves/refinement/drag.
- `native/main.cpp` maps it to the pinned native PlaneGCS sources.
- `plane.mjs` maps it to the published WASM API; `worker.mjs` isolates execution.
- `validate.mjs` evaluates geometric residuals independently of solver costs.
- `src/geometry.rs` checks existing CAM geometry plus prototype provenance rules.
- `lifecycle-native.mjs` tests helper-process termination; this does not implement
  cooperative native-thread cancellation.

This is trusted-fixture code. Protocol inputs are not an external file parser;
operand types and indices are assumed valid. Geometry assertions intentionally
fail the test on unsupported topology. A production boundary needs validated,
bounded inputs and located errors rather than these fixture assertions.

Browser warm timings use 21 edits, or five for scaling/difficult-initial cases.
Each fixture starts a fresh worker; cold solve timings include JIT effects but
exclude module-ready time. Arael cold timings include a bounded public-API
refinement; warm timings use a reused symbolic-factorization session. Native
PlaneGCS diagnostic timing includes an additional explicit diagnosis while the
browser field measures getters. The report explains these differences.

Downloaded licenses remain in their upstream directories. Arael's solver declares
MIT; PlaneGCS source headers declare LGPL-2.1-or-later, while its npm metadata
differs. Eigen, Boost and nlohmann/json retain their own notices. No third-party
source or solver binary is committed by this experiment.
