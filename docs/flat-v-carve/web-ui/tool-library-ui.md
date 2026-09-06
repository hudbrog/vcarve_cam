# Tool library frontend integration

## Plan and scope

Use the Rust library from main (`706ba8d`) through the local service. Keep the existing frontend worktree and machining ownership intact.

1. Add bounded, session-protected library transport using `ToolLibraryStore` directly. The server chooses one persistent local directory, with a CLI override for tests or an existing library. Initialization is explicit; corrupt data is never replaced. Expose current revisions and preserve Rust conflict/error codes.
2. Add a library dialog in Carve & tools with name/ID/context search, cutter filtering, complete tool and preset editors, duplicate/remove, portable import/export, and capture from either job slot. No sample library or inferred cutting settings.
3. Add selection review showing current and replacement dimensions/capabilities/cutting values. Geometry-only selection clears all five cutting values. Revalidate the selected library revision when applying, reject intervening job edits, and apply as one undoable edit while retaining unrelated draft text. Keep job IDs and machine mappings unchanged.
4. Keep failed/conflicting edits in the editor. Reloading the library never silently updates an edit's expected revision; the user must reopen the latest record before saving again. Library changes do not invalidate existing job snapshots; applying actual job changes does invalidate planning and output.
5. Verify persistence/reopen and CLI parity, conflicts, rejected imports, partial/null values, reordered job slots, undo/staleness, and the browser workflow in a separate test tab.

Library writes are explicit Save/Import/Remove actions. Deletion has a record-specific review in the dialog. Library edits stay in memory while the dialog is closed; durable recovery of unsaved library edits is outside this slice, alongside the remaining general file/recovery work.

## Implementation and validation

Implemented on `codex/web-integration`, rebased onto backend commit `706ba8d`.
The existing `cam_app::tool_library::ToolLibraryStore` owns persistence and
`cam_core::tool_library` owns validation/application. No machining algorithms or
portable job schemas changed in this integration.

The browser exposes search/filter, complete endmill/V-bit editors, nullable
capabilities, nested cutting preset CRUD/duplication, tool duplication/removal,
raw JSON import and portable library download, and capture from either job slot.
New libraries are empty; geometry and cutting settings are entered or captured
explicitly. Context labels make no machine/material suitability claims.

Apply loads Rust-validated library records for review, then reloads them on the
final click to recheck the library revision, service identity and selected tool
settings. The browser copies only the chosen geometry, capabilities and cutting
values into the draft; it does not require a valid whole-job receipt to edit a
tool. Draft revision checks reject intervening edits, and the reducer enforces
the revision again. Only the intended tool can
change. Matching selections can be confirmed to associate the library tool and
preset names without a job revision or undo entry. Actual changes are one undo entry, retain
unrelated/inactive draft text, and invalidate the existing plan/output as normal.

### Service contract

- UI wire version **ui-6**, engine **0.7.2**, library schema **1**, job schema **3**.
- `--library-dir <directory>` selects the service-owned local store. Otherwise,
  use the platform application-data location documented in `web/README.md`.
  Neither starting the service nor loading a missing library initializes it.
- `POST /api/v1/library` handles load, initialize, change, and import with a
  16,100,000-byte request limit (8 MB raw library JSON plus envelope escaping).
  `POST /api/v1/library/job` handles capture and apply with the existing bounded
  job/request limits. Both share the two blocking document-worker permits.
- Requests bind API version, service instance, and request ID. Job requests also
  carry a job revision and matching Rust document fingerprint. Browser inputs
  cannot choose filesystem paths. Existing Host/Origin/session checks apply.
- Changes and imports pass the original JSON string to strict Rust parsing.
  Duplicate/unknown fields are rejected. Writes use the expected library revision;
  lost responses require reload and never trigger automatic retries.
- Missing, corrupt, conflicting, and busy stores remain distinct outcomes. A
  failed save keeps the form. Reload never advances its frozen expected revision.

### Validation evidence

- Frontend: **77 tests**, typecheck/production build, and contract/display checks.
  Coverage includes three Rust library field-set drift alarms, null/partial/zero
  values, reordered role lookup, unchanged job IDs/machine settings, five-field
  clearing, one-step undo, no-op/stale reducer guards, and candidate rejection
  when identities or unrelated settings differ, including responses arriving
  after reconnect and mismatched response request IDs without replaying writes.
- Live service/CLI suite: **22 tests** including capture/export/apply parity,
  explicit initialization, persistence through a fresh client, all preset
  mutations, independent CLI write conflicts, kind/receipt rejection, failed
  import byte preservation, strict duplicate fields, one-revision import, and
  continued validity of job snapshots after deleting their library record.
- Rust server: **15 tests** including no creation on read, corrupt-data protection,
  session/instance enforcement, strict command fields, route classification,
  and the metadata body limit. Clippy passes with warnings denied.
- In-app browser, synthetic M4 wide-floor job and isolated
  `artifacts/web-library-preview`: explicit initialization; capture with preset;
  geometry-only review showing all five cleared values; apply and one Undo;
  partial numeric text; stale CLI edit rejection; reload retaining the old form
  revision; manual pointed V-bit with literal zero tip and nullable capabilities;
  partial preset round-trip; wrong-slot apply disabled; rejected and accepted
  imports; keyboard focus wrapping; library download request; context search and
  cutter filtering; persisted records after service and tab restart. No browser
  console errors or warnings were reported on the final build.

The in-app browser confirms download requests but does not establish on-disk
Blob saves; portable library contents were checked against CLI export. Library
forms have in-memory retention while the dialog is closed, not durable recovery
after a tab reload. Saved library data is durable through the existing Rust store.
Native file/save conflict UX, general durable job recovery, packaging, 3D/playback,
and separate setup/machine preset libraries remain outside this feature.

## Workflow follow-up (2026-09-06)

The initial explicit computation-starter action described below was subsequently
replaced by automatic defaults; see the defaults follow-up at the end.

The library's apply panel now comes before the geometry and management details.
It requires an explicit cutting-preset or geometry-only choice, separates review
from final application, and labels the selected record as selected for review.
After applying, Carve & tools shows the tool and preset names beside the job's
values. These display labels survive same-tab draft recovery; changing the tool
values marks the association as edited. They are not serialized into portable
jobs or used as machining evidence.

Rust's missing-setting list is now visible in Issues and before Generate, with
human-readable labels and links that open the right editor. Missing V-bit
computation settings block combined submission up front; endmill-only guidance
omits V-bit cutting/computation settings, and ramp entry omits the unused endmill
plunge feed. Tool-ID paths are resolved to editor indices after role lookup.
Unreferenced tools do not block the selected stage. Actual feasibility remains
the Rust planner's responsibility.

The V-bit computation group explains why library cutting presets do not fill
it. Its explicit starter action fills only blank resource/sampling fields as one
undoable edit. It preserves entered values, including zero and partial text,
and becomes disabled once no blanks remain. No cutting parameters are supplied.
Native select options and their controls use matching theme background/text
colors, including in the library.

Validation: 82 frontend tests, 23 live service tests, typecheck/production build,
and 16 Rust field-set/display contract checks pass. The added live regression
starts with only `vbit_planning` missing and completes a combined plan after the
explicit starter action. Browser checks in a separate synthetic-job tab confirm
the visible issue and disabled Generate button, linked editor focus, explicit
preset selection, matching-value confirmation, geometry-only five-field review,
applied labels, and one-step Undo. The combined browser plan completed with 392
recorded motions. Dark select and option computed colors are `rgb(28,38,49)` and
`rgb(224,232,242)`; visible controls and the apply panel were inspected. The
embedded browser did not expose a native popup screenshot, so popup rendering
was checked through option styles rather than a captured open menu. No browser
errors or warnings were reported. These synthetic checks did not modify saved
library records or the user's open job tab.

## Automatic computation defaults (2026-09-06)

Empty planning-budget and numerical-tolerance fields now resolve to visible
defaults before Rust validation, submission and portable download. This removes
the manual computation-block setup step. Existing numbers, zero and partial
numeric text retain their meaning. Endmill travel and entry remain explicit;
budget defaults do not create an incomplete travel block by themselves. Library
application preserves unrelated draft defaults rather than freezing them into
overrides. Portable downloads intentionally freeze resolved numeric values.

The work ceilings use the current Rust-supported maxima instead of the smaller
starter caps. One **Use default planning budgets** button resets work overrides
without changing sampling, cleanup, cutting values, tolerances or finish targets.
Advanced sections explain each override and provide separate reset actions.
Defaults are 0.01 mm motion tolerance and 0.05 mm verification tolerance, with
1 mm planning sample spacing, 2 cleanup iterations and 8 stock slices. Accuracy
defaults stay fixed across geometry/tool edits and are never relaxed on failure.
The Rust API/CLI job schema and engine resource protections are unchanged.

Budget/precision diagnostics link to their editors, including issues attached to
partial results. A failed task from an older revision no longer appears as a
current issue after the setup changes; its diagnostic remains in task history.

Validation: 87 frontend tests and 24 live service tests pass, including missing
computation/tolerances resolving automatically, a deliberately low layer limit
followed by successful planning after resetting that override, preservation of
cut/accuracy settings, undo, portable round-trip, and unrelated library apply.
Typecheck and build pass; the main minified bundle is about 504 kB and triggers
Vite's advisory 500 kB chunk warning. No build checks were disabled.

The separate synthetic browser check also confirmed default labels and advanced
help in dark theme, same-tab recovery of automatic values, the low-layer-limit
failure linking to its editor, reset to defaults, and a complete 392-motion
combined plan. The previous failure stayed out of current Issues after editing.
No browser console errors or warnings were reported.

## Library selection on unfinished drafts (2026-09-06)

Review previously required successful validation of the entire editable job.
Incomplete travel, partial numeric input, or an invalid supplied setting could
therefore disable tool loading even when a library selection would repair it.
Review and Apply now work on the draft directly using freshly loaded,
Rust-validated records. The comparison includes partial target-tool text. Other
unfinished fields and automatic computation defaults are retained, and ordinary
Rust job validation still gates planning. Capturing a job tool into the library
continues to require the current valid job receipt. The Rust apply API is unchanged.

Applied labels compare the tool's draft fields, so unrelated unfinished settings
no longer mark a library tool as edited. Matching selections do not create a new
revision. Library/connection changes and intervening draft edits reject stale
reviews; target job IDs and machine mappings remain unchanged.

Validation: 92 frontend tests, 25 live service tests, typecheck/build and contract
checks pass. Live checks compare draft copies with Rust application for both
cutter kinds, full/partial presets and geometry-only clearing, and verify that
remaining invalid settings still fail Rust validation. A separate browser tab
confirmed enabled Review and Apply with unfinished travel and a `1e` diameter,
partial-value comparison, correct applied labels, one-step Undo/Redo, preserved
clearance and blank start positions, and disabled planning until setup is ready.
Matching confirmation kept the same revision; no browser errors or warnings
were reported. Saved library records and the user's open job tab were unchanged.
