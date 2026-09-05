# Web UI: integration and delivery plan

Date: 2026-09-05\
Status: delivery roadmap; U1/U2, bounded U3 planning/2D inspection, and U5 M5 verification/M6 output are implemented.\
Companion: [product and interaction design](README.md).

Implementation update: [U2](u2-service-integration.md), [background planning](u3-background-planning.md), [stock inspection](u3-stock-inspection.md), [M5 verification](u5-verification.md), and [M6 output](u5-linuxcnc-output.md) implement the local service through `ui-5` with Rust 0.7.2. Import/open/display/validation, cancellable tasks, artifact access, recorded motions, stock slices, and continuous verification are live. M6 profile editing, cancellable output checks, report review and gated program downloads are live. Native-file operations below remain future work. The operation tables describe the overall target and are not a frozen HTTP specification.

## 1. Ownership and independent progress

Rust remains authoritative for SVG normalization, geometry, machining rules, planning, stock/reachability calculations, verification, artifact identity, migration, and postprocessing. TypeScript owns forms, navigation, local draft text, selection presentation, cameras, display rendering, and task/result presentation.

The UI can start against deterministic fixtures through a replaceable service adapter. It must not call the CLI by shell command from browser code or copy machining formulas into client validation. The production adapter talks to the local application service; the fixture adapter exercises the same UI contract. CLI behavior remains a parity oracle for identical inputs and engine versions.

Integration runs in the `codex/web-integration` worktree. The frontend and `cam-server` adapt the core APIs; backend milestones no longer block this work. No hosting infrastructure is configured.

## 2. Current capability inventory

| Capability | Current evidence | UI integration work |
| --- | --- | --- |
| SVG import and inspection | Embedded source, normalized source components, physical bounds, stable selection IDs, diagnostics. | Transport DTO, inert rendering, inclusion controls, field/source associations. |
| Portable jobs | Schema 3, migrations from 1/2, nullable machining fields, validation. | Draft-to-job boundary, compatibility responses, recovery and file operations. |
| Endmill/combined plans | Actual XYZ motions, tool transition, fingerprints, partial results. | Async task envelope, summaries, motion access and cancellation. |
| Stock/quality inspection | Layer/slice polygons and point samples with residual/reachability evidence. | Display primitives, spatial queries, chunking, stable overlay identity. |
| Diagnostic model | Code, warning/error severity, stage, message, optional source ID. | Optional field paths, region/motion IDs, location/bounds, measured-limit data and remediation hints. Missing fields remain absent. |
| Continuous stock verification | M5 service/UI implemented for combined plans; M4 planning retains slice/sample quality scope. | Reuse bound-aware review and immutable identities for M6 output checks. |
| Machine profile/output | M6 profile editing, combined/per-tool output, exact-byte readback checks and gated downloads are integrated. | Native file lifecycle and actual machine/controller integration. |
| Local browser service | `cam-web`, same-origin `ui-5` API, shared planning/verification/export queue and bundled UI are implemented. | Release lifecycle/file integration. |
| 3D and arbitrary cross-sections | Debug SVGs and core geometric data exist; no browser display API. | Engine-derived display meshes/heightfields and section queries, with error/resolution metadata. |
| Local tool library | [Rust API and CLI](../tool-library.md) implement named tools, cutting presets, revisioned persistence, import/export, and job snapshots. | Service transport, library management controls, changed-value review, and undoable application. |

Rust 0.7.2 supports 32 MB SVG sources and 64 MB job JSON. The service advertises these limits and a 128.1 MB request envelope limit through capabilities; saved plans and display responses retain separate bounds. Handle large result data separately from interactive summaries.

## 3. Proposed service interface

The names below describe operations, not frozen URLs. Final transport types should be generated from, or mechanically checked against, Rust-owned schemas. Job schema, plan schema, engine identity, and API version are distinct values. Do not treat the current `planning_available` boolean as combined-plan or export eligibility.

| Operation | Request | Response and behavior |
| --- | --- | --- |
| Capabilities | UI API version | Engine/API identity; supported artifact schemas, planning stages, verification scopes, render features, resource limits, export formats. A missing capability is unavailable. |
| Import artwork | Source bytes, filename, import options | Editable job plus normalized geometry and diagnostics; failed import does not replace the current job. |
| Inspect / validate draft | Draft revision, candidate job, requested purpose | Canonical job when valid; normalized geometry as appropriate; missing/invalid fields and stage-specific eligibility. May preserve a valid incomplete job. |
| Start plan | Immutable job snapshot, revision, explicit stage, idempotency key | Task ID and accepted input identity. Heavy work executes outside the request handler. |
| Task events / snapshot | Task ID, last event sequence | Ordered progress and latest state. Reconnect obtains a snapshot even if event history was trimmed. |
| Cancel task | Task ID | Cancellation acknowledgment followed by terminal state; a cancellation request is not proof the worker stopped. |
| Result summary | Artifact ID | Identity, stage results, scoped evidence, available views, compact diagnostics and limits. |
| Display data | Artifact ID, view, slice/section/detail request | Derived geometry/motions or bounded chunks, coordinates/units, resolution, diagnostic linkage. Never a new authoritative verification result. |
| Verify artifact | Artifact identity, requested scope/settings, idempotency key | Verification task; independent report bound to the exact inputs. Any required job setting change creates a new revision. |
| File open/save | Selected file reference and document; expected prior file identity for save | Migration/validation or atomic save result. No silent external-file overwrite. |
| Export | Plan identity, machine-profile snapshot identity, output options | Task/result with exact-output verification and files only when authoritative checks pass. |

A proposed result envelope should carry at least `api_version`, `engine_version`, `request_id`, `job_revision`, `input_fingerprint`, `task_id`/`artifact_id` as appropriate, and typed diagnostics. Existing artifact fingerprints stay engine-produced; the client must not recreate Rust JSON hashing.

The local server should use loopback binding, serve bundled assets and API from the same origin, reject unexpected Host/Origin values, and authenticate mutating local API requests using a session mechanism. This is an application boundary for local files and expensive computation, not an account feature. Normal operation must not require remote fonts, CDN code, analytics, or uploaded artwork. Choose and validate the exact session/file-dialog mechanism during service implementation.

## 4. State, identity, and cancellation

Model independent state dimensions instead of one overloaded `status`:

| Dimension | States / values |
| --- | --- |
| Document | No job; editing revision; valid incomplete job; valid configured job; invalid draft text. |
| Persistence | Unsaved changes; recovery saved; saved to file; external conflict. |
| Service | Connecting; available; disconnected; incompatible. |
| Task | Queued; running; cancellation requested; cancelled; succeeded; failed. |
| Planner outcome | Complete; empty; incomplete; inconclusive, retaining the engine's scope and meaning. |
| Verification | Not available; not run; running; passed; failed; inconclusive, with named check scope. |
| Freshness | Current; stale; incompatible engine/schema. |
| Output | Unavailable; blocked with reasons; generating; failed; checked files available. |

Task success means the computation returned a result; it does not mean the planner completed or verification passed. An empty endmill stage can be valid for a V-bit-accessible region, while an entirely empty selection cannot generate a useful combined job. Read the engine's interpretation instead of translating every empty state into an error or success.

Revision rules:

1. Commit each document edit as a monotonically increasing revision. Keep transient numeric text separate until parsed; invalid text blocks computations based on that draft.
2. Start tasks from an immutable snapshot and store its engine-issued input identity.
3. Apply progress only to its task ID. Deduplicate/reorder events by sequence and reconcile after reconnect.
4. A terminal result becomes current only when its identity matches the current accepted job and engine. Otherwise retain it as a labeled previous result.
5. Cancellation/completion races are resolved by service terminal state. A superseded or cancelled task never silently installs a current result.
6. Verify/export recheck identities server-side at time of use. Client button state is only a helpful reflection of that decision.

Conservative invalidation baseline: every serialized job change invalidates the current plan because the current engine hashes the whole job, including metadata/profile fields. Camera, visible overlays, and inspector navigation do not change the job. Finer dependency-based reuse is a later Rust contract; the UI must not promise that a name/profile edit preserves current fingerprints.

An export binds the current job, motions, verification settings/evidence, machine profile, postprocessor configuration, output precision, and resulting bytes. Output formatting can fail after a geometric plan passes. Keep that failure separately inspectable and never reuse a checked-download status after regeneration with changed options.

## 5. Data and rendering strategy

Maintain four separate stores: portable job data; recoverable form text/edit history; immutable artifact/task metadata; view state such as camera, selected item, and layers. Do not serialize view caches into the strict Rust `Job` schema. A local preset/recent-job index is separate application data with its own versioning.

The initial viewport renders normalized polygon rings and motions through a renderer abstraction. Use SVG for modest geometry and evaluate a GPU renderer with the large fixtures before committing to a library. DOM forms and accessible lists remain usable regardless of renderer choice. The full-release 3D view consumes engine-derived display data; display tessellation, culling, and camera math are UI concerns, but target depth, stock removal, and verification are not.

Avoid transferring whole large JSON artifacts for every control change. Summaries, diagnostic metadata, and bounds should arrive first; use artifact-keyed geometry/motion chunks or typed buffers for heavy views. Cancellation of display requests and worker-based decoding should prevent obsolete work from blocking the latest view. Resource exhaustion produces a reduced visual representation with a visible resolution label or a clear failure; it never changes evidence silently.

Initially reuse captured real CLI artifacts as immutable fixture sources. Fabricated task/error sequences are acceptable for UI lifecycle testing when clearly labeled; they must not be described as engine verification evidence. Keep synthetic cutting settings in examples and tests only.

No frontend framework, HTTP framework, mesh library, or package version is approved by this plan. Select them at U1 using build/distribution simplicity, accessible forms, state management, testability, and measured renderer workload. The stable architectural choice is a thin TypeScript UI and one local Rust service. WebAssembly and remote hosting remain outside the initial application path.

## 6. UI delivery stages

These U stages expand M7; they do not rename or block the Rust M stages. Advance design and fixture-driven UI while backend capabilities are pending.

| Stage | Deliverable | Can proceed against | Exit evidence |
| --- | --- | --- | --- |
| U0 — design baseline | Screen map, interaction model, capability gaps, normal/error wireframes, terminology. | Current docs and M2–M4 fixtures. | User can review the guided workspace, state meanings, and primary workflow. This planning pass starts U0; remaining scenario designs are still open. |
| U1 — shell and contracts | Project scaffold, form conventions, mock/live adapter boundary, schema checks, navigator/viewport/inspector. | Approved interface proposal and fixtures. | All steps navigable, editable incomplete draft retained, themes/keyboard/resizing work. |
| U2 — artwork and setup | Open/import, physical placement, region inclusion, stock/tools/quality settings, save/reopen, undo/recovery. | M2–M4 models plus local import/file/validation service. | Existing SVG/job fixtures round-trip through the UI without JSON editing or invented values. |
| U3 — plan and 2D inspection | Background tasks, stage progress, cancellation, stale-result protection, paths/slices/issue drawer. | M3/M4 and task/display adapters. | Complete, empty, incomplete, and inconclusive results are inspectable; editing during planning cannot install old output. |
| U4 — full visual inspection | 3D stock view, section tools, motion playback, layer selection, large-artwork interaction. | New engine display/query contract plus U3. | Source, desired shape, each tool's result, and residuals can be compared; approximation labels survive every view. |
| U5 — verification and output | Bound-aware review, spatial diagnostics, machine profiles, formatted-output preview, combined/per-tool downloads. | M5/M6 capabilities. | Failed/inconclusive/stale outputs are gated server-side; exact checked files match the reviewed snapshot. |
| U6 — usable local release | Local presets, file conflicts/recovery, offline packaging, lifecycle/browser checks, help and first-run workflow. | Chosen target OS/browser and integrated service. | Ordinary import-to-export workflow, restart/recovery, CLI parity, and supported installation are demonstrated. |

U3 now has bounded stock slices, tool/path-layer filters, and links from supported slice diagnostics to geometry. U4 still needs 3D/section display contracts and large-artwork rendering work. U5's M5 verification and M6 profile/output workflows are implemented, including exact-output parity and server-side outcome checks plus browser freshness gates. Remaining delivery work is U4 inspection and U6 release/file lifecycle. No calendar commitments are made here.

## 7. Acceptance scenarios

| Scenario | Required observable behavior |
| --- | --- |
| Actual Inkscape SVG and compound letters | Physical bounds and inclusion match Rust inspection; holes remain preserved. |
| Unsupported text/stroke/reference | Import fails visibly with a remedy and available source association; current job survives. |
| Empty selection or missing tool feeds | Draft saves; stage-specific missing requirements prevent planning. |
| Scale/rotate/change origin | Workpiece axes and dimensions follow the documented transform; only intended settings change. |
| Pointed V-bit and zero ridge requiring area clearing | Show the Rust rejection linked to quality/tool fields; do not repair the request automatically. |
| Finite-tip detail and missed reachable floor | Display as separate findings, with different labels and available bounds. |
| Narrow region with no endmill access | Inspect empty endmill stock and subsequent V-bit behavior without suggesting the job has no usable geometry. |
| Exact-fit/contact and resource-limited fixtures | Preserve inconclusive/partial status and limited evidence; never substitute a complete-looking preview. |
| Edit during plan; undo; out-of-order result | Each task retains its identity; only a matching artifact can become current. |
| Cancel near completion; disconnect/reconnect | Service terminal status wins; previous complete artifact is preserved and current evidence remains correctly labeled. |
| Reopen old or edited artifact | Recompute/reject as the engine requires; cached analysis cannot establish success. |
| M4 complete result | UI says slices/samples and continuous clearance at their actual scope; machine output remains unavailable. |
| M5 failed/inconclusive; M6 rounded-motion failure | Exact failure is surfaced; machine program download is blocked despite any earlier passing stage. |
| Combined/per-tool export | Downloaded bytes/profile identity match the reviewed result; per-tool setup independence is checked in Rust/M6 tests. |
| Unsaved changes, malformed draft, external edit, two tabs | Recovery is distinct from portable save; conflicts do not overwrite unnoticed. |
| Large motions/components and narrow screen | Typing, cancellation, navigation, and issue selection remain usable; drawing detail reduction is explicit. |
| Keyboard, zoomed text, light/dark, reduced motion | Essential tasks and findings remain accessible without hover or color alone. |
| Offline app startup and restart | Bundled UI works with the local service; version mismatch or service loss gives a recoverable state. |

Use focused UI state tests for identity/cancel/recovery/export gating, browser scenarios for the ordinary workflow and key failures, and contract fixtures checked against Rust. Reuse existing machining tests; do not duplicate planner mathematics in frontend tests. Compare normalized settings, identities, motion sequences, and report semantics for CLI parity rather than relying on screenshots.

## 8. Decisions to hand to Rust integration

The next concrete integration review should settle:

1. Capabilities and stage-specific validation DTOs, including field paths and nullable capability semantics.
2. Task identity, monotonic events, cancellation acknowledgment/terminal behavior, and restart persistence policy.
3. Artifact metadata versus heavy display data; normalized coordinates, per-view resolution, spatial diagnostic references, and section/mesh query ownership.
4. Current whole-job fingerprint invalidation and whether future versions intentionally separate geometry, plan, profile, and output dependencies.
5. Clearance-field reconciliation and the complete M6 machine/compensation/precision schema.
6. Export eligibility and report scopes for M4, M5, and formatted output; a machine-readable distinction between sample evidence and proven bounds.

Until agreed, these are proposed adapter requirements. The frontend must remain capable of displaying older engines' supported results without pretending missing capabilities exist.
