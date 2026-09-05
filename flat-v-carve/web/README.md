# Flat V-carve web workspace

The first U1 implementation of the [web UI design](../../docs/flat-v-carve/web-ui/README.md). This is a local, static TypeScript frontend for the future Rust service. It starts with a deterministic fixture adapter. No Rust source, Cargo files, or shared milestone documents are needed to develop it.

## Run

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

The production bundle is in `dist/`, with relative asset URLs for future local serving. `pnpm preview` serves that build on loopback port 4173. Fonts, scripts, and fixture artwork are bundled or use system resources; ordinary use makes no external requests. No hosting infrastructure is configured.

## What works

- Persistent 2D viewport, six-step navigator, inspector, and issues/activity drawer.
- Source/component inspection, explicit machining inclusion, independent visibility, and preserved holes. The bundled Inkscape coupon uses real captured Rust normalization; a new example job defaults the editable endmill wall allowance to 0 mm, matching Rust import. Other machining settings remain unset. Opening saved jobs preserves their allowance values.
- Display-only placement, fit job/inspected region, zoom, pointer/keyboard pan, and physical axes. Placement uses `scale × rotate(page XY − origin)`; no machining calculations run in TypeScript.
- Stock, placement, target allowances, both tool geometries/cutting values, and accuracy fields. Travel, entry/strategy, all current resource/sampling limits, and machine-profile fields are editable. Empty fields are distinct from zero; capability choices distinguish unset/Yes/No. Partial numeric and grouped inputs remain editable.
- Schema 3 job file open/download, undo/redo, and recovery across reloads in the **same browser tab**. Recovery is distinct from a downloaded portable snapshot. Open/replacement is undoable; rejected input preserves the current draft. Invalid recovery is preserved until explicitly replaced.
- System/light/dark appearance, responsive stacked panels, desktop inspector resizing, labeled controls, visible focus, native text-editing shortcuts, and keyboard equivalents for viewport actions.

This is a frontend development slice, not an import-to-export CAM release. The fixture banner and unavailable actions are intentional.

## Boundaries and next work

Rust remains authoritative for migration, normalization, numeric ranges, machining rules, planning, geometry/stock, identity, verification, and output. The Zod schema checks the portable **structure**; it does not certify machining settings. A downloaded draft must still pass Rust validation. Unknown fields and schemas are rejected instead of silently pruned or migrated.

The fixture adapter only supplies geometry for the captured source and import precision. A different source or tolerance removes the preview until the engine can inspect it. User source SVG is retained as data and never injected as HTML/SVG markup. Region inclusion is shown as editing intent, not a recomputed union/target. Placement previews transform supplied display coordinates only.

Optional planning/profile blocks can remain entirely unset. A partly entered block cannot be serialized until its required fields are supplied or the block is cleared; its unfinished text remains in recovery. Switching from ramp to plunge retains ramp text in the draft but excludes it from the active job. Clearing a block is undoable. Planning fields cannot be inferred from defaults. Different planning/profile clearances are reported without silently choosing between them.

The next U2 work is local service import/validation/file integration. U3 adds asynchronous plan tasks, identities, cancellation/reconnect, and recorded paths/stock. Verification and LinuxCNC output need authoritative M5/M6 service capabilities and exact-output checks. 3D, cross-sections, motion playback, durable recovery, external-file conflicts, and multi-tab document ownership remain later work. Per-tab session recovery deliberately has no cross-tab shared document writes.

## Code map

| File / directory | Responsibility |
| --- | --- |
| `src/contracts/job.ts` | Runtime structural checks and inferred TypeScript types for current portable jobs. |
| `src/contracts/service.ts` | Proposed replaceable service interface, capabilities, and display/diagnostic DTOs. No HTTP routes are frozen. |
| `src/service/fixture.ts` | Deterministic captured-artwork adapter; absent capabilities remain unavailable. |
| `src/state/` | Recoverable form text, candidate serialization, grouped edit history, monotonically increasing revisions. |
| `src/components/Viewport.tsx` | Inert ring renderer and display/camera transforms. |
| `src/components/SetupEditors.tsx` | Stock, planning, computation, and machine-profile editor composition. |
| `src/App.tsx` | Workspace composition, local file fallback, recovery, and capability presentation. |
| `tests/` | Draft/schema preservation, recovery, fixture/display, and output-gating regression checks. |

Inject a `CamService` into `<App service={adapter} />` to replace the fixture boundary. A real adapter must validate wire data and implement authoritative identity checks; merely advertising a capability cannot enable output in U1. Job schema, proposed UI API version, captured engine version, draft revision, and future artifact identities are separate concepts.

`pnpm check:contracts` checks 13 Rust struct field sets and job schema version read-only, then checks every projected display coordinate/ID/hole flag against the captured inspection. It is a drift alarm, not generated bindings or a full type-equivalence proof. Tests additionally round-trip all current M4 JSON jobs.

An optional CLI parity check uses an **existing** executable without compiling Rust:

```powershell
node scripts/check-cli-roundtrip.mjs ../target/release/cam.exe
```

The implementation history is in [U1 implementation](../../docs/flat-v-carve/web-ui/u1-implementation.md). The follow-up [setup and browser report](../../docs/flat-v-carve/web-ui/browser-checks.md) records 37 automated regressions and browser checks of all steps, keyboard/focus, recovery, file round-trips, themes, and 200% text. [React](https://react.dev/) provides the component/state layer; [Vite](https://vite.dev/guide/) produces a local static distribution without a server framework.
