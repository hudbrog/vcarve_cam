# Flat V-carve CAM: technical design

Date: 2026-09-05\
Status: M0–M4 implemented and tested, including both planners and combined stock/quality previews. Adaptive continuous combined-stock verification and machine output remain M5–M6 work.

See [architecture](architecture.md) for scope and component boundaries, and [implementation plan](implementation-plan.md) for delivery order. Unless explicitly attributed to a source, the geometry below is derived for this project.

The [M3 capability report](m3-capability-report.md) defines the current endmill-only implementation: offset loops with a numerical guard, explicit plunge/ramp entries, clearance-plane links, independent continuous segment clearance, and actual-motion stock comparisons at stepdown slices. M3 uses `verification_tolerance_mm` as the XY floor-coverage tolerance at those slices. The adaptive volume/depth uncertainty and combined finish-quality contracts below remain M5 work; an M3 `complete` stage does not claim them.

The [M4 capability report](m4-capability-report.md) defines the current combined planner: guarded full-depth boundaries, threshold-split medial paths, floor lanes, conservative air proofs against actual endmill sweeps, bounded cleanup, and a retained final finishing family. M4 checks continuous linear-radius cutter clearance, floor coverage at the ridge depth minus an explicit numerical budget, and a configurable sample lattice with independent reachability bounds. Its sampled quality maxima and fixed slices are not the adaptive global certification specified for M5.

## 1. Coordinate and tolerance conventions

Use millimeters for all internal lengths, millimeters per minute for feeds, and RPM for spindle speed. The workpiece top is Z = 0. Internal depth `d` is nonnegative downward; its machine coordinate is `z = -d`. Transform SVG coordinates into a right-handed XY workpiece plane once during import, including the SVG Y-axis reversal. Tool positions always refer to the center of the physical tool tip plane, not a virtual cone apex.

Use named domain types for lengths, angles, tool IDs, operation IDs, and depths where this prevents mixing meanings. Reject non-finite numbers, invalid angles, nonpositive dimensions/feeds, and inconsistent cutting dimensions before planning.

Separate these controls:

| Control | Meaning |
| --- | --- |
| `geometry_tolerance_mm` | Budget for normalization, offsets, curve approximation, and geometric calculations. |
| `motion_tolerance_mm` | Budget for turning continuous candidate paths into linear XYZ moves. |
| `verification_tolerance_mm` | Requested uncertainty bound for stock comparison. |
| `max_floor_ridge_mm` | Permitted physical ridges above the target floor after V-bit clearing. |
| `max_detail_residual_mm` | Permitted depth residual in nominal detail proved unreachable by the selected cutters; distinct from missed reachable material. |
| `wall_allowance_mm` | Extra horizontal stock left by the endmill near the target wall. |
| Output precision | Decimal resolution of serialized machine coordinates. |

No machining defaults are confirmed yet. Numerical tolerances are not substitutes for finishing allowances. The verifier must aggregate approximation bounds; rounding and each processing stage cannot independently consume the entire allowed error.

Polygon adapters use a documented integer scale shared with segment Voronoi input. Choose it from the requested resolution and supported coordinate range. Check multiplication, overflow, and topology changes caused by snapping. Reject an impossible tolerance/range combination instead of silently degrading accuracy.

M0 implements this policy and the finite-parabola evaluation method. See the [capability report](m0-capability-report.md) for the common integer range, budget allocation, measured fixture errors, rejection behavior, and limits of the current evidence.

Detail residual is zero by default. A nonzero setting is a visible finish-quality choice, with the affected regions highlighted. It must not excuse missing stock that a valid toolpath could remove.

## 2. Nominal target shape

Let:

- `P` be the selected planar removal region, with holes representing preserved islands.
- `D` be the maximum carve depth.
- `theta` be the V-bit included angle, and `alpha = theta / 2`.
- `rho(x)` be the shortest Euclidean distance from an interior point `x` to any boundary of `P`.

For an ideal sharp V-bit, define target removal depth:

```text
T(x) = min(D, rho(x) / tan(alpha))    for x inside P
T(x) = 0                            outside P
```

At depth `t`, the target cross-section is:

```text
P_t = erode(P, t * tan(alpha)),  0 <= t <= D
```

Here `erode` means inward offset by a Euclidean disk, including around islands. It is not an arbitrary mitered graphical offset. Circular parts of offsets must be approximated within the geometry budget. Check topology as regions split or vanish.

The nominal flat-floor region is `P_D`. In a straight channel of width `W`, its width is `max(0, W - 2*D*tan(alpha))`. If it has zero width, the ideal cut has no finite-area flat floor there. Ideal terminal points can approach the original surface as depth tends to zero.

This target describes the intended carving before cutter limitations. A finite tool tip can leave some ideal corners unreachable. Report the resulting residual against this target; do not silently round the artwork or redefine the target to hide the residual.

## 3. Cutter models and admissible positions

### 3.1 Flat endmill

Model a flat endmill as a cylindrical cutting region with radius `R_e` and a flat tip plane. Record usable cutting length and whether direct plunging is permitted.

At tip depth `d`, a valid center position must have its entire disk inside `P_d`. With horizontal wall allowance `a`, valid centers are:

```text
C_e(d, a) = erode(P, d*tan(alpha) + R_e + a)
```

This distinction matters: `P_d` is a region to remove, while `C_e` is where the tool center can move. An additional tool-radius compensation must not be applied twice.

At an exact fit, a valid center set may collapse to a line or point even when polygon offset output is empty. Use clearance-distance/medial-axis information to distinguish those cases from no access. An exact-fit cut still requires a supported entry and numerical margin sufficient for its claimed tolerance.

Accessible floor areas are finished to `D` by the endmill in the MVP. A separate axial floor allowance is deferred. The lateral allowance can still leave thin floor strips for V-bit cleanup.

### 3.2 V-bit, including a finite flat tip

Let `r_t` be the physical tip radius and `R_v` the maximum usable cutting radius. Above the tip plane, within the conical cutting section:

```text
radius(u) = r_t + u*tan(alpha)
usable_conical_height <= (R_v - r_t) / tan(alpha)
```

At center `q`, the deepest admissible tip position relative to the stock top is:

```text
d_max(q) = min(D, (rho(q) - r_t) / tan(alpha), usable_conical_height)
```

A nonpositive result provides no positive-depth cut. Validate the declared angle, radii, and cutting height against each other. Do not treat the shank as an extension of the cutting cone.

For the MVP, require `D <= usable_conical_height`. Finishing a deeper nominal wall with only part of a short cutting cone engaged would require additional shoulder/shank clearance logic and is deferred. Do not quietly clamp an incompatible requested job depth and call the result complete.

At full depth, the valid V-bit center region is:

```text
C_v(D) = erode(P, r_t + D*tan(alpha))
```

Thus `P_D` and `C_v(D)` coincide only for the ideal zero-radius tip. Finite-tip handling must be present before evaluating a physical cutter.

### 3.3 Removal at a point

For horizontal distance `r` from a V-bit center, define height above its tip plane:

```text
k(r) = 0                            when r <= r_t
k(r) = (r - r_t) / tan(alpha)        when r_t < r <= R_v
```

A pose at depth `d` removes depth `max(0, d-k(r))` within its cutting footprint. The endmill removes depth `d` within radius `R_e`. The stock model takes the maximum removal over the entire cutting trajectory, including entry moves.

Rounded or damaged tips require a different cutter model; a nominal flat tip diameter does not model every real engraving tool.

### 3.4 M1 finite-tip capability preview

The best removal at artwork location `x` is not `d_max(x)`: the flank can reach `x` with the tool center elsewhere. Let `s(q)` be signed boundary clearance (positive inside `P`, negative outside) and `m = tan(alpha)`. For the validated cone and `D <= usable_conical_height`, define:

```text
M(x, r_t) = max s(q) over |q - x| <= r_t
A_v(x)   = clamp((M(x, r_t) - r_t) / m, 0, D)
```

To see why flat-tip coverage suffices, consider a feasible pose at depth `d` that removes `x` to depth `t`. Its center is at most `r_t + (d-t)*m` from `x`. Move the center toward `x` until the flat tip covers it, by at most `(d-t)*m`. Signed clearance is 1-Lipschitz, so the moved center still has clearance at least `r_t + t*m` and is feasible at depth `t`. Conversely, any such center within the tip disk removes `x` to `t`. Maximizing clearance over that disk therefore gives the best geometric capability.

M1 evaluates this maximum with a branch-and-bound search using independent boundary distances. Feasible samples provide lower bounds; cell radii and the Lipschitz property provide upper bounds. It returns depth/residual intervals and an explicit unresolved status when the requested resolution cannot be met within the cell budget or floating-point resolution. Input snapping uncertainty is recorded separately; the floating-point reserves are engineering margins, not a formal interval-arithmetic proof.

The preview compares `A_v` with the unchanged nominal `T`. This is V-bit capability over arbitrary feasible poses, not combined endmill/V-bit stock removal or evidence that a path visits those poses. Profiles sample specified lines; the SVG interpolates between samples for display. See the [M1 capability report](m1-capability-report.md) for analytic corner/channel checks and center-set representation limits.

## 4. SVG normalization

The first importer accepts filled closed paths and basic closed shapes from Inkscape, transforms, physical units, `viewBox`, compound paths, and both `evenodd` and `nonzero` fill rules. Preserve stable source IDs for selection and diagnostics.

Process in this order:

1. Parse the supported subset and identify unsupported geometry-affecting features.
2. Resolve units, transforms, visibility, and supported inherited fill properties.
3. Convert basic shapes and path curves into bounded-error polylines in job coordinates.
4. Resolve filled regions and selected-shape unions using the declared fill rule.
5. Establish outer rings and islands; eliminate exact duplicates and zero-length edges.
6. Quantize, then check for collapsed features and non-endpoint segment intersections.
7. Supply normalized nonintersecting boundaries to the geometry adapters.

Flattening tolerance applies after transforms so scaling does not amplify an untracked error. SVG arc commands must also be supported or reported explicitly. Reversed ring orientation alone must not change the selected filled region.

Initially require text and strokes to be converted to paths in Inkscape. Report open paths, external references, masks, clip paths, filters, and unsupported styling. Ignore non-geometric editor metadata. Do not automatically close a substantial gap or remove a tiny island without a diagnostic.

M2 implements the supported subset with `roxmltree` 0.21.1 and `svgtypes` 0.16.1. It resolves filled components in page coordinates before workpiece placement, preserving IDs across placement edits; both source and final snapping budgets are recorded. Unsupported rendering effects and out-of-page geometry are rejected. See the [M2 capability report](m2-capability-report.md) for exact subset boundaries, transformed curve bounds, Inkscape measurements, and unresolved precision cases.

## 5. Endmill planning

For depth levels ending exactly at `D` and respecting the chosen maximum stepdown:

1. Construct `C_e(d, a)` for the current level.
2. Generate offset clearing loops with stepover no greater than the configured fraction of tool diameter.
3. Handle disconnected components independently; record empty or unreachable components.
4. Add entries and links, validating the whole cutter sweep against the target and current stock.
5. Simulate emitted cutting moves and update the stock representation.

Smaller inward offsets at shallow depths allow more upper stock to be removed. Do not reuse the deepest center region for every level except in the initial conservative prototype.

Use a ramp entry when the selected tool and available region allow it. A direct plunge requires a plunge-capable tool and an explicit plunge feed. If no supported entry fits, report that region rather than inventing an entry. Start with retract-to-clearance links between disconnected cuts; optimize in-stock links only after they can be verified.

The generated paths, not the intended pocket region, determine what material was removed. Check for gaps between offset loops and residual regions near topology changes.

## 6. V-bit planning

### 6.1 Candidate finishing geometry

Construct the interior segment Voronoi diagram of normalized boundaries. Extract medial-axis branches relevant to the removal region, excluding islands, exterior cells, and non-medial endpoint artifacts. Preserve source-feature associations and the local clearance radius.

The adapter must represent straight and curved edges and support bounded-error evaluation along them. Endpoint radii or a visually sampled skeleton are insufficient for machining.

Generate three complementary candidate families:

- Boundaries of `C_v(D)` at full depth for broad-area wall finishing.
- Interior medial-axis branches with `d = (rho-r_t)/tan(alpha)` where positive and below the depth cap, for tapering channels and terminal details.
- Full-depth area-clearing paths in the admissible V-bit center region for residual floor material.

Split branches at depth-cap crossings, finite-tip reachability limits, and topology events. Connect or overlap families within the tolerance budget so the constant-depth and rising paths do not leave seams. Simply clipping all medial-axis depths to `D` does not clear a broad floor.

Retain branches that lie exactly at the depth cap when they represent a valid centerline in an otherwise collapsed center region. A valid isolated tool position may require an explicit cutting plunge; it must not disappear as a zero-length XY path.

### 6.2 Rest clearing

Use stock by depth, as described below. Identify which candidate sweeps intersect remaining material. A valid center may lie in already cleared space while its flank cuts residual material, so do not require tool centers to lie inside a residual polygon.

The initial implementation generates complete candidate paths and conservatively removes only sections proved to be air cutting. Preserve the final boundary-finishing pass even where a coarse stock preview suggests it could be omitted.

Add intermediate V-bit depth passes within the configured stepdown. Respect stock remaining before the V-bit stage rather than assuming that every channel was roughed by the endmill. Stepdown is a geometric limit, not a prediction of acceptable cutting forces.

For floor cleanup, use clipped parallel lanes plus boundary coverage first. Recompute residuals after their actual sweeps. Narrow leftover regions require another valid path family or an explicit unreachable/unfinished diagnostic; reducing the lane spacing alone must not loop forever.

Finish with the complete achievable boundary, including island boundaries and rising corner paths. Output order is all endmill work followed by all V-bit work.

### 6.3 Floor ridges

For adjacent parallel V-bit passes at the same depth, spacing `s`, and flat tip radius `r_t`, the ideal ridge height is:

```text
h = max(0, (s/2 - r_t) / tan(alpha))
s <= 2 * (r_t + h_allowed*tan(alpha))
```

This relation applies between long straight passes; entries, endpoints, turns, and residual boundaries require stock verification. With a pointed bit and zero allowed ridge height, positive-spaced area clearing cannot satisfy the request. Reject that configuration when V-bit floor clearing is necessary. Report unreachable detail separately from allowed floor ridges.

## 7. Stock and verification

### 7.1 Model

The supported stock remains a height field: at each XY position it has one upper material surface. Let `A(x)` be the depth actually removed by all prior cutter sweeps. Then:

```text
overcut(x)  = max(0, A(x) - T(x))
residual(x) = max(0, T(x) - A(x))
```

Outside `P`, target depth is zero. Islands use the same rule. Do not mask the comparison to a preview image of the pocket, which could hide removal outside the intended region.

Use two representations with explicit purposes:

- Depth-slice polygons for conservative rest decisions and area coverage.
- A sampled/adaptive height field for interactive preview and quantitative cross-checks.

Neither a fixed coarse XY grid nor a few fixed depth slices prove continuous correctness. Verification records uncertainty and refines ambiguous regions. If a required bound cannot be established within resource limits, return `inconclusive`, not a successful check.

### 7.2 Sweeps at a depth slice

At slice depth `t`, a tool pose contributes a disk only when its tip depth `d >= t`. The disk radius is `R_e` for the endmill and `r_t + (d-t)*tan(alpha)` for the V-bit within its modeled cutting section.

For linear XYZ motion, first clip the segment to the parameter interval where `d >= t`. An endmill then sweeps a capsule. The V-bit's disk radius changes linearly; its planar sweep can be constructed as the convex hull of the endpoint disks, including endpoint caps. Approximate circular boundaries with controlled inner/outer bounds.

At a slice:

```text
A_t = union of actual cutting sweeps at depth t
R_t = P_t minus A_t
```

Carry lower and upper removal bounds when approximations matter. Air-cut pruning uses removal known to have occurred; overcut detection uses the possible removal extent. Refine slices near entry depths, floor levels, and changing topology. The continuous-error strategy must be demonstrated in the verification milestone.

### 7.3 Independent checks

Use independent point-to-boundary distance evaluation for tool-admissibility checks rather than relying entirely on the same offset library that generated the path. Validate a continuous segment with analytical bounds or adaptive subdivision that carries an error bound. Checking only endpoints or a fixed number of samples is insufficient.

Verify:

- Cutting sweeps respect the target envelope, islands, depth cap, and tool dimensions.
- Entries and cutting links are included in stock removal.
- Rapid motion occurs on a configured clearance plane with separate Z retract, XY travel, and Z approach moves.
- Claimed residual and floor-ridge bounds include geometric and simulation uncertainty.
- Quantized output motions still meet these requirements.

A successful geometric result applies to the modeled tools and stock. It does not certify fixtures or hidden motions inside M6 macros.

Quality acceptance has separate criteria: overcut must stay within the stated numerical budget; reachable floor residue must satisfy `max_floor_ridge_mm`; unreachable nominal detail must satisfy `max_detail_residual_mm`. Other reachable stock must be removed within the geometric/verification budget. Unreachable classification requires geometric evidence, not merely a failed cleanup search. Any uncertainty larger than the relevant criterion makes verification inconclusive.

## 8. Domain and artifact contracts

| Type | Required information |
| --- | --- |
| `Job` | Schema version, embedded source snapshot, selection, transform/origin, stock, operation, tools, tolerances, optional machine profile. |
| `Tool` | Stable ID, kind, dimensions, cutting limits, spindle speed, cutting/plunge feeds, stepdown and relevant stepover. |
| `FlatVCarveOperation` | Selected region IDs, depth cap, endmill/V-bit IDs, horizontal wall allowance, floor-ridge/detail-residual limits, clearing strategy. |
| `NormalizedGeometry` | Rings with hierarchy, source mapping, integer scale, geometric bounds, normalization diagnostics. |
| `Motion` | Explicit start/end XYZ, kind, tool and operation IDs, feed where applicable; linear segments initially. |
| `Plan` | Validated job snapshot including tools, normalized geometry, input fingerprint, engine/dependency versions, ordered operations and motions, tool-change markers. |
| `VerificationReport` | Passed/failed/inconclusive status, error bounds, overcut/residual findings, locations, and model limitations. |
| `MachineProfile` | LinuxCNC settings, work offset, clearance plane, tool mapping, length-compensation policy, M6 contract, output precision. |

Use versioned JSON for jobs and planning artifacts. UI transport schemas are derived from or checked against the Rust model. Preserve the input SVG snapshot for repeatability; an external filename alone is insufficient. Derived previews are replaceable caches.

A saved job can be incomplete while the user is editing it. Planning validates all required machining fields. Export additionally requires a complete machine profile and a successful verification of the required bounds.

M2's schema-version-1 `Job` implements embedded artwork, import placement/precision, selected component IDs, nullable stock/operation/tool settings, tolerances, and an optional editable machine profile. It stores no trusted normalized-geometry cache. The implemented `import`, `inspect`, `select`, and `validate-job` commands rebuild/validate the source snapshot. M3 and M4 extend the job through schemas 2 and 3 and implement `plan`, `inspect`, and `verify` for endmill/combined artifacts. Export still requires M5 verification and the M6 machine profile.

CLI (`export` and `serve` remain planned):

```text
cam import artwork.svg --output job.json
cam plan job.json --output job.plan.json
cam inspect job.plan.json --output preview.svg
cam verify job.plan.json --output verification.json
cam export job.plan.json --output job.ngc
cam serve
```

`import` creates an editable job; it does not invent tool or feed settings. `export` rechecks artifact fingerprints and required verification rather than trusting a stale status field. Progress goes to stderr and machine-readable results to files/stdout. Failure returns a nonzero exit status and stable diagnostic codes.

## 9. LinuxCNC postprocessor

MVP output uses explicit linear `G0`/`G1` moves and a known modal state: millimeters, absolute XYZ, XY plane, units-per-minute feed, and no controller-side XY cutter compensation. Start with exact-path mode `G61`. Arc fitting and bounded blending are later optimizations because they change the executed path assumptions.

LinuxCNC distinguishes `G61` exact path, `G61.1` exact stop, and `G64` blending; unrestricted blending can deviate from programmed geometry. [LinuxCNC path-control documentation](https://linuxcnc.org/docs/stable/html/gcode/g-code.html#gcode:g61)

The machine profile must select one length-compensation contract:

1. **Macro managed:** the existing M6 macro performs the required measurement/compensation; the post does not overwrite it.
2. **Post managed:** the post applies the configured tool-table length offset after M6.

Standard LinuxCNC M6 does not itself change the tool-length offset. Existing custom macros may do more. Verify their behavior before choosing the contract. [LinuxCNC M6 documentation](https://linuxcnc.org/docs/stable/html/gcode/m-code.html#mcode:m6)

Before a tool change, retract according to the known current setup and stop the spindle. Emit the mapped tool selection and M6. Restore the required units, positioning/feed/path modes, work offset, and spindle/feed settings according to the macro contract before cutting resumes. Never blindly cancel a valid dynamic length offset. Do not invent G28/G30/G53 positions or probing commands.

The combined program groups both stages with descriptive comments and tool IDs. Separate per-tool exports contain complete setup/end sequences. Account for output rounding by regenerating the numeric move list from emitted words and validating it against the plan. The emitted-subset reader is not a general LinuxCNC interpreter; macro semantics are covered by the profile and machine tests.

## 10. Diagnostics and unresolved engineering work

Diagnostics have a stable code, severity, stage, source region/operation, and optional geometric location. Expected cases include unsupported SVG features, collapsed regions, invalid tool geometry, unreachable detail, incomplete floor clearing, overcut, uncertain verification, missing machine settings, and cancellation.

Planning may return a partial diagnostic preview, but it must label it incomplete. No automatic geometric repair or resource-limit fallback may silently remove requested detail.

M4 implements bounded curved-medial paths and measured rest-floor coverage for the documented fixture workload. The remaining verification work is adaptive global stock/quality bounds, accumulated Boolean/topology uncertainty, and output quantization. Those acceptance tests precede usable machine output in the implementation plan.
