# GUI6 drag-knife review

GUI6 adds a standalone passive XYZ knife workflow. Technical results and exact
build identities are in [the progress report](gui6-progress.md). User acceptance
and physical cutting trials remain pending.

## Launch and starting files

Run `flat-v-carve/artifacts/gui6/review/knife-authoring-native/cam-gui.exe`. For the browser,
from `flat-v-carve` run `node crates/cam-gui/web/serve.mjs` and open
`http://127.0.0.1:5182/web/index.html` in desktop Chrome with WebGPU.
The frozen browser package is under `artifacts/gui6/review/browser`.
The previous GUI5 packages remain under `artifacts/gui5-before-gui6`.

Open `flat-v-carve/fixtures/gui6/knife.job.json` for the short review. It contains
one open corner chain and one closed square, one real knife tool, and an explicit
review machine snapshot. There is no dummy milling tool. These fixture settings
are test inputs, not material or machine recommendations:

| Setting | Value |
| --- | --- |
| Stock / clearance | 40 × 30 × 2 mm / 5 mm |
| Blade offset / maximum cutting depth | 1 / 2 mm |
| Cutting / plunge / swivel feed | 150 / 50 / 75 mm/min |
| Tool maximum / operation stepdown | 1 / 1 mm |
| Bottom / swivel depth / corner threshold | −1 mm from stock top / 0.5 mm / 20° |
| Initial heading | 180°, pivot toward tip, counterclockwise from +X |
| Motion / verification tolerance | 0.01 / 0.05 mm |

## Five-to-ten-minute review

1. In Cutting, inspect the open and closed chain checkboxes under **Geometry to
   cut**. Clear and select them explicitly, or click and Shift-click chains in
   the viewport. Artwork keeps placement, hide/lock and source management only.
   Use Move, Rotate or Scale, then Undo. Importing another SVG adds a source
   without assigning its geometry to the cut.
2. Inspect Cutting and the knife tool. The process is passive XYZ, spindle and
   coolant off. Edit depth, feeds, swivel depth, corner threshold and initial
   heading. Partial text such as `-` must stay visible and block Generate until
   completed or undone. Negative height offsets point downward.
3. In **Knife start**, choose a selected closed chain at 0%, 25%, 50% or 75% of
   source length, or an open chain's source endpoint. Inspect the named anchor,
   then return to **Automatic knife start**. Set **Closure overlap** explicitly
   when wanted. Core validates entry, depth, compensation and closure.
4. Generate the unmodified fixture: expect 96 motions. Simulate, Start, Next
   corner / entry and After knife. Stock removal stays 0.00 mm³. Read actual Z,
   purpose, pass/layer and modeled heading. Orange is pivot, cyan is planned tip,
   white is modeled blade. The colored inspection overlay is projected above
   the intact stock so the below-surface cut remains visible.
5. Export prepares checked bytes. Return to Inspect result: green is replay
   from those actual bytes, with deviation, heading error, precision, work-zero
   offset and SHA-256. Save the file. Change output precision or work zero: the
   old emitted replay must become stale, and new preparation must bind the new
   output settings without replanning unchanged cutting geometry.
6. In Tool library, create a drag knife with explicit dimensions and a knife
   cutting profile. Save the library, then Use tool & profile. Edit its copied
   feed in the job and Reset knife overrides. Reset uses the portable copied
   baseline. Saving a library does not silently update existing jobs.
7. Save job, reopen it, Generate and export again. Source bytes, qualified
   references, knife dimensions, assignments and applied machine travel in the
   job; retained plans and emitted evidence do not acquire trust through reopen.

To replace an existing V-carve, use **Delete operation** in the left Operations
list, then **+ Add operation → Add drag knife**. Deleting retains artwork,
placement, stock, tools and the applied machine; Undo restores the operation.
The empty job can be saved and reopened. Adding a knife leaves its selection,
dimensions and cutting values unset; existing filled artwork is not converted
to strokes. In Cutting, **Geometry to cut** lists individual paths grouped by
source. Select checkboxes, or use **Select knife paths** in the viewport:
click assigns one path and Shift-click adds/removes a path. Undo restores the
selection. **Import knife geometry** adds another SVG.

For filled artwork, choose **Create knife outlines → Outlines of [source]**.
This creates a separate stroked SVG from core-imported outer/hole boundaries,
including the source placement and page size. Curves use the existing import
tolerance. The original source remains intact, and selection does not expand
automatically. Choose the new outlines individually in Geometry to cut.

Once the knife is defined, **Suggested operation values** displays the values
before **Use suggested operation values** fills blank fields. Pass stepdown
uses the smaller of maximum blade cutting depth and the assigned profile's
stepdown limit. With no profile limit, the knife capacity is only a starting
limit to review for the material. Swivel depth suggests 10% of effective pass
depth; corner threshold suggests 20°. These two are editable starting values,
not deductions about blade/material behavior. Existing values and partial
input are preserved, and application is undoable. Feeds, desired cut depth and
physical initial heading still need a profile or explicit input.

For a blank start, use **+ Add operation → Add drag knife** before importing
artwork, or File → **New drag knife from SVG** and `chains.svg`.
Select chains and supply the values above, including stock, tool and machine.
New knife imports intentionally leave cutting values, geometry and heading unset.

## Real-source review

Open `fixtures/gui6/flower-knife.job.json`. It uses one closed chain,
`path1-chain-7`, from the user's flower artwork and generates 813 motions with
the same explicit cutting settings. The stock XY page is 200 × 100 mm.

The original `real_data/flower_box.svg` is filled artwork. Core imports fills
as regions and does not silently reinterpret them as knife cuts. The checked-in
`flower-centerlines.svg` is an explicit review derivative: only the path style
changes from black fill to `fill:none;stroke:#000000;stroke-width:0.1`; every
path coordinate and the page dimensions remain intact. The fixture script
records that conversion. The original source is unchanged. Stroke width is
ignored for centerline cutting. Filled-only SVGs receive an actionable message.

## Current limits

The workspace supports zero or one operation over SVG sources. Delete the
current operation before adding another. Mixed milling/knife sequences
belong to GUI7d. Source starts use the core anchor model; the menu offers the
listed seam positions, while other valid imported anchor fractions are preserved.
Near reversals and unsupported passive alignment/contact remain core errors.
The initial physical blade heading is an explicit assumption; software replay
does not establish physical blade tracking.

The full pivot execution is paged. The tip overlay shows the latest 2,048
motions and emitted display evidence holds at most 2,048 samples. Missing
sample intervals are not interpolated or colored as checked. Aggregate replay
checks still inspect the full output. The existing 100,000-motion, 8 MB source/
portable-output limits and a 16 MB knife-detail budget fail explicitly.
