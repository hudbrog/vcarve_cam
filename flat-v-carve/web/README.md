# Flat V-carve web workspace

The local TypeScript workspace from the [web UI design](../../docs/flat-v-carve/web-ui/README.md), connected to Rust for SVG import, job migration/opening, validation, cancellable background planning, recorded motion previews, 2D stock inspection, and M5 continuous verification. Development can still use the deterministic fixture adapter.

## Run with Rust

From `flat-v-carve/`, with Node.js 24+, pnpm 11.19.0, and the workspace's Rust 1.95.0 toolchain:

```powershell
pnpm --dir web install --frozen-lockfile
pnpm --dir web build
cargo build --release --locked -p cam-server -p cam-app
.\target\release\cam-web.exe --port 4848
```

Open the exact loopback URL printed by `cam-web`. Linux uses `./target/release/cam-web`. `--port 0` selects an available port; a specified occupied port fails explicitly. `--ui-dir` selects the built UI directory (default `web/dist`). The service loads build assets at startup, so rebuild and restart after changes. Ctrl+C stops it.

The production bundle selects the live adapter. Import starts a new job with cutting settings unset and 0 mm wall allowance; open accepts editable jobs and migrates schemas 1/2 through Rust. Supplied invalid settings are reported without rewriting the draft. Downloads require a successful editable-job validation receipt for the current revision. This does not establish planning readiness or machining verification.

The service runs as a separate `cam-web` executable to keep CLI/machining development independent. It binds only `127.0.0.1` and serves UI and API from one origin. It launches its own hidden compute worker for each plan, calls core planners directly, and kills/reaps that worker on cancellation. It never launches the CLI or a shell, writes jobs to local paths, or accepts filesystem paths from HTTP requests. File selection and portable downloads remain browser-owned.

## Develop against fixtures

Use Node.js 24 or newer and pnpm 11.19.0. From the repository root:

```powershell
Set-Location flat-v-carve/web
pnpm install --frozen-lockfile
pnpm dev
```

Vite binds only to `127.0.0.1:5173` and refuses to take a different port silently. If another task uses that port, run `pnpm dev --port 5175`. Development does not start or build Rust.

```powershell
pnpm test
pnpm check:contracts
pnpm build
```

The production bundle is in `dist/`. `pnpm preview` serves it on loopback port 4173; use `http://127.0.0.1:4173/?mode=fixture` for an explicit static demonstration. Without that query the production UI expects `cam-web`. Fonts, scripts, and fixture artwork are bundled or use system resources; ordinary use makes no external requests. No hosting infrastructure is configured.

## What works

- Persistent 2D viewport, six-step navigator, inspector, and issues/activity drawer.
- Source/component inspection, explicit machining inclusion, independent visibility, and preserved holes. The bundled Inkscape coupon uses real captured Rust normalization; a new example job defaults the editable endmill wall allowance to 0 mm, matching Rust import. Other machining settings remain unset. Opening saved jobs preserves their allowance values.
- Display-only placement, fit job/inspected region, zoom, pointer/keyboard pan, and physical axes. Placement uses `scale × rotate(page XY − origin)`; no machining calculations run in TypeScript.
- Stock, placement, target allowances, both tool geometries/cutting values, and accuracy fields. Travel, entry/strategy, all current resource/sampling limits, and machine-profile fields are editable. Empty fields are distinct from zero; capability choices distinguish unset/Yes/No. Partial numeric and grouped inputs remain editable.
- Schema 3 job file open/download, undo/redo, and recovery across reloads in the **same browser tab**. Recovery is distinct from a downloaded portable snapshot. Open/replacement is undoable; rejected input preserves the current draft. Invalid recovery is preserved until explicitly replaced.
- System/light/dark appearance, responsive stacked panels, desktop inspector resizing, labeled controls, visible focus, native text-editing shortcuts, and keyboard equivalents for viewport actions.

Live mode supports endmill and combined background planning, task cancellation/recovery, engine outcomes and diagnostics, and bounded recorded-motion previews. Stock inspection shows engine-produced depth slices after the endmill or both tools, lower/upper removal bounds, remaining target, possible overcut, and endmill floor coverage. Area and supported diagnostic links fit the affected region. Tool and path-layer filters change only the motion overlay. M5 verifies current combined plans with continuous error and depth-band bounds, optional rounded-coordinate checks, locatable findings, cancellation, and stale-report protection. 3D simulation and machine output remain unavailable. Fixture mode has no planning or verification capability.

## Boundaries and next work

Rust remains authoritative for migration, normalization, numeric ranges, machining rules, planning, geometry/stock, identity, verification, and output. The Zod schema checks portable **structure**; it does not certify machining settings. Live mode runs Rust validation after representable edits and discards stale responses. Unknown fields and unsupported schemas are rejected. Fixture mode cannot migrate or validate through Rust.

The fixture adapter only supplies geometry for the captured source and import precision. A different source or tolerance removes the preview until the engine can inspect it. User source SVG is retained as data and never injected as HTML/SVG markup. Region inclusion is shown as editing intent, not a recomputed union/target. Placement previews transform supplied display coordinates only.

Optional planning/profile blocks can remain entirely unset. A partly entered block cannot be serialized until its required fields are supplied or the block is cleared; its unfinished text remains in recovery. Switching from ramp to plunge retains ramp text in the draft but excludes it from the active job. Clearing a block is undoable. Planning fields cannot be inferred from defaults. Different planning/profile clearances are reported without silently choosing between them.

The [U3 background-planning report](../../docs/flat-v-carve/web-ui/u3-background-planning.md) records task execution and identity/recovery. The [stock-inspection report](../../docs/flat-v-carve/web-ui/u3-stock-inspection.md) records display limits and slice parity. The [M5 integration report](../../docs/flat-v-carve/web-ui/u5-verification.md) records the shared task queue, report identities, verification settings, and CLI/browser checks. Editable-job validation alone does not establish planning readiness or verification. M6 LinuxCNC output is implemented in Rust; its separate profile editor, exact-output checks, and gated downloads are next. Native save/file conflicts, 3D, arbitrary cross-sections, motion playback, durable recovery, and multi-tab document ownership remain later work.

## Code map

| File / directory | Responsibility |
| --- | --- |
| `src/contracts/job.ts` | Runtime structural checks and inferred TypeScript types for current portable jobs. |
| `src/contracts/service.ts`, `wire.ts`, `planning.ts`, `stock.ts`, `verification.ts` | Replaceable service interface and runtime checks for the `ui-4` transport. |
| `src/service/fixture.ts` | Deterministic captured-artwork adapter; absent capabilities remain unavailable. |
| `src/service/http.ts`, `useValidation.ts` | Same-origin session, checked responses, debounced validation, and stale-response guards. |
| `src/service/usePlanning.ts`, `src/components/PlanPanel.tsx` | Immutable submissions, monotonic task tracking, cancellation, recovery, scoped outcomes, and stale-result gating. |
| `src/service/useInspection.ts`, `src/components/StockInspector.tsx` | Identity-bound slice loading, depth/tool/layer controls, metrics, overlays, and geometry-linked findings. |
| `src/service/useVerification.ts`, `src/components/VerificationPanel.tsx` | Recoverable verification settings/tasks, continuous bounds, coordinate scopes, and findings. |
| `../crates/cam-server/` | Loopback HTTP boundary, bounded engine workers, build assets, and engine-to-display projection. |
| `src/state/` | Recoverable form text, candidate serialization, grouped edit history, monotonically increasing revisions. |
| `src/components/Viewport.tsx` | Inert source and stock rings, recorded motion projection, overlay controls, and camera transforms. |
| `src/components/SetupEditors.tsx` | Stock, planning, computation, and machine-profile editor composition. |
| `src/App.tsx` | Workspace composition, local file fallback, recovery, and capability presentation. |
| `tests/` | Draft/schema preservation, recovery, fixture/display, and output-gating regression checks. |

Inject a `CamService` into `<App service={adapter} />` to replace the boundary. The live adapter validates wire data and request/revision/engine identities. Merely advertising a capability cannot enable machine output. Job schema, UI API version, engine version, draft revision, document receipts, and future plan/output identities are separate concepts.

`pnpm check:contracts` checks 13 Rust struct field sets and job schema version read-only, then checks every projected display coordinate/ID/hole flag against the captured inspection. It is a drift alarm, not generated bindings or a full type-equivalence proof. Tests additionally round-trip all current M4 JSON jobs.

An optional CLI parity check uses an **existing** executable without compiling Rust:

```powershell
node scripts/check-cli-roundtrip.mjs ../target/release/cam.exe
```

After building both Rust executables and the UI, `pnpm check:live` starts its own server on an ephemeral loopback port and compares the actual HTTP adapter with the same-engine CLI. It also checks the shipped assets and wire schemas. `pnpm test` runs the frontend-only regressions. Run `cargo test --release --locked -p cam-server` for HTTP boundary and engine projection tests. Test output is under ignored `test-results/`.

The implementation history is in [U1 implementation](../../docs/flat-v-carve/web-ui/u1-implementation.md). The follow-up [setup and browser report](../../docs/flat-v-carve/web-ui/browser-checks.md) records 37 automated regressions and browser checks of all steps, keyboard/focus, recovery, file round-trips, themes, and 200% text. [React](https://react.dev/) provides the component/state layer; [Vite](https://vite.dev/guide/) produces a local static distribution without a server framework.
