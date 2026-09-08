# U9: WebAssembly engine and static hosting

Date: 2026-09-08\
Status: Implemented and browser-accepted. The planning pass runs in the browser as WebAssembly behind the same `CamService` contracts; the UI build can be statically hosted with no local service. The native portable application is unchanged.

This implements the direction measured in the [wasm packing investigation](../wasm-packing.md). The engine core, its service DTOs, and the browser workers are new code paths; the TypeScript UI, contracts, and state logic are untouched except for service selection.

## 1. What shipped

| Piece | Location | Role |
| --- | --- | --- |
| Engine serial fallbacks | `cam-core` (polygon erode, streamed slices, adaptive workers) | One-worker hosts take serial paths instead of panicking on `thread::scope` spawns; native behavior and result order are unchanged. |
| `cam-service` crate | `crates/cam-service` | Pure DTOs shared by the HTTP service and the browser build: document operations/envelopes, stock inspection projection, plan summaries, admission checks, and the verification/export request identities. Both adapters emit byte-identical wire shapes. |
| `cam-wasm` crate | `crates/cam-wasm` | wasm-bindgen entry: document commands, plan/verify/export compute, admission, limits, and default options. Every function is JSON-in/JSON-out and runs under native `cargo test`. |
| Browser workers | `web/src/service/wasm/` | A parent worker owns the engine instance and the task ledger; a disposable compute child per task mirrors the native service's compute process. Cancellation terminates the child. |
| Service selection | `web/src/service/wasm.ts`, `auto.ts`, `main.tsx` | `createWasmService` implements `CamService` over the worker; `?mode=wasm|live|fixture` overrides automatic detection (probe `/api/v1/session`, else wasm). |
| Build integration | `web/scripts/build-wasm.mjs`, `pnpm build:wasm` | wasm-pack (0.15.0) builds `cam-wasm` for `wasm32-unknown-unknown` into `web/src/wasm/gen`; `pnpm build` runs it first, so Vite bundles the module (2.07 MB raw / ~640 KB gzipped unoptimized-release profile). |

Tool-library operations are not offered in wasm mode (`capabilities.toolLibrary` is absent), and the UI disables the library controls accordingly.

## 2. Architecture notes

- **Same wire contracts.** The wasm service reports `mode: 'live'` with the same capabilities, envelopes, task snapshots, paged motions, stock slices, and verification/export results the HTTP service emits. All existing zod schemas and acceptance helpers (`acceptTask`, `acceptVerification`, `acceptExport`, `checkExportBytes`) validate the browser engine unchanged.
- **Disposable compute.** Browsers cannot interrupt a running wasm computation, so each plan/verification/export runs in a freshly spawned child worker; cancel kills it, exactly like the service's `--planning-worker` process. One compute runs at a time, four pending tasks maximum, 128 tasks per session, four retained results — the native ledger bounds.
- **In-memory artifacts.** The retained plan JSON and receipt live in the parent worker's memory instead of temp files; report tasks capture their compute input at admission, which is the in-memory equivalent of the native source-file lease.
- **Single source of truth.** The getrandom `wasm_js` opt-in (for the transitive `boostvoronoi` → `cpp_map` → `rand` chain) is declared once in `cam-wasm` with `.cargo/config.toml` target flags; it is inert on native targets. `Instant`/timers stay unavailable on the browser target, as documented in the investigation.

## 3. Evidence

Rust: `cargo test --workspace` passes (35 suites, including the moved cam-service tests and 8 new cam-wasm tests covering document envelopes, admission checks, plan summaries/paging payload, and a plan → M5 verify → LinuxCNC export round trip on real fixtures). `cargo check -p cam-wasm --target wasm32-unknown-unknown --locked` passes. Clippy is clean with `-D warnings`.

Web: 12 new vitest cases cover the ledger (admission failures, queue bounds, idempotent replay and key reuse, motion paging, slice lookup, cancellation winning over late results, failure diagnostics, retention eviction, report binding to a combined source) and the service (capabilities parse against the shared schema, envelope identity checks, complete paged preview with progress, worker-failure propagation). The full frontend suite (149 tests) passes, and `pnpm build` produces a static bundle containing the hashed wasm module plus parent and child worker chunks.

Browser acceptance (Edge over a static `vite preview` of the production bundle, `?mode=wasm`):

1. The page auto-detected the absence of the local service and connected through the wasm engine; capabilities parsed; the tool-library controls were disabled.
2. The bundled example opened through the in-browser engine ("Live Rust normalization", 7 regions with preserved holes).
3. Completing setup exercised real engine validation: an inconsistent V-bit cone (cutting diameter reached before the stated usable height) was rejected with `INCONSISTENT_VBIT`; after the fix, the draft validated.
4. Combined planning ran in a disposable worker and produced **18,354 recorded motions (17,115 cutting)** with the complete paged preview loaded; the viewport rendered endmill and V-bit overlays, and the UI stayed responsive during computation.
5. M5 continuous verification ran in a second disposable worker to a located **failed** result — 999,999 evaluated cells, 708,441 unresolved, 64 findings with coordinates and measured intervals — proving the detail-residual limit the ad-hoc test settings could not meet. Export stayed blocked pending a passing verification, as designed.
6. A compute-dispatch bug found during this run (a verification request routed to the export entry) was fixed by making the child reject unknown computation kinds, and the acceptance sequence above was rerun on the rebuilt bundle.

Remaining limits: single-threaded execution (~4× the native multithreaded time on equivalent artwork; the investigation's measurements apply); browser memory holds the retained plan instead of a temp file; Safari/Firefox were not exercised; the local tool library and cross-origin-isolated threaded execution remain future work, as recorded in the investigation's risk section.

## 4. Development workflow

```sh
cd flat-v-carve/web
pnpm build:wasm   # wasm-pack the engine into src/wasm/gen (prerequisite for build/typecheck)
pnpm build        # static bundle in web/dist, including the engine module
pnpm preview      # serve the static bundle locally
```

Deploy `web/dist` to any static host. `?mode=wasm` forces the in-browser engine, `?mode=live` forces the local service, and the default probes `/api/v1/session` first. The portable `cam.exe` embeds the same bundle (the wasm asset and its MIME type are now included), so `?mode=wasm` also works without the local service; its content-security-policy allows wasm compilation via `wasm-unsafe-eval`.
