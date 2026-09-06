# Flat V-carve web workspace

The local TypeScript workspace from the [web UI design](../../docs/flat-v-carve/web-ui/README.md), connected to Rust for SVG import, job migration/opening, validation, cancellable background planning, recorded motion previews, 2D stock inspection, M5 continuous verification, M6 checked output, and the local tool library. Development can still use the deterministic fixture adapter.

## Run the portable application

From `flat-v-carve/`, run `./scripts/build-portable.ps1` and then
`./artifacts/portable/cam.exe serve --open`. The executable contains this UI,
the HTTP service, and all CLI operations. It can run outside the checkout with
no Node.js or separate `dist` directory. Add `-Offline` to the build script when
dependencies are already cached. Rebuild the executable after UI changes.

## Develop with Rust

From `flat-v-carve/`, with Node.js 24+, pnpm 11.19.0, and the workspace's Rust 1.95.0 toolchain:

```powershell
pnpm --dir web install --frozen-lockfile
pnpm --dir web build
cargo build --release --locked -p cam-app
.\target\release\cam.exe serve --ui-dir web/dist --port 4848
```

Open the exact loopback URL printed by the service, or add `--open`. Linux uses `./target/release/cam serve`. `--port 0` selects an available port; a specified occupied port fails explicitly. `--ui-dir` selects a development UI directory, overriding any embedded assets. A build without `bundled-ui` falls back to `web/dist`. Rebuild and restart after changes. Ctrl+C stops it. The `cam-web` development alias remains available with `cargo build --release --locked -p cam-server` and accepts the same flags without the `serve` subcommand.

The production bundle selects the live adapter. Import starts a new job with cutting settings unset and 0 mm wall allowance; open accepts editable jobs and migrates schemas 1/2 through Rust. Supplied invalid settings are reported without rewriting the draft. Downloads require a successful editable-job validation receipt for the current revision. This does not establish planning readiness or machining verification.

The `cam-server` library is shared by `cam serve` and the `cam-web` development alias. It binds only `127.0.0.1` and serves UI and API from one origin. It relaunches the current executable in a hidden compute-worker mode for each task, calls core planners directly, and kills/reaps that worker on cancellation. Worker mode never loads UI files. It uses no shell and accepts no filesystem paths from HTTP requests. Complete plans are streamed into service-owned temporary files; verification/export use internal file references, while the browser receives bounded previews. File selection and portable downloads remain browser-owned.

Wire version **ui-7** removes the per-plan JSON byte ceiling from the live service.
The 32 MB worker reply and 20,000-motion preview budgets are independent of plan
file size. Artifact downloads stream in 64 KiB chunks. The latest four results
are retained; active verification, export, and downloads keep their source files
alive until finished. Rebuild the UI and service together and restart after an
update. See [plan artifact storage](../../docs/flat-v-carve/web-ui/u7-plan-artifacts.md)
for lifecycle, remaining bounds, and real-artwork regression commands.

## Tool library location and workflow

Live mode exposes **Carve & tools → Manage tool library**, plus a chooser beside
each job tool. Create an empty library explicitly, then add or capture tools,
manage cutting presets, or merge portable JSON. Search matches tool names/IDs
and preset names/context. Applying a selection first shows changed values and
then updates one job slot as a single undoable edit. Geometry-only selection
clears all five cutting values; existing job IDs and machine mapping are retained.

`cam serve --library-dir <directory>` selects an existing CLI library or another
local directory. The default is `%LOCALAPPDATA%/FlatVCarve/tool-library` on Windows,
`~/Library/Application Support/FlatVCarve/tool-library` on macOS, and
`$XDG_DATA_HOME/flat-v-carve/tool-library` (or `~/.local/share/flat-v-carve/tool-library`)
on Linux. Start/reload never creates or resets library data. Keep this directory
on a local filesystem. Rust performs revision checks and atomic store writes.

A conflict keeps the unfinished form. Reload updates the list, but saving that
old form stays disabled until it is discarded and the latest record is opened.
Closing the dialog keeps its form in memory; reloading the tab loses **unsaved
library forms**. Saved records persist independently of job recovery. The
[library report](../../docs/flat-v-carve/web-ui/tool-library-ui.md) records checks
and transport limits. Fixture mode has no tool library.

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

The production bundle is in `dist/`, with a generated `.bundle-manifest.json` recording source/asset hashes and engine version for native embedding. The manifest is not served to browsers. `pnpm preview` serves the UI on loopback port 4173; use `http://127.0.0.1:4173/?mode=fixture` for an explicit static demonstration. Without that query the production UI expects the local Rust service. Fonts, scripts, and fixture artwork are bundled or use system resources; ordinary use makes no external requests. No hosting infrastructure is configured.

## What works

- Persistent 2D viewport, six-step navigator, inspector, and issues/activity drawer.
- Source/component inspection, explicit machining inclusion, independent visibility, and preserved holes. The bundled Inkscape coupon uses real captured Rust normalization; a new example job defaults the editable endmill wall allowance to 0 mm, matching Rust import. Other machining settings remain unset. Opening saved jobs preserves their allowance values.
- Display-only placement, fit job/inspected region, zoom, pointer/keyboard pan, and physical axes. Placement uses `scale × rotate(page XY − origin)`; no machining calculations run in TypeScript.
- Stock, placement, target allowances, both tool geometries/cutting values, and accuracy fields. Travel, entry/strategy, all current resource/sampling limits, and machine-profile fields are editable. Empty fields are distinct from zero; capability choices distinguish unset/Yes/No. Partial numeric and grouped inputs remain editable.
- Schema 3 job file open/download, undo/redo, and recovery across reloads in the **same browser tab**. Recovery is distinct from a downloaded portable snapshot. Open/replacement is undoable; rejected input preserves the current draft. Invalid recovery is preserved until explicitly replaced.
- System/light/dark appearance, responsive stacked panels, desktop inspector resizing, labeled controls, visible focus, native text-editing shortcuts, and keyboard equivalents for viewport actions.

Live mode supports endmill and combined background planning, task cancellation/recovery, engine outcomes and diagnostics, and bounded recorded-motion previews. Stock inspection shows engine-produced depth slices after the endmill or both tools, lower/upper removal bounds, remaining target, possible overcut, and endmill floor coverage. Area and supported diagnostic links fit the affected region. Tool and path-layer filters change only the motion overlay. M5 verifies current combined plans with continuous error and depth-band bounds, optional rounded-coordinate checks, locatable findings, cancellation, and stale-report protection. M6 adds a separate LinuxCNC profile editor, cancellable generation, original/emitted verification review, combined/per-tool program previews, and hash-checked downloads gated by the current plan/profile/settings. 3D simulation remains unavailable. Fixture mode has no planning, verification or output capability.

## Boundaries and next work

Rust remains authoritative for migration, normalization, numeric ranges, machining rules, planning, geometry/stock, identity, verification, and output. The Zod schema checks portable **structure**; it does not certify machining settings. Live mode runs Rust validation after representable edits and discards stale responses. Unknown fields and unsupported schemas are rejected. Fixture mode cannot migrate or validate through Rust.

The fixture adapter only supplies geometry for the captured source and import precision. A different source or tolerance removes the preview until the engine can inspect it. User source SVG is retained as data and never injected as HTML/SVG markup. Region inclusion is shown as editing intent, not a recomputed union/target. Placement previews transform supplied display coordinates only.

Endmill travel/entry and machine-profile blocks can remain unset. A partly entered block cannot be serialized until its required machining fields are supplied or the block is cleared; unfinished text remains in recovery. Switching from ramp to plunge retains ramp text in the draft but excludes it from the active job. Clearing a block is undoable. Different planning/profile clearances are reported without silently choosing between them.

Planning budgets, V-bit sampling/cleanup settings, and numerical tolerances have automatic defaults. Empty editor fields show the effective default and an explanation; explicit values (including zero and low budgets in an existing job) are preserved. **Use default planning budgets** clears only work-ceiling overrides in one undoable edit. Advanced endmill/V-bit/accuracy sections allow individual overrides or resets. V-bit computation therefore needs no manual setup; endmill budgets are supplied once travel and entry are configured. Partial numeric text still blocks serialization rather than falling back to defaults.

Default work ceilings use the current Rust-supported maxima: endmill 256 layers / 1,024 loops per layer / 100,000 motions; V-bit 65,536 paths / 1,000,000 motions / 1,000,000 curve segments / 256 passes / 1,000,000 quality samples / 100,000 reachability cells. These bound work rather than preallocate it. Sampling and cleanup default to 1 mm spacing, 2 cleanup iterations and 8 stock slices; they can affect generated cleanup cuts and are distinct from pure work ceilings. Motion tolerance defaults to **0.01 mm**, verification tolerance to **0.05 mm**. They remain fixed when geometry or tools change and are never relaxed to make a job complete. Rust may require finer import geometry for these tolerances, especially with acute V-bits.

Resolved defaults are explicit numbers in submitted/downloaded jobs, so planning fingerprints and reopened jobs keep their actual settings. Same-tab recovery retains blank/default versus explicit overrides. Cutter geometry, feeds, spindle speed, travel, entry, depth and requested surface finish are still explicit user inputs. The Rust API/CLI portable-job semantics are unchanged.

Missing settings reported by Rust are listed in Issues and before the Generate button, with links to their editors. Endmill-only guidance omits settings used only by the V-bit stage. These presence checks do not replace the planner's feasibility checks. Planning budget and precision diagnostics link to the advanced editors; after edits, failures from an older setup remain in task history rather than the current Issues list.

To load a library tool, use **Choose endmill from library** or **Choose V-bit from library**, select the record, explicitly choose a cutting preset (or geometry only), review the changed values, then apply. This works while the job is unfinished: only the selected tool's fields are replaced, and other incomplete settings stay in the draft for validation before planning. Selecting a record alone does not edit the job. The selected tool and preset names appear next to the job tool after application; edits to its values change that label to **Edited since library selection**. This display association survives same-tab recovery but is not embedded in portable job files. Geometry-only application clears the five cutting values, as shown during review. Saving a job tool into the library still requires a valid editable job.

The [U3 background-planning report](../../docs/flat-v-carve/web-ui/u3-background-planning.md) records task execution and identity/recovery. The [stock-inspection report](../../docs/flat-v-carve/web-ui/u3-stock-inspection.md) records display limits and slice parity. The [M5 integration report](../../docs/flat-v-carve/web-ui/u5-verification.md) records the shared task queue, report identities, verification settings, and CLI/browser checks. Editable-job validation alone does not establish planning readiness or verification. The [M6 integration report](../../docs/flat-v-carve/web-ui/u5-linuxcnc-output.md) records the separate profile editor, checked output workflow and byte-for-byte CLI parity. Native save/file conflicts, 3D, arbitrary cross-sections, motion playback, durable recovery, and multi-tab document ownership remain later work.

## Code map

| File / directory | Responsibility |
| --- | --- |
| `src/contracts/job.ts` | Runtime structural checks and inferred TypeScript types for current portable jobs. |
| `src/contracts/service.ts`, `wire.ts`, `planning.ts`, `stock.ts`, `verification.ts`, `machineProfile.ts`, `export.ts`, `library.ts` | Replaceable service interface and runtime checks for the `ui-6` transport. |
| `src/service/fixture.ts` | Deterministic captured-artwork adapter; absent capabilities remain unavailable. |
| `src/service/http.ts`, `useValidation.ts` | Same-origin session, checked responses, debounced validation, and stale-response guards. |
| `src/service/usePlanning.ts`, `src/components/PlanPanel.tsx` | Immutable submissions, monotonic task tracking, cancellation, recovery, scoped outcomes, and stale-result gating. |
| `src/service/useInspection.ts`, `src/components/StockInspector.tsx` | Identity-bound slice loading, depth/tool/layer controls, metrics, overlays, and geometry-linked findings. |
| `src/service/useVerification.ts`, `src/components/VerificationPanel.tsx` | Recoverable verification settings/tasks, continuous bounds, coordinate scopes, and findings. |
| `src/service/useExport.ts`, `src/components/ExportPanel.tsx` | Recoverable machine profiles, checked program/report review and stale-output/download gates. |
| `src/components/ToolLibraryDialog.tsx`, `src/state/library.ts` | Library search/edit/import/capture, frozen edit revisions, candidate review, and one-slot job application. |
| `../crates/cam-server/` | Loopback HTTP boundary, bounded engine workers, build assets, and engine-to-display projection. |
| `src/state/` | Recoverable form text, candidate serialization, grouped edit history, monotonically increasing revisions. |
| `src/components/Viewport.tsx` | Inert source and stock rings, recorded motion projection, overlay controls, and camera transforms. |
| `src/components/SetupEditors.tsx` | Stock, planning, computation, and machine-profile editor composition. |
| `src/App.tsx` | Workspace composition, local file fallback, recovery, and capability presentation. |
| `tests/` | Draft/schema preservation, recovery, fixture/display, and output-gating regression checks. |

Inject a `CamService` into `<App service={adapter} />` to replace the boundary. The live adapter validates wire data and request/revision/engine identities. Merely advertising a capability cannot enable machine output. Job schema, UI API version, engine version, draft revision, document receipts, and future plan/output identities are separate concepts.

`pnpm check:contracts` checks 16 Rust struct field sets and job schema version read-only, then checks every projected display coordinate/ID/hole flag against the captured inspection. It is a drift alarm, not generated bindings or a full type-equivalence proof. Tests additionally check the three library record field sets and round-trip all current M4 JSON jobs.

An optional CLI parity check uses an **existing** executable without compiling Rust:

```powershell
node scripts/check-cli-roundtrip.mjs ../target/release/cam.exe
```

After building both Rust executables and the UI, `pnpm check:live` starts its own server on an ephemeral loopback port with an isolated library directory and compares the actual HTTP adapter with the same-engine CLI. It also checks the shipped assets and wire schemas. `pnpm test` runs the frontend-only regressions. Run `cargo test --release --locked -p cam-server` for HTTP boundary and engine projection tests. Test output is under ignored `test-results/`.

To exercise the single-file distribution instead, set `CAM_TEST_EXE` to the
absolute path of a portable `cam.exe`, then run `pnpm check:live`. The tests use
that EXE for both CLI and `serve`, start the server from its own directory with
only OS directories on `PATH`, and supply no `--ui-dir`. They compare every
embedded asset to the current build manifest and run the full live workflow.

The implementation history is in [U1 implementation](../../docs/flat-v-carve/web-ui/u1-implementation.md). The follow-up [setup and browser report](../../docs/flat-v-carve/web-ui/browser-checks.md) records 37 automated regressions and browser checks of all steps, keyboard/focus, recovery, file round-trips, themes, and 200% text. [React](https://react.dev/) provides the component/state layer; [Vite](https://vite.dev/guide/) produces a local static distribution without a server framework.
