# GUI4 implementation and review

GUI4 implements the portable project, artwork collection and explicit reference
repair slice from section 14.6 of the UI plan in the production `cam-gui` crate.
Starting commit: `3a7b243` (GUI3). User visual review and physical machining
acceptance remain pending. See [the manual review recipe](gui4-review.md).

## Requirement audit

| Requirement | Implementation and evidence |
| --- | --- |
| GUI4a: one portable project | Schema-5 jobs embed source bytes, placements, qualified assignments and applied machine settings. Explicit core migration, apply/save/reopen and independent reopening are tested. Open never restores execution/export trust. |
| GUI4b: artwork collection | Worker-owned single/batch Add, Duplicate, Reorder and independent placement. Qualified cross-source assignments use the real core selected-region union. Combined generation, stage simulation, exact checked export and portable reopen are exercised. |
| GUI4c: explicit repair | Replace/Delete preserve unresolved assignments across save/reopen. Repair binds the exact old reference to an explicitly picked current component. Referenced deleted owner IDs remain reserved. Tests cover invalid replacement, stale repair targets, empty collections and Undo. |
| Collection interaction | Hide/Lock are workspace recovery properties; hidden assigned geometry still cuts, and locked geometry remains numerically editable. Batch reads run off-frame with per-file errors and one accepted-additions Undo transaction. Single SVG drops append; New from SVG starts a project. |
| Draft and asynchronous safety | Source-qualified raw fields and widget identities survive row changes/recovery. Pending text on any source blocks generation. Revision-bound file and worker completions cannot replace newer edits. |
| H3 semantic currentness | The retained worker compares core machining identity. Unassigned-source edits and row ordering can reuse the unchanged plan; used-geometry changes make it stale. Tests verify one planning run through checked output preparation. |
| H1/H4 snapshots | Portable applied machine settings and exact prepared bytes remain separate from local recovery and retained execution state. Existing save/retry, recovery and cancellation remain covered. |

## Validation

- 54 GUI tests passed: 35 unit, 7 authoring, 6 collection, 2 actual native
  worker-process tests and 4 workflow tests. Log: `artifacts/gui/gui4-tests.txt`.
- 48 core tests passed across artwork commands, collection machining/resources,
  project-v5 and sequence planning. Log: `artifacts/gui/gui4-core-tests.txt`.
- Clippy with `--all-targets --locked -- -D warnings` passed. Native release
  and optimized browser builds completed; `git diff --check` passed.
- Final GUI3 browser regression: `artifacts/gui/browser-smoke/2026-09-12T05-48-42.447Z/evidence.json`.
- Final GUI4 browser acceptance: `artifacts/gui/browser-smoke/2026-09-12T05-50-38.324Z/evidence.json`.
- Earlier flower and new-SVG regression tours passed on browser build `91de1ce1a459`:
  `2026-09-12T05-29-53.656Z/evidence.json` and
  `2026-09-12T05-32-01.222Z/evidence.json` under the same smoke directory.
  Later changes concern collection/drop behavior and workspace validation;
  final GUI4 and GUI3 tours exercise the updated application.

The browser tours use real Chromium pointer/keyboard input, file chooser/drop,
WASM workers and actual downloaded bytes, with a read-only application probe.
An initial final GUI4 attempt reached migration but failed because the harness
resolved the legacy fixture from the wrong directory; that path was corrected.
Collection, sampled-stock simulation, unresolved references, hidden artwork and
batch-error screenshots were inspected. The rejected file is visibly identified
without discarding accepted artwork.

## Review builds

Paths below are relative to `flat-v-carve`. Generated builds and browser evidence
are local ignored artifacts, not repository source files.

- Windows: `artifacts/gui/native/cam-gui.exe`, SHA-256
  `1ccc48ae0900084fcc49454289cbd52a5f752e2b89f0b1b24aeb413cca96e8ea`.
- Browser: `artifacts/gui/browser`, offline manifest version
  `33dc0f0957a83a2fff8c961b98e16024e7e0c9c18cbfb49f4668d34e3d6ffbf6`.
  Packaged and served WASM hashes match.
- Preserved GUI3: `artifacts/gui3`, source commit
  `3a7b243b9bceef6f6009c4039af29a77bb61a4cc`; native SHA-256
  `36db158c8bbc94a21b40d17b85de73c0cdf05d80ba78a9ff70ef7bbf39f3fe95`,
  browser manifest `bba188968c7cd8850a140caf2b3ff5dd975bde79dad8e02d8dec5c4b0b19d095`.

## Boundaries

This slice supports zero or more SVG sources and one Flat V-carve operation.
The existing 8 MB input/job and 100,000-motion GUI limits remain. Batch chooser
input is capped at 128 files; multi-file drops direct users to that chooser.
Old saved-data compatibility is not required; explicit migration uses the
existing supported core route. Library management, multiple operations/Face and
old-GUI cutover belong to later slices. Native window interaction, additional
platform/GPU qualification and physical machining remain separate review work.
