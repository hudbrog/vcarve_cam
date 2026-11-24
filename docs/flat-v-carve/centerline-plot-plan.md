# Centerline Plot

Operation for a passive pen raised and lowered by the machine's Z axis. The user selects lines in SVG or sketch artwork and the pen traces each selected chain once. For example, selecting an open L and a closed square produces two strokes. With the default settings, the pen lifts after the L, travels to the square, traces all four sides and lifts at the starting corner.

## What the operator controls

| Control | First-version behavior |
| --- | --- |
| Geometry | Pick one or more existing centerline chains. Use their placed XY coordinates, including artwork rotation and translation. A thick SVG stroke yields one trace along its path. For outlined lettering, picking both boundaries draws both outlines. Recovering a handwriting stroke from a filled shape would be a separate feature. |
| Stroke order | Follow the stored selection order and the catalogue's vertex order. Show the order in the editor and allow moving a stroke up or down. Preserve the endpoints of open strokes and the start of each closed stroke. Continue drawing when the current end and next start match exactly in setup coordinates. Lift across every gap. Interior crossings preserve the stroke order. |
| Tool | Assign a passive Pen job tool. Use the existing job-tool identity and machine tool mapping so a plot can run alone or between machining operations. |
| Pen down | Use a flat drawing surface, initially stock top. Default to contact at that surface. An optional nonnegative compression distance commands additional downward Z travel for a calibrated spring-loaded holder. The operator sets this in millimeters. |
| Pen up | Use the existing setup clearance above stock for every travel between strokes and for entry and exit. This gives the job and machine profile one clearance value to agree on. |
| Feeds | Set XY drawing feed and Z lowering feed independently. Lift vertically to clearance, then use ordinary rapid travel. |

## Motion and output

For each chain, reach clearance before moving XY to its start, lower vertically at the Z feed, then follow the chain at constant contact Z and drawing feed. With **Closure gap** at zero, draw the whole loop. A positive value leaves that many millimeters undrawn at the end of each closed stroke, measured along the placed centerline. For a fineliner, try 0.2 mm. The gap must be shorter than the loop. If the next chain starts at the same XY, continue drawing at contact Z. Otherwise retract vertically before crossing the gap. The plot stage ends at clearance.

Use the catalogue's tolerance-bounded polylines for the first release. Keep spindle and coolant off throughout the plot stage and request exact-path control for draw and lift transitions. The postprocessor applies the existing work-zero and tool-length contracts. Store drawing moves for preview and verification. A plot contributes zero removed material and establishes no machined surface for later operations.

## Editor and export

Choose **Operations → Add Centerline Plot**. In **Paths**, check individual paths, choose **Select all** to include every path, or choose **Select all in** a named layer to select just its paths. Bulk selection replaces the drawing order with the matching paths in list order. Use the up and down arrows to reorder them. In **Pen**, choose the job pen and enter drawing and lowering feeds in mm/min. A new operation creates a passive Pen tool. Its feeds belong to the operation. Changing the pen clears its feeds and compression so the replacement holder can be calibrated separately.

The plot picker skips paths with no drawable length. If a saved job already includes them, choose **Paths → Remove undrawable paths** to remove them together and preserve the drawing order of the remaining paths.

Map the pen's job-tool ID in the applied machine configuration, then generate the job or a prefix ending at the plot. The preview shows ink on the drawing surface and clearance travel in a different color. Playback retains the commanded Z, including spring compression. Export supports one LinuxCNC program or sequential files. The emitted bytes are checked against the retained motions, including the lifts, working heights and passive process state.

## Verification

The [planner and export tests](../../flat-v-carve/crates/cam-core/tests/centerline_plot.rs) exercise ordered paths, closed seams, small gaps, interior crossings, artwork placement, sketch input, stale references and a milling → plot → milling sequence. The [editor test](../../flat-v-carve/crates/cam-gui/tests/centerline_plot.rs) follows a plot from selection and numeric edits into the generated preview.

For a physical check, plot the [open L and closed square](../../flat-v-carve/fixtures/gui6/chains.svg) on paper with a calibrated contact height. Look for a continuous corner, a closed square and a clean transfer between them.
