# U5: checked LinuxCNC output integration

Date: 2026-09-05. Rust 0.7.2; local wire `ui-5`.

This follows the M5 verification integration committed as `8c69ff6`. It connects the existing M6 core postprocessor to the local service and Export step. Core machining and CLI source are unchanged.

## Operator workflow

- Generate a current combined endmill/V-bit plan.
- Open or edit the separate schema-1 LinuxCNC machine profile: work offset, stock-top/bottom datum, precision, clearance, startup position, compensation owner, T/H mappings, spindle direction/dwell, coolant, and M6 return/offset declarations.
- Empty profile fields remain unset. The optional copy action copies only job tool IDs and planning clearance. Machine contracts are never inferred from the legacy job description. Required declarations and their reference must be reviewed explicitly or supplied in an opened profile.
- Choose combined or per-tool output. Generate starts an immutable background task and rechecks both the original plan and exact emitted program. It uses Verification-step computation/report budgets; M6 precision comes from the machine profile.
- Review original and emitted outcomes, diagnostics, bounds/findings, coordinate datum, program preview, prerequisites and hashes. Download the original report and each checked program separately. Per-tool V-bit output displays the required matching endmill-file prerequisite; independently initialized programs still share stock history.

Profile downloads are editable settings without a verification claim. Profile text, including incomplete numbers, and layout recover in the same tab. A reload recovers task handles but requires regenerated planning before machine output becomes current again. Job edits, new plans, profile changes, layout changes, verification-budget changes and service replacement invalidate program buttons. Previously loaded report/program previews remain explicitly stale. Report downloads remain available for diagnosis.

The drawing continues to show recorded plan paths. The code preview shows the first 80 emitted lines; downloads contain every checked byte. Export evidence is in stock-top workpiece coordinates after inverse machine-datum translation. Profile startup/retract coordinates are in the selected machine work frame.

## Service and identity

`POST /api/v1/exports` admits a source plan task, document revision/receipt, source input and motion fingerprints, explicit profile, layout, and verification options. It accepts no client-supplied plan, report, program, shell command, or filesystem path. Only a retained combined planning task can be a source. Exact retries remain idempotent after source eviction; reusing an ID with changed inputs is rejected.

The worker copies the source artifact at admission, authenticates it with `CombinedPlan::from_json`, and calls `cam_core::post::export_plan`. Core profile/job compatibility checks include clearance and representability, startup/return contracts, tool mapping and compensation, stock thickness, and optional job machine constraints. Invalid profiles produce a failed task diagnostic. A completed calculation can have a passed, failed or inconclusive report; task completion alone does not authorize output.

The existing queue is shared by planning, verification and export: one hidden process, four unfinished slots, 128 task records, five-minute timeout, latest four retained results. Cancellation kills/reaps the worker before reporting cancellation. Inputs are capped at 96 KB for export metadata and 64 KB for the profile. Reports are limited to 16 MB, the complete program set to 8 MB, and private worker replies to 32 MB. Oversized results fail without publishing partial programs.

`GET /api/v1/tasks/{id}/export` returns the immutable task, original report JSON, parsed report, and exact UTF-8 program strings. Programs are returned only for a passed core outcome. Generic task/status/cancel routes apply; plan and verification result routes reject an export task. The artifact route returns `export-report.json` for export tasks.

The browser structurally checks the full envelope, profile, report and file set, matches the immutable request, validates the core authenticated-plan fingerprint, then checks report SHA-256 and every program SHA-256 against the exact UTF-8 bytes before storing a downloadable result. Downloads run synchronously in the click handler. Already loaded, checked bytes may remain usable after server cache eviction while the current plan/profile identity still matches; unsaved browser edits are gated in the client, as the server cannot observe them.

Capabilities advertise `exportFormats: ["linuxcnc"]`, layouts and input/output limits. UI and service must be rebuilt/restarted together for `ui-5`; portable job/plan schemas and core version are unchanged.

## Validation

- 69 frontend tests: real captured passed/rejected reports, exact-byte/hash rejection, source/profile/layout/budget freshness, contradictory outcomes, HTTP identity checks, profile variants and incomplete-text recovery, stale download controls.
- 21 live service/CLI tests: all previous import/planning/stock/M5 parity plus combined and per-tool M6 output, stock-bottom macro and stock-top tool-table contracts, coarse-formatting failure, budget exhaustion, invalid contracts/clearance, active cancellation and responsive document checks. Every report field and each successful program byte matches the same-engine CLI.
- 13 Rust service tests cover boundaries, shared queue behavior, source/profile identity, metadata size, idempotent retry after source expiry and cancellation. Production/type build, Clippy with warnings denied, formatting and 16 Rust structural-contract checks pass.
- Browser checks on a separate synthetic test tab cover profile open, successful original/emitted checks, immediate stale-output gating after precision changes, `POST_ROUNDING` rejection with no program download, and recovery of an unfinished `1e-` field with generation disabled. Per-tool review uses the synthetic wide-floor fixture (76 endmill and 316 V-bit checked motions), displays the matching endmill prerequisite, and disables both file downloads immediately after a job edit. Desktop preview checks report no browser errors or warnings.

The embedded browser's previously observed Blob-download cancellation remains a browser limitation. Download controls and byte preparation are exercised; on-disk browser saving is not claimed. CLI/service parity confirms the exact bytes independently. Actual LinuxCNC macro/configuration review and controller preview/simulation remain the machine integration checks described in the core M6 report.

## Remaining frontend work

U4 still includes 3D/sections, playback and rendering scale. Native saves and file conflicts, durable recovery, multi-tab ownership, broader accessibility/browser qualification and packaging remain release work. The M5/M6 service integration is no longer a frontend blocker.
