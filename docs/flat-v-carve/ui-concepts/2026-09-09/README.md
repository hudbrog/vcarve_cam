# 2.5D CAM — UI concept gallery

Synthetic screenshots for a new egui-based web/desktop workspace, aimed at experienced CNC users. These were generated from the [UI proposal](../../2.5d-cam-ui-plan.md) using built-in image generation. The existing project UI was not supplied as a reference. Both [original prompts](prompts.md) and [revision 2 prompts](revision-2/prompts.md) are included.

The proposed flow is **Prepare → Generate → Simulate → Export**, with free movement between editing and inspection. Machine editing lives under Setup; Export reviews the applied settings and links back to their editor. The core layout stays stable: job and operation order on the left, work area in the center, contextual controls on the right.

These images communicate layout and interaction context. They are not egui application captures, measured machining geometry, or cutting presets. The written proposal governs exact labels and behavior.

## Revision 2 — Independent artwork sources

Machine and the global Tool library are under Setup. Artwork is a list of independently placed SVG sources, while Job tools remains anchored at the bottom.

![Multiple artwork files and revised navigator](revision-2/01-artwork-sources.png)

## Revision 2 — Global tool library

One tool geometry owns multiple named cutting profiles. Copy geometry into a job or apply a selected profile to an operation explicitly.

![One tool geometry with several cutting profiles](revision-2/02-tool-library.png)

## Revision 2 — Job tools

The selected job cutter has several operation assignments, each with its own applied profile or custom cutting values. Controller mappings link back to Setup → Machine.

![Job tools and operation cutting assignments](revision-2/03-job-tools.png)

The five original images below remain references for the broader workflow. Their older navigator and editable machine settings in Export are superseded by revision 2. Future sketch items would join Artwork, but sketch creation is not part of the initial release.

## 01 — Setup

Physical stock, artwork placement, and an independently selected work zero.

![Physical stock and work zero](01-setup.png)

## 02 — Profile and tabs

Compact operation settings, direct tab placement, and an explicit stale-plan state.

![Profile editing and tab placement](02-profile-tabs.png)

## 03 — Simulation

Ordered stage playback, recurring tools, remaining tab material, and separate plan/output status.

![Simulation and stage timeline](03-simulation.png)

## 04 — Export

Sequential files preserve operation order. This illustration shows prepared output; the initial layout default would be one ordered program.

![Ordered output review](04-output.png)

## 05 — Passive knife

The same shell explains holder versus tip motion, contact alignment, and swivels on a separate sheet-stock job.

![Passive knife motion inspection](05-knife.png)

For this revision's review, focus on independent source placement, the scope difference between global and job tool edits, and how applying or overriding a cutting profile affects an individual operation.
