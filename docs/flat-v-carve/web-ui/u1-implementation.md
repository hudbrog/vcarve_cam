# U1 implementation baseline

Date: 2026-09-05\
Status: initial implementation complete; setup editors and targeted Chromium browser checks completed in the [follow-up report](browser-checks.md). Rust service integration and broader release qualification remain pending.

Implementation lives entirely in [`flat-v-carve/web/`](../../../flat-v-carve/web/README.md). The work used the existing checkout with a disjoint frontend directory, following the option to leave Rust untouched while M5 progresses. No Cargo files, Rust sources, shared root READMEs, or Rust milestone reports were changed by this UI work.

The stack is React, strict TypeScript, Vite, and Zod, with a pinned pnpm lockfile. The output is a local static bundle, consistent with the Rust-served browser architecture. There is no Node production service or remote hosting dependency.

## Implemented slice

The six guided steps share a persistent artwork viewport, an inspector, and an issues/activity drawer. The source list separates inspection, machining inclusion, and visibility. A real captured Inkscape inspection supplies IDs, source rings, hole flags, page dimensions, and precision. The viewport uses physical axes, a display-only placement transform, fit/zoom/pan, and a grid. No target, toolpath, stock, or verification result is synthesized.

Incomplete settings are editable with explicit units and nullable capability controls. Form text stays separate from portable job data, including malformed numbers and partial geometry. Job schema 3 files open/download structurally; unsupported schemas and unknown fields are rejected. Existing strategy/resource/profile fields are preserved even where editors have not yet been implemented. Download is available for structurally representable incomplete jobs; partially entered required geometry must be completed or cleared first.

Edits support undo/redo and monotonically increasing revisions. A source/precision change cannot display an unrelated captured result. Per-tab recovery retains unfinished input across reloads without claiming the portable file was saved. Invalid recovery is not silently overwritten, and file-read failures retain the current draft. Theme preference is the only shared browser-storage value.

All steps remain navigable. Fixture mode advertises no import, validation, planning, verification, or machine-output capabilities. Even a future adapter advertising export cannot enable output without a reviewed exact identity. The interface distinguishes source-only display from machining evidence.

## Validation

- Strict TypeScript compilation and a production bundle.
- 20 passing unit/contract tests for empty versus zero versus invalid numbers, partial tool geometry, nullable booleans, schema rejection, all M4 fixture round-trips, grouped undo/redo, monotonic revisions, recovery, immutable display data, cancellation signals, output gating, and inert SVG rendering.
- Read-only comparison of 13 Rust serialized struct field sets and job schema version against frontend schemas.
- Full display projection comparison with captured Rust coordinates, source IDs, and hole flags.
- The existing Rust 0.5.0 CLI accepted round-tripped incomplete Inkscape (22 unset fields) and configured finite-tip (zero unset fields) jobs, without rebuilding or changing Rust. The check is retained as an optional script.
- A loopback development preview and module request returned HTTP 200. The Codex preview handoff was queued. No browser interaction, screenshot, responsive visual, or accessibility qualification is claimed for this implementation.

## Remaining work

This starts U1 and selected U2 form/file behavior; it does not complete U2–U6 or change M5–M7 status. Remaining U1 acceptance includes real browser keyboard/resizing/theme/text-zoom checks. Schema field-set checks are a temporary drift alarm; generated bindings or mechanical type-equivalence checks should accompany service integration.

The follow-up completed travel, strategy, resource-limit, and machine-profile editors and the targeted U1 browser checks. The baseline descriptions and 20-test count above record the original slice; the current suite has 37 tests. Next: agree on stage-specific validation/diagnostic DTOs, connect local Rust import/inspection/validation, and extend UI/CLI parity. Then add asynchronous task identity/cancellation/reconnect and recorded-motion/stock display. Full verification and LinuxCNC export require authoritative M5/M6 contracts. Durable recovery, file conflicts, shared document ownership, 3D/sections, and packaging remain later milestones.
