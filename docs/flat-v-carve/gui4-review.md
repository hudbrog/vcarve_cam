# GUI4 portable artwork collection review

GUI4 lives in the production `cam-gui` crate. Run
`flat-v-carve/artifacts/gui/native/cam-gui.exe` for Windows, or run
`node crates/cam-gui/web/serve.mjs` from `flat-v-carve` and open
`http://127.0.0.1:5182/web/index.html` in Chrome. The packaged browser build is
in `flat-v-carve/artifacts/gui/browser`. GUI3 is preserved in `artifacts/gui3`.
See [the progress report](gui4-progress.md) for target readiness and evidence.

## Portable project

Open `flat-v-carve/fixtures/gui4/lettering.job.json`. This contains the embedded
SVG, its placement, cutting assignments and the applied machine snapshot,
including explicit T3/T8 mappings. It needs no separate SVG or profile on reopen.
The fixture starts in Endmill-only mode and retains its finishing values.

For an existing earlier-format job, use **File → Import older job**. This is an
explicit forward import through the core migration contract. Apply the desired
machine profile in **Machine**, then **Save job** and reopen that one file.
The original file is not overwritten by import. Unsupported newer schemas and
operations are rejected while the current project remains intact.

**New from SVG** starts another project. **Add SVG** and **+ Import artwork**
add to the open project and support selecting several SVG files. A single SVG
dropped into an open project also adds artwork; dropping a job opens that job.
Multi-file drops are explicitly refused; use the batch file chooser instead.
Accepted batch additions form one Undo transaction. File-specific failures are
shown in the artwork inspector and do not discard accepted additions.

## Independent sources and machining assignment

Add `fixtures/gui4/second.svg`. Select its row in the navigator and set Origin X
to -25, Origin Y to -3, Rotation to 0 and Scale to 1. The first source keeps its
origin (-5,-3). Each source's numeric placement and viewport gesture target use
its stable item ID. Partial text remains attached to its source across row
changes/reordering/recovery and blocks generation while unresolved.

Move the second L, Undo and repeat numerically. Rotate and Scale use setup
(0,0), the image of that source's page origin. Each released gesture is one Undo
transaction; Escape cancels, and a changed document/active source cancels a
stale gesture. Switching rows and ordinary picking do not change machining.

With **Select artwork**, pick near setup (9,23) and Shift-pick (29,20). The
second point lies inside the first O's hole and on the second L, so each pick
has a different owner. At genuinely coincident fills, **Next overlap** cycles
the candidates. Verify `artwork-1 / letter-l::0` and `second / letter-l::0` in
the assignment list, then choose **Use picked**. **Add picked** and **Remove
picked** make explicit incremental changes. Importing or duplicating never
expands the assignment automatically.

Choose **Combined** in Cutting, Generate, Simulate, then **After endmill** and
**After V-bit**. Both Ls must be carved. Inspection still uses actual sampled
stock and exact stage/playhead ranges. Prepare checked output, save its exact
bytes, save the project and reopen it. Both sources, their placements,
assignments, finishing values and the applied machine remain portable.

## Organization and currentness

**Hide artwork** hides source outlines/picking; it does not erase its machining
or stock removal. **Lock artwork** prevents viewport picks/gestures while
leaving explicit numeric edits available. These are local workspace properties,
stored in recovery rather than the portable machining job.

**Duplicate artwork** creates a distinct item and leaves assignments alone.
**Move row up/down** organize the collection; they do not reorder machining.
Adding/moving unassigned artwork, duplication and row ordering trigger a
worker-owned machining identity check. An unchanged retained carving can become
current again without replanning; changes to a used source remain stale until
Generate. Output preparation still validates the current machine/output
settings through the retained service. Reopening or restarting never restores
execution/output trust from a saved file.

## Replacement, deletion and explicit repair

Select the first source and **Replace SVG** with `fixtures/gui4/replacement.svg`.
It changes the L while deliberately reusing its local IDs. Its existing
assignment becomes unresolved because the source revision changed. The second
source remains assigned. The issue links to **Unresolved assignments**; save
and reopen this incomplete project to verify the reference is preserved.

Pick the intended new L and choose **Repair reference 1 with picked**. The
command replaces that exact old reference with the explicitly chosen current
component; other unresolved references stay untouched. Alternatively remove an
unresolved assignment explicitly, or Undo the replacement. Generate the repaired
project and inspect/export again.

Delete a used source. Its references remain visibly dangling, and the project
is still saveable. Undo restores the exact source/assignment. Adding a new file
with the same name cannot reuse an owner ID still named by dangling references.
Deleting the last source leaves an editable empty collection with Add/Undo and
reference repair available. Try `fixtures/gui4/broken.svg`: replacement fails
without changing the project. A late file/worker completion after another edit
cannot overwrite newer state.

## Verification

From `flat-v-carve`:

```text
cargo test -p cam-gui --release --locked
cargo test -p cam-core --release --locked --test artwork_commands --test collection_machining --test project_v5 --test collection_resources --test sequence_plan
cargo clippy -p cam-gui --all-targets --locked -- -D warnings
./scripts/build-gui.ps1
./scripts/build-gui.ps1 -Target web
node crates/cam-gui/web/serve.mjs
node crates/cam-gui/web/smoke.mjs --gui4 --port=9336
node crates/cam-gui/web/smoke.mjs --gui3 --port=9337
node crates/cam-gui/web/smoke.mjs --authoring --port=9338
node crates/cam-gui/web/smoke.mjs --port=9339
```

The browser tours use real pointer/keyboard, file chooser/drop and download
controls. The application probe is read-only. Review the timestamped reports,
screenshots and saved files in `artifacts/gui/browser-smoke` as well as exit
status. Physical machining and native window interaction are separate user
review activities; this slice does not claim new platform or large-job
qualification. The existing 8 MB job/input and 100,000-motion GUI limits remain.
