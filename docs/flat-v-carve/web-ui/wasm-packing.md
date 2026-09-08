# WebAssembly packing investigation

Date: 2026-09-08\
Status: Implemented by [U9](u9-wasm-static.md); this document records the feasibility evidence that preceded it. No production code was changed for this report.

This report answers the packaging question deferred in the [architecture](../../architecture.md): can the planning pass run in the browser so the web UI can be statically hosted without the local `cam-web` service? The answer is yes, with measured evidence below. The native portable application remains unchanged; a static build would be an additional deployment of the same `cam-core`.

## 1. Summary

| Question | Result | Evidence |
| --- | --- | --- |
| Does `cam-core` compile for the browser target? | Yes, both `wasm32-unknown-unknown` and `wasm32-wasip1`, Rust 1.95.0, pinned `Cargo.lock` | §3 |
| Any dependency blockers? | One opt-in: `getrandom` needs its `wasm_js` backend (pulled by `boostvoronoi` → `cpp_map` → `rand`) | §3 |
| Does it run on real artwork? | Yes. The unchanged `flower_box-svg.job-real.json` plans and verifies end-to-end single-threaded | §4 |
| Engine changes required | Three serial-fallback guards where `thread::scope` spawns even with one worker | §5 |
| Size | 221 KB raw / 91 KB gzipped with size optimization; 1.87 MB / 625 KB with default release settings | §6 |
| Speed | Endmill 1.7 s, combined 10.7 s, M5 verification 16.7 s single-threaded (native multithreaded combined: 2.7 s) | §4 |
| Static hosting | Works on any static host, including GitHub Pages, without cross-origin isolation | §7 |

## 2. What moves into the browser and what stays native

The browser UI already programs against the `CamService` interface ([service contract](../../../flat-v-carve/web/src/contracts/service.ts)) with two implementations: `fixtureService` and `createHttpService`. Selection happens in one line ([main.tsx](../../../flat-v-carve/web/src/main.tsx)): development and `?mode=fixture` use fixtures, everything else uses HTTP against `/api/v1`. A static deployment adds a third implementation, `createWasmService()`, backed by a Web Worker; the UI, state, contracts, and simulator are unchanged.

| Component | Static-web role |
| --- | --- |
| `cam-core` | Compiled to wasm and loaded in a dedicated Web Worker. All planning, verification, export postprocessing, and SVG import run there. |
| `cam-server` | Not used. Its compute seam — `planning_worker.rs`'s `calculate(input) -> Output` over stdin/stdout with bounded IPC — maps one-to-one onto worker `postMessage` with the same bounded payloads. |
| `cam-storage` / plan files | Replaced by worker memory. The flower plan is about 111 MB of JSON on disk ([plan storage report](u7-plan-artifacts.md)); as live Rust structures inside the worker it needs no serialization, and the existing paged-motion protocol transfers previews in bounded messages. |
| `cam-app` (CLI, portable exe) | Unchanged. Native distribution continues to work exactly as today. |

Tool-library storage (currently a local JSON file addressed by path) would move to IndexedDB or `localStorage` under the same snapshot schema. Cancellation semantics are preserved: the service cancels its compute process today, and the wasm service terminates its worker, discarding the same in-flight state.

## 3. Compile evidence

A scratch crate depending on `cam-core` by path was built with the pinned toolchain and `Cargo.lock` (workspace untouched):

- `cargo build --release --target wasm32-unknown-unknown` fails in `getrandom` 0.3.4 by design: the crate refuses wasm32-unknown-unknown unless its JS backend is enabled. The dependency chain is `boostvoronoi` 0.12.1 → `cpp_map` 0.2.0 → `rand` 0.9.5 → `rand_core` → `getrandom`. This randomness is genuinely used at runtime (the cpp_map skip list draws coin flips from `ThreadRng`), not test-only.
- Enabling `getrandom`'s `wasm_js` feature plus `--cfg getrandom_wasm_js` (via `[target.wasm32-unknown-unknown] rustflags` in `.cargo/config.toml`) compiles the entire tree with no other changes. The backend imports `crypto.getRandomValues` through wasm-bindgen glue, so the browser artifact must be produced with wasm-bindgen tooling (wasm-pack targeting `web`); Vite already in use for the UI consumes that output.
- `wasm32-wasip1` builds with no flags at all (WASI provides randomness and clocks natively), which is what the runtime measurements used.

Two std behaviors were verified at runtime rather than assumed:

- `std::env::var_os("CAM_TIMINGS")` returns `None` on wasm32-unknown-unknown without trapping, so `cam-core`'s stage `Timer` stays disabled. This matters because `Instant::now()` **does** trap on wasm32-unknown-unknown (verified with a minimal module; it works on wasip1). No reachable `cam-core` path calls `Instant` when the timer is disabled; browser stage timings, if ever wanted, would use a `performance.now()` shim instead.
- `std::thread::available_parallelism()` reports unsupported, so every `map_or(1, …)` computation yields one worker. Sites that compare against a threshold (`> 1`, `== 0`) degrade to their serial path automatically; three sites did not, and are listed in §5.

`cam-core` contains no filesystem, clock, or environment use beyond the gated timer, matching its declared boundary. `unsafe_code` remains forbidden workspace-wide.

## 4. Runtime measurements

The real saved job `real_data/flower_box-svg.job-real.json` was embedded in the spike binary and executed under Node 24's WebAssembly runtime (V8, the same engine family as Chrome) on `wasm32-wasip1`, release profile, single-threaded, after applying the §5 fixes. Three consecutive runs:

| Stage | Result | Time (3 runs) |
| --- | --- | --- |
| Endmill planning | 7,048 motions, 0 generation issues | 1.67 / 1.67 / 1.69 s |
| Combined planning | 7,048 endmill + 15,845 V-bit motions, status `Complete` | 10.65 / 10.65 / 10.68 s |
| M5 verification (default options) | `Passed`, 0 findings | 16.64 / 16.65 / 16.66 s |
| Module compile + instantiate | 1.8 MB module | 4 ms |

Native reference on the measured Windows machine is 2.71–2.73 s for combined planning across all cores ([three-second benchmark](../flower-performance-3s.md)). Single-threaded wasm is therefore about 4× slower on the heavy stage. Two mitigations exist if that ever matters, both out of scope for a first static deployment: size/speed profile tuning (the perf numbers above use the default `opt-level=3`; the size numbers in §6 use `opt-level=z`), and true threading, which requires the §7 headers and a parallelism refactor.

Before the §5 fixes, the same run panicked with `failed to spawn thread: Not supported` — once during endmill planning and again, further in, during combined planning — confirming the audit below.

## 5. Required engine changes

Every `std::thread::scope` site in `cam-core` was audited for a serial path when `available_parallelism()` yields one worker. Most already have one (`workers == 1` / `partitions == 1` / `parallel` flag guards in `stock.rs`, `stock/chains.rs`, `stock/slices.rs`, `vcarve/rest.rs`, `vcarve/mod.rs`, and the remaining `vcarve/quality.rs` sites; the `target/mod.rs` site is test-only). Three sites spawn even with a single worker and panic on wasm; all three fixes preserve native behavior because they only change the `workers <= 1` case, which previously spawned one thread to run the same work:

1. `geometry/polygon.rs` — `Region::erode()` multi-component branch: guard the `thread::scope` behind `workers > 1` and run the same per-component offsets inline otherwise. Triggered by the flower job during endmill planning.
2. `vcarve/quality.rs` — `streamed_slices()`: add a `workers == 1` branch that evaluates depths in order without spawning. Triggered by large V-bit motion sets during combined planning.
3. `verification/adaptive.rs` — worker selection: change `available_parallelism().map_or(1, usize::from)` to `map_or(0, …)` so a host that cannot report parallelism runs the coordinator-only evaluation (the `Evaluator` already handles zero workers serially).

The verified diff (123 lines, exercised by the §4 runs, then reverted from the working tree) is saved at `.worktrees/wasm-spike/serial-fallback.patch` outside version control; the three changes are small enough to reapply from the descriptions above in the first commit of an implementation milestone. Deterministic partitioning comments at these sites remain true: the serial path merges results in the same input order.

One build-side addition is needed: the workspace (or a new `cam-wasm` crate) declares

```toml
[target.'cfg(target_arch = "wasm32")'.dependencies]
getrandom = { version = "=0.3.4", features = ["wasm_js"] }
```

with `[target.wasm32-unknown-unknown] rustflags = ["--cfg", "getrandom_wasm_js"]` in `.cargo/config.toml`. This is inert on native targets.

## 6. Size

The spike binary — which links `Job::from_json`, `plan_endmill`, `plan_combined_with_receipt`, and `verify_plan`, keeping the whole planner and verifier alive — measures:

| Profile | Raw | Gzipped (transfer) |
| --- | --- | --- |
| Default release (`opt-level=3`) | 1.87 MB | 625 KB |
| `opt-level=z`, LTO, 1 codegen unit, `panic=abort`, stripped | 221 KB | 91 KB |

The aggressive profile trades some speed for size; `opt-level=s` or wasm-opt `-O3` postprocessing are middle grounds. Either way the artifact is comfortably static-hostable, and instantiation is negligible (4 ms measured).

## 7. Static hosting constraints

- **No cross-origin isolation required.** The measured configuration is single-threaded. Any static host serves it: GitHub Pages, Cloudflare Pages, Netlify, or a plain directory. The existing Vite build produces the assets; the wasm worker and its glue join `web/dist`.
- **Threads are a separate, later step.** Restoring parallelism needs `SharedArrayBuffer`, which needs COOP/COEP response headers — not settable on GitHub Pages, settable via `_headers` on Cloudflare/Netlify. It also needs more than flags: `std::thread::scope` cannot become a browser thread pool as-is; the parallel sites would move to rayon-style scoped parallelism (`wasm-bindgen-rayon`). The §5 fixes are a prerequisite either way.
- **Deployment shape.** A static build is additive: the portable `cam.exe` keeps serving the bundled UI locally, while the same UI build gains a wasm mode selected in `main.tsx` (for example `?mode=wasm`, or automatic when `/api/v1/session` is absent). The `Capabilities.mode` field already distinguishes fixture from live; a wasm mode reports its own limits and the same `engineVersion` from `cam-core`, preserving the UI's version-mismatch checks.

## 8. Risks and open questions

- Browser memory for large artwork: the worker holds the full plan as Rust structures instead of the service's temp-file spill. The flower job fits comfortably on desktop-class browsers; a hard cap with an explicit diagnostic (mirroring `PLAN_RESULT_LIMIT`) should replace the current implicit assumption.
- `Instant` remains unavailable on wasm32-unknown-unknown; any future core code must keep time use behind the `Timer` gate or a wasm-aware clock shim.
- Randomness enters through `crypto.getRandomValues` for the skip-list structure only; geometry output does not depend on it, and plan fingerprints hash recorded data, not RNG state.
- The M5 verification number above uses default options; heavier refinement settings scale the same way planning does (roughly one core's worth of the native multithreaded run).
- wasm performance in browsers other than V8 (Safari, Firefox) was not measured here; the Node number is evidence of feasibility, not a cross-browser benchmark. An implementation milestone should add a browser acceptance check like the simulator's.

## 9. Reproducing the investigation

```sh
# from a scratch crate depending on cam-core by path
rustup target add wasm32-unknown-unknown wasm32-wasip1
RUSTFLAGS="--cfg getrandom_wasm_js" cargo build --release --target wasm32-unknown-unknown   # browser target
cargo build --release --target wasm32-wasip1                                                # measurement target
node run-wasi.mjs target/wasm32-wasip1/release/<spike>.wasm                                 # flower job end-to-end
```

The spike sources and the saved serial-fallback patch live outside version control in `.worktrees/wasm-spike/`; the patch content is fully described in §5.
