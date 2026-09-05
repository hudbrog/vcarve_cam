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

Apply performs two read-only Rust resolutions: the first produces the visible
before/after review; the final click rechecks the same library revision and
candidate fingerprint. Job revision/source receipt checks reject intervening
edits, and the reducer enforces the revision again. Only the intended tool can
change. No-op selections are disabled. Actual changes are one undo entry, retain
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
