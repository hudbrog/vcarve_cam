# U3 background planning and recorded motions

Date: 2026-09-05. Implemented in the isolated `codex/web-integration` worktree, on top of `fabc02f`. This is the background-planning slice of U3; stock slices and richer inspection remain follow-up work. No `cam-core` or `cam-app` source changes are required, so parallel M6 development remains separate.

Subsequent update: the [stock-inspection slice](u3-stock-inspection.md) rebases this work onto Rust 0.7.2 and delivers depth slices, tool/layer filters, and supported diagnostic locations. That report supersedes the wire/input-limit values and remaining-U3 status below; this report retains the original implementation evidence.

## Delivered workflow

The production browser workspace can submit endmill or combined plans from an immutable, Rust-identified job snapshot. It shows queued/running/cancelling/terminal task state, preserves engine diagnostics and complete/empty/incomplete/inconclusive outcomes, and displays the recorded XYZ motions projected into XY. Endmill, V-bit, and optional travel overlays have distinct colors. Fit plan uses the recorded coordinates; artwork placement is not applied a second time.

Editable-job validation is a prerequisite for submission, not proof of stage readiness. The selected core planner checks its own setup and returns its actual diagnostic before calculating when settings are missing. There is no duplicate readiness algorithm in TypeScript and no separate exhaustive stage-eligibility endpoint yet. Task success means a calculation returned an artifact; it does not imply a complete plan or passed geometric verification. Engine evidence meanings and limitations remain visible.

Every task carries its service instance, engine, stage, immutable document fingerprint, draft revision, task ID, and monotonic sequence. Explicit retries use the same ID and body; conflicting reuse is rejected. Polling reconciles snapshots without an event replay log. Network errors pause polling and expose explicit check/reconnect/retry controls. Neither disconnects nor reloads restart or cancel work automatically.

Edits, unfinished text, pending validation, stage changes, or a different service/engine immediately hide nonmatching motions. Prior results stay labeled stale. A tab recovers only its latest task handle through session storage, with no session token or plan data in the job. Since editable-draft revision numbers restart on reload, recovered plans conservatively remain stale until explicitly regenerated. Undo also advances the revision and requires a new plan. Camera, overlay toggles, and navigation do not edit the job.

## Execution and cancellation

The existing core planners are synchronous and expose no cooperative progress/cancellation hooks. The service launches its own executable in a private `--planning-worker` mode, supplies a bounded job/stage JSON message through stdin, and reads a bounded structured reply. It calls the core planners directly. It does not invoke `cam`, a shell, or a user-selected executable/path. Windows workers use `CREATE_NO_WINDOW`; workers do not construct an async runtime.

One worker calculates while up to three more tasks wait. Document inspection retains its separate two-worker budget, so setup editing and validation stay responsive. Progress is deliberately coarse: queue, selected planner running, cancellation requested, and terminal state. No invented percentage or internal combined-stage progress is presented.

Cancelling a queued task releases its slot without computation. Cancelling a running task kills and reaps its process before reporting `cancelled`. Under the ledger lock, cancellation wins if requested before result installation; a result installed first remains inspectable. A killed task never installs an artifact. A five-minute timeout terminates/reaps the worker and returns a failure. Normal Ctrl+C shutdown closes task admission, cancels pending work, and waits for worker exit. Unexpected process/OS termination is not durable task recovery; a new service instance reports old handles as lost and never replays them.

## Transport and limits

The checked wire version is now **`ui-2`**. The existing `/api/v1` route namespace remains; it is independent of the wire and portable artifact schema versions. Rebuild the UI and restart `cam-web` together.

| Route | Behavior |
| --- | --- |
| `POST /api/v1/tasks` | Submit `{apiVersion, instanceId, requestId, revision, documentFingerprint, stage, job}`; return an accepted task snapshot. |
| `GET /api/v1/tasks/{id}` | Latest identity, monotonic sequence, task state, diagnostic, summary, and result availability. |
| `POST /api/v1/tasks/{id}/cancel` | Request cancellation; return authoritative state, possibly already terminal. |
| `GET /api/v1/tasks/{id}/result` | Identity-bound motion preview in `workpiece-mm-z-up` coordinates. |
| `GET /api/v1/tasks/{id}/artifact` | Original core-produced portable plan JSON, requiring the session header. This endpoint is available for integration and subsequent verification work; the UI does not yet expose a plan-download button. |

All routes inherit U2's exact loopback Host/Origin checks, session header, body/request limits, no CORS, and same-origin offline assets. Session secrets remain in memory. Capabilities advertise both stages and the actual task/display limits; verification and machine-output capabilities remain absent.

- Four unfinished tasks, one active planning process, five-minute execution limit.
- 128 immutable task records per service lifetime. Saturation is explicit; save jobs and restart to clear the ledger. Keys are never silently recycled in a live instance.
- The latest four successful artifacts/previews are retained. Older summaries and idempotency records remain; eviction increments sequence and marks the result unavailable. The result/artifact endpoint returns an explicit unavailable error.
- 16 MB per portable plan artifact, 32 MB worker reply, first 20,000 motions per display response. Preview omission counts are shown. The complete artifact is never silently truncated; serialization overflow fails the task.
- Summaries show up to 100 diagnostics and 100 generation issues, with omitted counts; the portable artifact retains the full engine evidence.
- Existing 8 MB job, 2 MB SVG, 16.1 MB request, eight concurrent requests, and two concurrent inspections remain in effect.

These are service/display resource limits, not new machining limits or proof tolerances. Runtime Zod schemas check responses; live tests compare Rust-produced replies and artifacts with the same-engine CLI. Fully generated bindings are not claimed.

## Validation

- 53 frontend regressions pass, including strict task/result schemas, all outcome states, monotonic update handling, identity rejection, immediate stale-result gating, explicit retry keys, and motion-coordinate projection without duplicate placement.
- Nine Rust service tests pass, including existing HTTP/normalization checks plus bounded queue admission, queued cancellation, idempotency under saturation, shutdown admission, cancellation/completion ordering, and explicit result expiry.
- Eleven real service/CLI integration tests pass. Both planners match exact CLI artifact JSON, input/motion fingerprints, and previewed motions. Fixtures cover complete, empty, incomplete, and inconclusive results; missing combined settings preserve Rust diagnostics. Validation stays responsive during planning; reconnect does not replay work; active cancellation yields no artifact.
- Production browser checks confirmed a combined plan (955 recorded motions), fit-plan rendering, editing during planning with no stale overlay, both cancellation/completion orderings, recovery of a cancelled task without replay, and restart detection. An incomplete endmill result retained its engine diagnostics in both the result panel and issue drawer. A larger synthetic sampling workload was used to hold a worker long enough to cancel through browser automation. The final browser preview had no console errors or warnings.

Build/typecheck, contract drift checks, Rustfmt, Clippy, and diff whitespace checks accompany this slice. Browser checks use synthetic repository fixtures only; their cutting parameters are not application presets. The U2 embedded-browser download restriction is unchanged and does not block planning.

## Follow-up

Complete the remaining U3 inspection work with engine-derived stock slices, layer/tool filters, and diagnostic locations. Add explicit stage-eligibility responses if the core exposes a shared preflight API. Geometric verification, formatted-output checks, M6 machine profiles, native file persistence, plan import/download UX, richer progress, 3D, and motion playback remain separate integration work. No G-code or independent geometric verification is enabled by this change.
