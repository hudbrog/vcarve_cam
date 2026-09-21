# Job and operation contracts

## One portable document

The additive schema-5 [Pocket operation](pocket.md) stores filled-component
references, one milling assignment, shared top/bottom heights, entry/leads and
optional wall finishing. It has no dependency on a V-bit assignment.

`cam-core/src/project/v5/mod.rs` defines `CamJobV5` (schema 5): setup, embedded
artwork items, job tools, ordered operations, tolerances and one applied machine
configuration. Imports create artwork and page-sized stock without inventing an
operation. Older or future document schemas are refused without conversion.

Three gates have distinct purposes:

| Gate | Contract |
| --- | --- |
| Structural validation | Supported shapes, finite values, unique valid IDs and bounded content; required to save/open. |
| Reference inspection | Reports missing tools/artwork, changed source revisions, invalid face dependencies and unattached anchors. These issues remain saveable. |
| Planning readiness | Requires machining inputs and resolved references only for the requested enabled operation scope. |

Source files and external libraries are not needed to reopen a saved job. Plans,
meshes, caches and recovery drafts are not interchangeable with the document.
The document byte bound is 64 MB; the per-SVG bound and other limits live beside
their admission code and must not be bypassed by callers.

## Artwork and geometry identity

Each artwork item owns embedded bytes, import settings, placement and a stable
ID. The importer publishes the readings supported by the drawing: filled
regions, contour/centerline geometry and drill markers. The operation selects
the reading it needs; there is no import-wide milling/knife interpretation mode.

Geometry references include the artwork owner, kind, local geometry ID and
source revision. Wire IDs use `item:kind:local`; local IDs may themselves contain
colons. Duplicate names or identical SVG bytes do not merge ownership. Replacing
artwork cannot silently rebind a selection to a geometrically nearby object.
Typed document commands explicitly repair references and participate in Undo.

Flat V-carve selects filled regions, Profile selects contours, Drag knife
selects chains and Drill selects points. Face can be source-free. The Artwork
panel manages sources/placement; operation panels own machining selections.
Viewport click/Shift-click applies operation-owned commands. Hidden/locked
artwork is not assignable through picking. Manual contour anchors retain their
source binding and must be reattached when that binding becomes invalid.

Stock resizing uses an explicit anchor. Stock and artwork are not moved by
changing a cutter, facing margin or display setting. Geometry outside the stock
is reported, not silently repositioned. Coordinate transformations are defined
in [technical design](technical-design.md#1-coordinate-and-tolerance-conventions).

## Tools, assignments and heights

A job tool owns cutter geometry/capabilities. An operation assignment owns its
cutting values and copied profile baseline. Multiple assignments may share one
job tool without sharing every cutting setting. Library provenance is metadata;
later library edits cannot change a saved job. Machine T/H mappings belong to
job-tool IDs, independently of operation ordering. See [tool library](tool-library.md).

Heights can reference stock or an earlier facing result. A face-result
reference names its producing operation and becomes unresolved when that
operation is removed, disabled or moved after its consumer. Ordered stock
history supplies established planes and prior removal; a preview image never
establishes a usable surface.

## Implemented operations

| Operation | Behavior | Principal code/tests under `flat-v-carve/crates` |
| --- | --- | --- |
| Face | Whole-stock or rectangular coverage, margins, 0/90-degree raster, layered passes and explicit entry choice. | `cam-core/src/operations/face.rs`, `cam-core/tests/face_core.rs`, `face_workflow.rs` |
| Flat V-carve | Endmill-only or combined roughing/rest/finish, finite-tip target, bounded links and optional arc fitting. | `cam-core/src/operations/flat_vcarve.rs`, `cam-core/src/pocket/`, `cam-core/src/vcarve/`, `cam-core/tests/rest_planning.rs`, `arc_fit.rs` |
| Profile | Contour side, depth passes, tabs, radial finishing, starts and leads/ramps. | `cam-core/src/operations/profile/`, `cam-core/tests/profile_basic.rs`, `profile_tabs.rs`, `profile_finish.rs`, `profile_entries.rs` |
| Drag knife | Tip paths compensated to pivot motion, explicit initial heading, corner swivels, bounded simplification and independent replay. | `cam-core/src/operations/drag_knife/`, `cam-core/tests/knife_geometry.rs`, `knife_output.rs`, `knife_simplification.rs` |
| Drill | SVG marker points, selected or X/Y order, single plunge or peck cycles, dwell and tip/full-diameter depth references. Output expands to G0/G1/G4. | `cam-core/src/operations/drill.rs`, `cam-core/tests/drill_points.rs`, `drill_core.rs` |

Facing margins define coverage; entry/exit travel extends the cutter's travel
beyond that coverage. Entry is resolved once from min/max/explicit/alternating
mode. Every layer transition includes its actual retract/travel/descent, and
`PLAN_MOTION_DISCONTINUITY` rejects a stage that skips a position. Unsafe entry
reports the required clearance; it does not assume a cutter can plunge.

Knife programmed XY is the holder pivot, while the intended cut is the blade
tip. For a straight tip path `p` with tangent `t` and blade offset `d`, the pivot
is `p + d*t`. Corner/alignment swivels can use native arcs. Replay checks the
resulting tip trace; knife blending is bounded by available replay tolerance.
It is not a stock-removal milling operation or an actively steered knife axis.

Drill markers preserve analytic centers where available and use area-derived
centroids for other supported closed shapes. Full-diameter depth includes the
drill tip extension, so through-cut allowance and stock-bottom checks matter.
Canned cycles and tapping/reaming/boring are not implemented.

## Retained execution and output

The enabled operation list is execution order. A prefix request ends at a named
operation; later enabled operations do not widen that scope at export. Stock
history and tool stages follow this same order. Reusing a tool later does not
permit regrouping cuts across intervening operations.

`cam-core/src/sequence.rs` and `cam-service/src/retained.rs` bind execution to machining
identity and retain generated plans. Output preparation checks freshness and
binds the current applied machine configuration and work zero. Display-only and
provenance edits can reuse matching execution; changed machining intent cannot.
Prepared bundles retain exact bytes, ordered manifests and reports so a failed
save can be retried without silently generating a different program.

Basic checks, generation completeness, detailed stock-quality analysis and
display simulation are distinct outcomes. The ordered export path uses basic
checks and strict numeric readback; detailed M5 quality analysis stays separate.
See [LinuxCNC output](linuxcnc.md) for the machine boundary.
