# Operation-owned geometry selection

Status: implemented and verified in software; user review pending.

## Problem

Geometry selection had two owners. The drag-knife operation selected its own
centerline chains under **Cutting → Geometry to cut**, with checkboxes, Select
all/Clear and viewport click/Shift-click, and every change was a document
command bound to the current source revisions. The Flat V-carve operation was
different: its filled components were assigned from the **Artwork** panel
(**Carving assignment · filled components**, plus **Use picked**, **Add
picked** and **Remove picked** driven by a display-only viewport pick).

That split made the artwork look like the owner of machining intent, gave two
operations of the same workspace different mental models, and put selection
state, undo grouping and issue routing in the wrong panel.

## Target

Every operation owns a selection of geometry for itself, exactly as the knife
already did, and the artwork panels never assign geometry.

## Implementation

- **Flat V-carve geometry lives in the operation.** `Cutting → Shape & depth`
  now opens with **Geometry to carve**: selected/available count, **Select all
  filled components**, **Clear component selection**, a per-source checkbox
  list of the current catalogue's filled components with their placed bounds,
  and the unresolved-reference rows. `Cutting → Endmill` and `Cutting → V-bit`
  keep exactly the fields they had. The drag-knife **Geometry to cut** group is
  unchanged at the top of its Cutting panel.
- **Artwork panels manage artwork only.** The Flat V-carve artwork panel keeps
  Add/Replace/Delete/Duplicate/Reorder, Hide/Lock, placement numbers, the
  import-rejection messages and an **Open operation geometry** link. The knife
  artwork panel keeps placement, Replace and Delete plus the same link; its
  duplicate chain checkbox list is removed.
- **Viewport assignment is a document command.** In **Select artwork** mode a
  click assigns that filled region to the operation, Shift-click adds or
  removes one member, and **Next overlap** advances from whichever coincident
  owner is assigned now and assigns the next one. Hidden or locked sources stay
  unassignable, and a click that hits no selectable region changes nothing.
  (The knife already worked this way; both operations now share the pick
  helper.)
- **No silent rebinding.** `ArtworkCommand::CarveSelection { references }`
  routes to the core's `set_component_selection`, but the GUI adapter first
  requires every reference to exist in the displayed catalogue (owner, kind,
  local ID and the current source revision). A reference left over from a
  replaced source fails the whole command instead of being rebound, matching
  the knife's existing refusal. Viewport assignment drops stale members of the
  previous selection for the same reason.
- **Explicit repair stays explicit.** Unresolved references are listed inside
  the operation's geometry group. **Replace reference N with…** offers the
  current components of that source (or every current component when the source
  is gone) and calls the core's `replace_component_reference`; **Remove
  unresolved reference N** deletes just that reference. Nothing infers a repair
  from filenames, names or nearest shapes.
- **Undo and issues follow the operation.** Each checkbox, Select all, Clear,
  click, Shift-click, Next overlap or repair is one artwork-command transaction,
  so Undo restores the previous selection. `components[N]` issues now open
  Cutting → Geometry to carve → **Unresolved selections**; `components` /
  `component_ids` open **Select all filled components**; knife `chains[N]` opens
  **Unresolved knife selections** and `chains` / `artwork` opens **Select all
  knife chains**. The operation header switches to the shape tab for these
  destinations.

The portable job contract is unchanged: the selection is still the operation's
own qualified `components` / `chains` list, so saved jobs, migrations and the
retained service see exactly the same document.

## Behaviour notes

- Pointer assignment is a command, not display state, so artwork interaction is
  disabled while a command is in flight (the same rule the knife already had).
  A click arriving in that window is ignored rather than queued.
- Clicking a filled region does not change the active artwork row; the row
  still selects the source that numeric placement and placement gestures edit.
- **Geometry to carve** is rendered on the operation's shape tab, so the tool
  tabs keep their previous length, and a selected component is visible in the
  viewport as scene geometry (cyan selected, gray excluded) rather than as a
  separate orange pick outline.

## Verification (2026-09-12)

Rust, from `flat-v-carve`; log `artifacts/gui/operation-geometry-tests.txt`:

| Command | Result |
| --- | --- |
| `cargo test -p cam-gui --locked --lib --tests` | 96 passed, 0 failed (57 lib, 8 authoring, 7 collection, 5 knife, 4 native worker, 1 operation lifecycle, 10 resources, 4 workflow) |
| `cargo clippy -p cam-gui --all-targets --locked -- -D warnings` | clean |
| `cargo fmt --all -- --check` | clean |

New Rust coverage:

- `tests/collection.rs::operation_owns_its_geometry_selection_and_binds_current_revisions`
  selects, clears, refuses a stale reference without touching the document and
  re-selects the current component of the same owner.
- `app::inspector::operation_ui::tests::operation_owns_the_component_selection_and_artwork_does_not`
  renders the geometry group on the operation's shape tab and asserts the
  artwork panel and the tool tabs no longer carry it.
- `app::inspector::operation_ui::tests::geometry_issue_focus_reveals_the_shape_tab`
  and `app::inspector::issues::tests::selection_issues_route_to_the_operation_geometry_group`
  pin the issue destinations to the operation's geometry group.
- `app::inspector::knife_ui::tests::knife_geometry_is_selected_in_the_operation_not_in_the_artwork_panel`
  asserts the knife artwork panel lost its chain list while **Geometry to cut**
  keeps it.

Browser tours, real local Chromium/WebGPU with the WASM worker, packaged build
`524efafe5d4d`, each with zero recorded console errors. Directories under
`artifacts/gui/browser-smoke`:

| Tour | Directory | Result |
| --- | --- | --- |
| GUI3 lettering (`--gui3`) | `2026-09-12T15-30-47.225Z` | click/Shift-click assignment, holes, overlap owners, placement, issues, export, reopen |
| GUI4 collection (`--gui4`) | `2026-09-12T15-32-57.039Z` | cross-source assignment, replace/reopen, explicit repair menu, delete/Undo, hide/lock, reorder, batch import |
| GUI5 resources (`--gui5`) | `2026-09-12T15-28-37.065Z` | library/profiles, assignment picker, machine edits, exact output |
| GUI6 knife (`--gui6`) | `2026-09-12T15-35-49.524Z` | knife geometry in the operation, start, playback, library, real flower source |
| Knife authoring | `2026-09-12T15-37-12.965Z` | viewport chain assignment, Undo, derived outlines |
| Flat V-carve authoring | `2026-09-12T15-34-04.770Z` | new job selects components in the operation before generation |
| Canonical flower milling | `2026-09-12T15-38-02.527Z` | unchanged default carving, recovery, checked bytes |
| Operation lifecycle / add-replace | `2026-09-12T15-39-13.238Z`, `2026-09-12T15-40-51.714Z` | delete/add/Undo/Redo and operation-tab flows |

The review recipes were updated to the new flow:
[gui3-review.md](gui3-review.md), [gui4-review.md](gui4-review.md) and
[gui6-review.md](gui6-review.md).

## Review artifacts

- Native: `flat-v-carve/artifacts/gui/native/cam-gui.exe`, SHA-256
  `9ee958578c68ea1bfb940bbf634282de3ed68d5e04fad861a481f966c0603bf1`
  (recorded in its `SHA256SUMS`).
- Browser: `flat-v-carve/artifacts/gui/browser`, offline build
  `524efafe5d4d`. Serve it with `node crates/cam-gui/web/serve.mjs` from
  `flat-v-carve` and open `http://127.0.0.1:5182/web/index.html`.

## Limits

The workspace still supports one operation at a time (mixed milling/knife
sequences remain GUI7d); this slice makes the selection model per-operation so
that later slices can add more of them without reworking the artwork panels.
Selection is replaced as a whole list with no marquee or range picking, and
physical machining acceptance remains a separate user activity.
