# U2 local service integration slice

Date: 2026-09-05\
Status: implemented on `codex/web-integration`, based on `2bbe2f8` (M5 + committed UI). Planning/task integration and native file persistence remain next work.

The isolated worktree is `D:/proj1/.worktrees/web-integration`. The original checkout and its concurrent M6 work are untouched. This branch adds `cam-server`, updates the Rust workspace member list/lockfile, and connects the frontend. No `cam-core` or `cam-app` source changes are needed. Integrating the M6 branch later will require reconciling the shared Cargo manifest/lockfile as usual.

## Delivered workflow

`cam-web` serves a built UI and the local API on one loopback origin. The browser can import supported SVGs, open/migrate portable jobs, inspect fresh Rust-normalized source rings, edit incomplete settings, and receive authoritative editable-job validation. Import is a new job with unset machining values; opening/replacing is undoable and errors retain the previous draft. No CLI subprocess or TypeScript machining implementation is involved.

Display requests depend on the embedded SVG and all import options. In particular, scale affects Rust's source flattening/grid precision, so scale/placement edits must refresh normalized data. The response stays in source-page mm/Y-up coordinates; the viewport applies display placement. Source inspection ignores unrelated unfinished machining fields. Failed normalization removes that preview and preserves the engine diagnostic.

Validation is debounced by 400 ms and applies only to the exact draft revision. Edits and undo invalidate rendered evidence immediately. Aborted, mismatched, or superseded responses cannot install success. A structurally incomplete draft stays in per-tab recovery and does not invoke validation. Rust errors retain their code, severity, stage, and optional source association. Missing settings come from `Job::inspect`; they are not a new stage-readiness contract. No field paths are invented for diagnostics that lack them.

Portable downloads use the successful validation receipt for the current revision and run synchronously inside the user gesture. Missing machining settings are allowed, while invalid settings or partial text block the live download. This is an editable document check, not cutting verification or machine-output approval. There are no native filesystem writes, save-overwrite APIs, or external-file conflict claims.

## Versioned API

| Route | Contract |
| --- | --- |
| `GET /api/v1/session` | `apiVersion`, `engineVersion`, process-local session token. |
| `GET /api/v1/capabilities` | Import/open/validation availability, engine identity, effective limits; no exposed planning/verification/export capabilities. |
| `POST /api/v1/document` | `{ apiVersion, requestId, revision, command }`, with a tagged `command.operation` of `import`, `open`, `display`, or `validate`. |

Commands accept SVG bytes as UTF-8 JSON strings and explicit import options, job-file JSON text for migration/open, or a candidate job for validation. Responses echo API/engine/request/revision identity and carry checked data or a structured engine diagnostic. Transport failures have a typed error envelope. The client rejects unsupported versions, malformed data, and mismatched identities. There is no automatic operation replay after a restarted session; Reconnect obtains a fresh session and checks the retained draft again.

Accepted documents carry a SHA-256 `documentFingerprint` generated in Rust over a domain/version prefix, engine version, and serialized job. This identifies only the editable-document receipt. It is deliberately distinct from the existing engine plan, motion, verification, and future output fingerprints. Validation has scope `editable-job-and-svg`; no `planning_available` boolean is promoted into stage/export eligibility.

`crates/cam-server/src/document.rs` owns request DTOs and engine projection. `web/src/contracts/wire.ts` checks the wire at runtime. The live integration suite feeds real Rust responses through those schemas and compares them with CLI results. The existing 13-struct job field-set check remains an additional drift alarm; fully generated type bindings are not claimed.

## Local boundary and lifecycle

- Bind only `127.0.0.1`. Explicit ports fail if occupied; port 0 chooses a free port. Use the exact printed URL.
- Reject unexpected/duplicate Host values, foreign/null Origin values, and cross/same-site Fetch Metadata. Require the random in-memory session header for document and capability requests. Do not enable CORS or store tokens in URLs/browser storage.
- Serve only the startup-loaded `index.html` and recognized build assets. HTTP paths never become filesystem paths. Assets are bounded to 32 MB, with no directory listing or source/job file serving. Responses use no-store, nosniff, no-referrer, framing restrictions, and a same-origin content policy.
- Bound request bodies to 16.1 MB and concurrent requests to eight. Advertise the existing engine limits of 2 MB SVG and 8 MB job JSON. Up to two blocking inspection workers run outside async HTTP handling; saturation returns a retryable error.
- A 30-second request timeout or browser abort does not claim cancellation of an already-running engine computation. The worker retains its permit until completion. Long-running planner tasks and cooperative cancellation are deferred to U3.
- Restart rotates the session and reloads built assets. No document data is persisted by the server. Ctrl+C shuts it down.

[Axum](https://docs.rs/axum/0.8.9/axum/) provides the HTTP/extractor boundary, [Tokio](https://docs.rs/tokio/1.53.1/tokio/) runs IO and bounded blocking work, and [getrandom](https://docs.rs/getrandom/0.4.3/getrandom/fn.fill.html) supplies the session secret. Direct versions are pinned and the Cargo lockfile is updated with the implementation.

## Validation evidence

- 48 frontend regressions: the original draft/setup tests plus checked HTTP responses, mismatched IDs/revisions/engines/API versions, cancellation, invalid results, session restart without replay, and current-revision download gating.
- Six Rust service integration tests: exact local-origin/session checks, capabilities and limits, unknown routes/path traversal, full Inkscape coordinate/hole projection, schema migration, invalid/oversized requests, and configured M4 job round-trips.
- Four live integration checks launch the real server and CLI: production assets; exact Inkscape job/rings/holes/missing-settings parity; configured jobs and old/future schemas; changed placement and document fingerprint invalidation.
- Production TypeScript build, Rust formatting, and warning-free service Clippy checks.

The in-app Chromium browser displayed live Rust 0.6.0 normalization, rejected negative stock values, retained `1e-` through reload, rejected unsupported text SVG with `SVG_TEXT` and source `label`, imported a new rectangle, and validated scale 1.4/rotation 27/stock 10. A real service restart produced a session error; Reconnect preserved the edited name and stock and obtained fresh validation. The new import dialog fits a 360×800 viewport and contains keyboard focus through a complete Tab cycle; Escape closes it. No application console errors appeared during the successful workflow.

The browser emitted the expected portable download name and 1,562-byte Blob, but the embedded browser canceled the filesystem save (`Page.downloadProgress: canceled`). Its documented download event API did not complete, and raw download-policy overrides are unsupported. Capturing that actual generated Blob for testing and submitting it to the same Rust CLI succeeded with 21 fields unset. **An ordinary browser filesystem download still needs qualification; this run does not claim it completed.** The UI reports a download request and asks the user to confirm saving in the browser's downloads, rather than claiming filesystem completion. Temporary browser probes were removed by reload, and viewport overrides were reset. Browser fixtures/evidence remain in ignored `web/test-results/browser/`.

Some transformed versions of the full Inkscape coupon produce the existing Rust `QUANTIZATION_TOPOLOGY` rejection. Integration preserves that result; it does not relax geometry checks. A separate supported rectangle verifies successful transformed placement.

## Next integration work

U3 needs stage-specific readiness, immutable planning snapshots, background task identity/events, cancellation/reconnect semantics, and compact recorded-motion/stock views. M5 verification and M6 exact-output/profile integration follow those task/artifact contracts. Native file saves/conflicts, durable recovery, full browser/assistive-technology qualification, 3D, and release packaging remain later scope. See the [run instructions](../../../flat-v-carve/web/README.md).
