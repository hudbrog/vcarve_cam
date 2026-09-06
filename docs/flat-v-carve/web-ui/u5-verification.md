# U5: continuous verification integration

Date: 2026-09-05. Follows committed stock inspection `d680e48` on `codex/web-integration`, using Rust 0.7.2.

This slice exposes the existing M5 verifier for retained combined endmill/V-bit plans. No core machining implementation changes are needed. M6 profile setup and checked machine output remain the next integration slice.

## Workflow and evidence

Verification is an explicit action on a current combined plan. The service copies the retained immutable artifact and invokes `CombinedPlan::from_json` and `verify_plan` in its private compute process. Plan authentication/replay and all error bounds remain Rust-owned. An endmill-only source is rejected for M5; its original M3 evidence remains in Plan & inspect.

The panel separates task success from `passed`, `failed`, and `inconclusive` verification outcomes. It shows lower/upper error intervals, residual/overcut volume intervals, declared physical limits, unresolved cells, maximum uncertainty, and area intervals valid throughout each closed depth band. Core limitations and report identity remain inspectable. Very small nonzero bounds use scientific notation rather than rounding to zero.

Original and optional rounded-coordinate reports have separate evidence selectors. Overall status retains the combined verdict: an original-coordinate pass does not erase a rounded-coordinate failure. Rounding here checks numeric coordinates and is not validation of an emitted LinuxCNC program.

Findings retain the core code, status, message, measured interval/limit, point, optional cell, and optional motion ID. Location buttons and issue-drawer entries focus the supplied geometry in workpiece coordinates. A point without a cell receives a small camera context, with no invented error bounds. The viewport labels the selected evidence scope; path overlays still show original recorded motions. Findings and depth bands are revealed twenty at a time, with explicit counts and omitted-finding totals from Rust.

## Settings, identity, and recovery

Rust supplies default verification options through capabilities. The UI edits computation budgets and optional coordinate decimal places separately from the job's physical tolerances. Blank decimal precision means original coordinates only; zero means an actual zero-decimal rounding check. Required budgets preserve unfinished text and reject incomplete/out-of-range values. Verification option drafts and the latest task handle recover in the same browser tab without modifying the portable job.

Every verification task is bound to its service instance, engine, task ID, submitted revision/document receipt, source plan task, source input/motion fingerprints, and explicit options. The server rejects mismatches before work admission. It reuses the original task for an exact retry, including after the source artifact expires; reusing a key with changed input is rejected. Reports have their own core-generated verification and authenticated-plan fingerprints. M5 report input/motion fingerprints use their own core definitions and are not treated as the M4 artifact fingerprint fields.

Edits, pending/invalid job validation, another plan, changed options, a changed service, or reload recovery make nonmatching evidence stale immediately. Previous reports stay readable, but their location overlays disappear. The browser cannot turn a report current by changing a label or accepting a reordered response. Reload never replays computation; a recovered source plan remains conservatively stale until explicitly regenerated.

## Service and resource limits

Wire version **`ui-4`** adds `verificationScopes: ["continuous-stock"]` and `verification.defaultOptions`. Rebuild/restart UI and service together; job schema 3 and plan schema 1 are unchanged.

- `POST /api/v1/verifications` accepts task identity, a retained source-plan reference/fingerprints, and options. Its body is capped at 16 KiB; it accepts no supplied artifact, job replacement, or filesystem path.
- Existing `GET /api/v1/tasks/{id}` and `POST /api/v1/tasks/{id}/cancel` serve both computation kinds. Verification snapshots add explicit source/options identity and a verification-specific summary.
- `GET /api/v1/tasks/{id}/verification` returns the matching task and complete core report in workpiece coordinates. A plan task cannot be read as verification, and a verification task cannot be read as a plan preview.
- The authenticated artifact route serves the original JSON report as `verification.json`; there is no report-download UI in this slice.

Planning and verification share one active process, four unfinished slots, 128 immutable task records, a five-minute timeout, and the latest four retained results. Reports count toward that shared retention budget. Document validation keeps its separate worker budget. Cancelling stops and reaps the child before terminal cancellation; disconnecting does not cancel it. No service operation launches a shell or the CLI. The worker/report transport remains bounded, and oversized reports fail explicitly rather than dropping evidence.

## Checks

- **63 frontend tests** pass, including captured M5 report structure, contradictory interval/scope rejection, option/plan/task mismatch handling, stale-result gating, zero versus blank precision, and finding projection without duplicate placement.
- **15 live service/CLI tests** pass. New cases compare every report field for a passed original-coordinate report, a failed zero-decimal report, and an inconclusive one-cell report. They also cover explicit retries, mixed identities, wrong task-kind access, reconnection, responsive document validation, and active verification cancellation.
- **12 Rust service tests** pass. New checks cover source/stage/options admission, retry after source expiry, shared queue cancellation, and the small verification-request body limit.
- Production build/typecheck, the existing 13-struct/captured-geometry drift check, Rustfmt, Clippy with warnings denied, and diff whitespace checks pass.
- Browser checks in a separate tab use `m4/narrow-channel`: original verification passes with 11,447 evaluated cells and no unresolved cells; zero-decimal verification fails while original evidence remains passed; findings locate actual points; edits hide markers synchronously; and unfinished `1e` cell-budget text survives reload with verification disabled and the old report stale. The user's artwork setup tab was left untouched.
- A one-cell browser run stays inconclusive and displays its unresolved cell, measured 0–2 mm interval, and 0.15 mm limit. The final build shows the finding's evidence scope above the viewport, and its console has no errors or warnings.

Tests use synthetic fixtures, not machining presets. Machine output remains unavailable even after an M5 pass. Next: complete LinuxCNC profile editing, original/emitted-motion checks, output/report previews, and downloads bound to exactly the checked plan/profile/bytes.
