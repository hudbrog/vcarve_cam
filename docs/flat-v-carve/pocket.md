# Pocket operation

Pocket clears closed, flat-bottom regions with one flat endmill. Select several
filled components to cut them at the same top and bottom in one operation.
Holes in the selected fill remain islands. Overlapping selections are unioned;
separate artwork drawn inside a fill is not automatically an island.

## Set up a pocket

1. Import artwork, choose **Add operation → Pocket**, and select filled regions
   in **Geometry & depth** or in the viewport.
2. Set top and bottom references. A negative bottom offset from **Operation top**
   is the pocket depth. A preceding Face result may establish the top if it
   covers all selected pockets. Different depths require separate operations.
3. In **Cutting**, select a flat endmill and enter cutting length, capability,
   cutting feed, spindle speed/direction, stepdown, stepover, and climb or
   conventional cutting. Tool-library geometry and copied cutting profiles use
   the same assignment workflow as Face/Profile.
4. Set nonnegative wall allowance. Enable **Finish walls with the same tool**
   to remove it with wall passes at the same bounded depth layers. An unset
   finish feed uses the cutting feed. There is no floor allowance or separate
   floor-finishing pass.
5. In **Entry & leads**, choose plunge, linear ramp, or helix. Optional tangent
   line/arc leads have explicit dimensions and feeds. Generate, inspect paths
   and stock, then prepare checked output with the applied machine configuration.

New operations remain saveable with missing values. Partial numeric input is
kept as an editor draft with undo/recovery; it cannot authorize fresh output.

## Entries and leads

Plunge requires declared plunge capability and plunge feed. Ramp and helix
require declared ramp capability, maximum entry angle, and entry feed. The
linear ramp travels out and back along a contained segment. Helix placement is
automatic for each cutting run; radius means the **cutter-center path radius**.
Its swept diameter is tool diameter plus twice this radius.

The helix radius must be smaller than the cutter radius with a numerical margin
so entry into solid stock leaves no central plug. Pitch is bounded by both the
angle and stepdown. Half-turn G2/G3 segments carry changing Z; the planner reduces
pitch to reach the requested layer exactly in whole turns, then makes a level
revolution before leaving the entry. No fractional final turn is needed.

Leads are tangent at the loop seam. Their entire cutter sweep must stay inside
the pocket, including around islands. Requested leads are never silently
shortened, dropped, or replaced; a lead or helix that cannot fit blocks the
whole operation with a pocket/layer diagnostic. Clearance retracts separate
runs and disconnected pockets. Each pocket's roughing and finishing complete
before the next pocket begins.

Offset clearing proceeds from the inside outward. With a clockwise spindle,
climb passes go counterclockwise around outer pocket walls and clockwise
around islands; conventional cutting reverses these directions. See Autodesk's
[pocket-clearing explanation](https://www.autodesk.com/products/fusion-360/blog/10-2d-cnc-milling-toolpaths/).

## Checks and current bounds

The constant-section planner lives in `cam-core/src/operations/pocket/` and
does not use a V-bit or the tapered V-carve target. It emits ordinary milling
stages, `pocket_rough` and `pocket_finish`. The schema remains version 5 with an
additive `pocket` settings variant; earlier readers without the variant reject
it rather than interpreting it as another operation.

- All selected regions must lie within stock XY, and the bottom must remain
  within stock thickness. A top below the established surface is refused.
- Stepover must exceed four geometry tolerances and be at most cutter radius.
  Cutting length must accommodate depth from the original stock top.
- Offsets carry a numerical clearance reserve. Continuous cutter containment
  includes lines, arcs, helical entries and leads. Coverage is reconstructed
  conservatively from recorded motions at every depth layer, and checked again
  before export; display simulation is not the verifier.
- Unavoidable internal-corner material is reported as
  `POCKET_UNREACHABLE_RESIDUAL`; stock simulation shows the actual residual.
  Intentional radial allowance has its own diagnostic. Unintended gaps in
  reachable material block completion with `POCKET_COVERAGE`.
- Offset topology can leave a gap that this initial planner cannot clear.
  Such work is returned incomplete, without partial cutting output. A wholly
  inaccessible selected region cannot be skipped.
- Planning limits are at most 256 layers, 1,024 loops per layer and 100,000
  motions; geometry/verification work has additional bounds. Exhaustion blocks
  checked output. Large or difficult geometry may need a separate operation,
  a different tool, or future cleanup/routing improvements.
- Optional fitted clearing arcs are disabled for Pocket. Native helix and
  lead arcs are retained and pass the existing numeric G-code readback.

Open-sided pockets, breakthrough below stock, multiple cutters, rest machining,
adaptive clearing, optimized stay-down links, and floor finishing are outside
this version. Pocket stock history records removal but publishes no reusable
top plane.

## Reproducible evidence

Open `flat-v-carve/fixtures/pocket/two-pockets.job.json` for an island and a
second pocket, shared depth, helical entry, arc lead-in, line lead-out and wall
finishing. Its numbers are demonstration inputs; no machine profile is applied.

Core tests are `pocket_document.rs` and `pocket_core.rs`; retained output is
covered by `pocket_prefix_prepares_retained_helices_and_rejects_stale_or_incomplete_work`.
GUI tests cover rendering, qualified selection, save/reopen, partial input,
undo/recovery, helical Z interpolation and stock removal without island damage.
Run `node crates/cam-gui/web/smoke.mjs --pocket` against a built browser served
on localhost:5182 to exercise editing, generation, stock replay, checked G-code
download and reopening the saved job.

On 2026-09-21, Chrome 152 / NVIDIA Ampere completed that workflow with no browser
errors: 2,434 motions, 8.439 seconds for generation including checks and display
preparation, using the scenario's 1.3 mm depth and 0.8 mm helix radius. This is a
local measurement, not a hardware-independent performance guarantee.

Release-mode core measurements on the same workstation:

| Workload | Motions / stages | Planning | Planning + independent checks |
| --- | --- | --- | --- |
| Full demonstration, 2.3 mm depth, helix/leads/finish | 3,651 / 4 | 6.865 s | 8.684 s |
| `real_data/flower_box.svg`, 2.5 mm endmill, 1 mm depth, plunge, 1.25 mm stepover | 56,495 / 15 | 13.662 s | 22.239 s |

Both completed with passing checks and explicit unreachable-corner diagnostics.
The flower workload is flat-bottom clearing, so its times are not directly
comparable to tapered V-carve finishing. Existing V-carve regression tests remain
part of the workspace gate. Reproduce the measurements with
`cargo run -p cam-core --release --example pocket_fixture -- --measure` and
`cargo run -p cam-core --release --example pocket_fixture -- --measure-svg ../real_data/flower_box.svg`.

The command-line `collection plan`, `collection apply-machine`, and
`collection export --through pocket --layout one` paths also passed with the
full demonstration: 3,651 motions, four stages, one checked program. The
example machine's requested three decimal places were escalated to four by
the existing output-precision policy, with `EXPORT_PRECISION_ESCALATED` reported.
The output still passed numeric readback; the applied machine precision remains
visible for review.
