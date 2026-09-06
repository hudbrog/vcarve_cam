# U3: stock slices and 2D inspection

The later [plan artifact storage update](u7-plan-artifacts.md) replaces this
report's 16 MB plan cap and full-plan worker transfer with streamed temporary
files. The stock display budgets described here remain in effect.

Date: 2026-09-05

The integration branch was rebased without conflicts onto `d01ddc3` (Rust 0.7.2, including M5 verification and M6 LinuxCNC output). The two earlier web-integration commits became `a7dceb1` and `6b9a10b`. This slice adds bounded 2D stock inspection using existing core analysis; it changes the service and frontend, with no new machining calculations.

## Delivered behavior

- Inspect stock after the endmill or both tools at the depths actually reported by the planner. Depth is positive below stock top and displayed with its negative workpiece Z.
- Toggle nominal target, lower/upper removal bounds, remaining target, and possible overcut. Endmill slices additionally expose accessible floor, missing floor beyond tolerance, and requested tool centers.
- Inspect core-produced area metrics and fit a region from its area button. Supported per-slice missing-floor and possible-overcut diagnostics link to their actual regions. Diagnostics without geometry retain their text and receive no invented location.
- Filter recorded paths by tool and path layer. These filters never recalculate stock or change the job revision. Stock still represents all contributing motions for the chosen stage, including motions omitted from the bounded path preview.
- Preserve compound rings and holes. Source placement is applied only to source display geometry; stock and motions already use workpiece coordinates and receive only the SVG Y-axis inversion.
- Hide stale polygons and motions synchronously after job edits, stage changes, or service identity changes. Depth requests are abortable and keyed to the accepted task, slice, and refresh attempt. Late responses cannot replace another selected depth. Loading, retry, eviction, and display-limit states are explicit.

Remaining target is the nominal section outside the lower removal bound. It can include intentional allowance and tool-limited material; it is not synonymous with missed reachable floor. Possible overcut is the upper bound outside the target, not proof of an actual gouge. Combined slices have no separate per-slice verdict. These views retain M3/M4 fixed-depth evidence and do not establish M5 continuous-volume verification.

## Transport and resource bounds

The wire version is **`ui-3`**; rebuild the UI and restart the service together. Job schema 3 and plan schema 1 are unchanged.

`GET /api/v1/tasks/{id}/result` now includes compact `stockSlices` metadata with depths, metrics, bounds, diagnostic summaries, and availability. `GET /api/v1/tasks/{id}/slices/{slice}` returns one complete slice projection with its task identity. Both require the session header and inherit the service's same-origin and loopback checks. Missing slice IDs return `SLICE_NOT_FOUND`; evicted results return the existing `PLAN_RESULT_UNAVAILABLE` response.

The worker captures display data from live core analysis before serialization: engine 0.7.2 omits derived analysis from portable plans and regenerates it during inspection. Display metadata and polygons remain outside the portable artifact. No browser HTTP request accepts a filesystem path or launches the CLI.

Capabilities expose 60,000 vertices per slice and 200,000 across a result. A slice that exceeds either budget keeps its exact metrics but has no polygons and an explicit reason. Rings are never partially transferred. If the worker's 32 MB transfer budget is exceeded, it retries with stock geometry omitted; an artifact that still cannot fit fails explicitly. The 16 MB portable artifact cap, first 20,000 motion preview, latest-four-result retention, and task execution/cancellation limits remain as documented in the background-planning report.

Input limits were aligned with Rust 0.7.2: 32 MB SVG, 64 MB job JSON, and 128.1 MB HTTP request envelope. Inputs within these caps can still exceed planner or result limits. Import defaults wall allowance to 0 mm while leaving cutting settings unset; UI explanations now match that behavior.

## Validation

- 57 frontend regressions pass. New coverage rejects partial stock polygons, mismatched task/depth metadata, and unavailable geometry with contradictory payloads; it also checks preserved holes, workpiece projection, and filtering without motion mutation.
- Ten Rust service tests pass, including exact projection of core rings/holes/areas and retaining metrics when a display budget is exhausted.
- Eleven real service/CLI integration tests pass. Five planning cases additionally regenerate analysis through CLI inspection and compare every available slice depth, contributing-motion count, radial error, ring, hole flag, and coordinate with the HTTP result. The active-cancellation fixture uses a bounded sampling workload that remains cancellable with the faster engine.
- Production build/typecheck, 13-struct contract drift check and captured geometry parity, Rustfmt, Clippy with warnings denied, and whitespace checks pass.
- Browser checks use the combined island and unsupported-entry fixtures. They cover preserved holes, depth switching, tool/layer filtering, region fitting, missing-floor links, edit invalidation, light/dark presentation, and responsive controls. The island result has 2,860 recorded motions. View changes preserve revision 1; editing its name advances the revision and immediately removes both overlay types. The unsupported-entry result retains its incomplete outcome, zero motions, and positive missing-floor area.
- The 800 px layout exposed a drawer overlap when inspection controls wrapped. The viewport now retains room for its drawing and the drawer shrinks or scrolls. Rechecks at 800 px and the 640 px stacked layout passed, and the final browser console had no errors or warnings. Temporary viewport/theme overrides were reset.

These synthetic jobs are test fixtures, not machining presets. The existing embedded-browser download restriction remains outside this slice.

## Next integration

M5/M6 backend milestones no longer block frontend progress. The next functional slice is verification and output: immutable verification tasks, bound-aware report review, the separate LinuxCNC profile editor, exact formatted-output checking, and downloads gated by authoritative current identities. After that, the larger visual/release work remains: 3D and arbitrary sections, playback, real-artwork rendering scale, native file lifecycle, durable recovery, and packaging.
