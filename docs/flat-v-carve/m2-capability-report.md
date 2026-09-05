# M2 capability report

Date: 2026-09-05\
Status: complete on the initial Linux x86-64 target. Engine version: **0.3.0**.

M2 implements SVG normalization, source/component mapping, portable editable jobs with embedded artwork, region selection, and headless inspection. It meets the [M2 exit criteria](implementation-plan.md#5-m2-svg-import-and-saved-jobs) for the supported subset below. Cutting paths begin in M3; M2 does not create a machining plan.

## Reproduce

From [`flat-v-carve/`](../../flat-v-carve/README.md):

```sh
cargo build --workspace --release --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
cargo run --release --locked -p cam-app -- import fixtures/m2/inkscape-export.svg \
  --output artifacts/m2/job.json
cargo run --release --locked -p cam-app -- inspect artifacts/m2/job.json \
  --output artifacts/m2/preview.svg --report artifacts/m2/report.json
cargo run --locked -p cam-app -- validate-job artifacts/m2/job.json
cargo run --locked -p cam-app -- select artifacts/m2/job.json \
  --select letter-b::0 --output artifacts/m2/selected.json
```

The release build, Clippy with warnings denied, formatting, and **65 integration tests** pass on Rust 1.95.0, Ubuntu 24.04.4/WSL2, `x86_64-unknown-linux-gnu`. There are 24 M2 core tests and 4 M2 CLI tests, alongside the previous 37 tests. Cached builds used `CARGO_HOME=/tmp/flat-v-carve-cargo` and `--offline`. Release regressions also pass **28/28 M0 fixtures** and **8/8 M1 demos**. Native Windows and WebAssembly builds of the CAM application remain untested.

## Parser decision and supported SVG subset

[`roxmltree` 0.21.1](https://docs.rs/roxmltree/0.21.1/roxmltree/) preserves the XML structure needed for source IDs, inherited styles, and explicit rejection of unsupported features. [`svgtypes` 0.16.1](https://docs.rs/svgtypes/0.16.1/svgtypes/) supplies lexical parsing for paths, transforms, lengths, numbers, and colors. The application owns rendering-subset decisions and normalization; it does not rely on a renderer silently discarding objects. Both dependencies are pinned, with transitive versions in `Cargo.lock`. Existing geometry dependency versions are unchanged.

| Feature | M2 behavior |
| --- | --- |
| Page size and units | Explicit positive root width/height; mm, cm, in, pt, pc, px, or unitless CSS pixels at 96 px/in. Percent/font-relative lengths and missing page size are rejected. |
| `viewBox` | Positive dimensions, nonzero origins, `preserveAspectRatio="none"`, and all nine aligned `meet` modes, including the default. `slice` and nested viewports are rejected. |
| Coordinates | SVG Y is reversed about page height. Workpiece placement uses `scale * rotate(page_XY - origin_mm)`, with positive uniform scale and rotation in degrees. |
| SVG transforms | Matrix, translate, scale including reflection/nonuniform scale, rotation about a point, and skew; nested transforms compose in SVG order. Singular/nonfinite/excessive matrices are rejected. |
| Paths | Absolute/relative M/L/H/V/C/S/Q/T/A/Z, repeated command coordinates, multiple subpaths, smooth-control reflection, elliptical arc rotation/flags, and SVG radius correction. Every subpath must explicitly close or end exactly at its starting point. No gap is automatically bridged. |
| Basic shapes | Rectangles, rounded rectangles, circles, ellipses, and polygons with positive dimensions. |
| Filled regions | `nonzero` and `evenodd`, holes, nested islands, intentional self-crossings and overlapping contours. Separate selected objects are unioned as filled regions. |
| Styles | Presentation attributes and inline styles; inherited fill/color/currentColor, fill rule, visibility, fill/stroke opacity, and stroke dimensions. Inline priority and `!important` precedence are respected. |
| Hidden/transparent content | `display:none`, zero cumulative opacity, hidden visibility, no fill, and transparent paint are excluded with diagnostics. Descendants can override inherited visibility; they cannot override a suppressed ancestor. |
| Editor metadata | Inkscape/Sodipodi metadata, titles, descriptions, and unreferenced definitions are ignored. Layer labels are preserved when present on a source element. |

Positive opacity retains the same geometric boundary and produces a diagnostic; it is not mapped to carving depth. Visible strokes and text require Inkscape conversion to paths. CSS stylesheets, paint servers/gradients, references/clones/images, clipping/masking/filter/marker effects, scripts, animation, conditional rendering, foreign rendering namespaces, and unsupported style properties are rejected. Geometry outside the page is rejected because viewport clipping is not implemented; resize the page to the artwork first. Empty/cancelling filled contours and zero-size basic shapes are explicit errors rather than silent removal.

The coordinate and arc conversions follow the [SVG coordinate-system specification](https://www.w3.org/TR/SVG/coords.html) and [elliptical-arc implementation notes](https://www.w3.org/TR/SVG/implnote.html#ArcImplementationNotes). These define the conversion formulas; application-specific precision and topology checks are described below.

## Geometry, precision, and source mapping

[`svg/path.rs`](../../flat-v-carve/crates/cam-core/src/svg/path.rs) resolves path commands and flattens curves after the document's full affine transform. Page-space tolerance is divided by job scale, so the final workpiece-space curve bound is **`e/4`**, where `e` is `geometry_tolerance_mm`. Rotation and translation do not amplify this bound.

Cubic subdivision uses the convex hull of its transformed controls and their distances to the endpoint segment. Quadratics are converted exactly to cubic controls. For a transformed ellipse `o + u*cos(t) + v*sin(t)`, `|r''(t)| <= |u| + |v|`; linear interpolation error is bounded by `(|u|+|v|)*delta_t²/8`. Elliptical arcs use endpoint-to-center conversion before this subdivision. Resource exhaustion returns a diagnostic instead of increasing the allowed error.

Filled contours are first resolved on a page grid chosen for at most `e/16` input snapping after placement. Components receive IDs in page coordinates before rotation/origin changes, preventing placement from swapping the selected disconnected region. They are then transformed and snapped to the workpiece grid with at most `e/8` additional input snapping. Reports record `source_grid`, the scaled `source_snap_bound_mm`, the final `grid`, and `flattening_bound_mm` separately. A finer explicit `ticks_per_mm` also refines the source grid.

For the bundled export at `e=0.001 mm`, the curve bound is **0.00025 mm**, source snapping is at most **0.0000070711 mm**, and final snapping is at most **0.0000707107 mm**. These are processing budgets, not a universal proof of accumulated Boolean/roundoff error. Clipper can introduce further rounding at intersections. As in M0, numerical evidence applies to the checked cases; complete machining verification remains M5.

The fill adapter detects collapsed edges, orientation changes, and changed crossings/contacts after snapping, including between distinct source objects. Resolved output is checked against the geometry engine's nonintersecting-boundary contract. Placement must preserve component/hole counts. A test with two rectangles separated by 0.000001 mm rejects a coarse-grid contact and preserves both components at 1,000,000 ticks/mm. Very small features can still be unresolved; no general proof of topology below the declared approximation budget is claimed.

Each selectable component has `source-id::index`, source ID, optional label, and geometry. Source IDs are unique; missing IDs get deterministic names that avoid existing IDs. IDs remain stable for the unchanged snapshot and checked placement edits; editing the source or changing topology may change the component set and requires reviewing selection. Unknown/duplicate selection IDs are rejected. The selected union also records which source components contribute positive area to each result, so overlapping shapes retain their provenance. An empty selection is a valid editing state.

## Inkscape and analytic evidence

The [fixture directory](../../flat-v-carve/fixtures/m2/README.md) contains an authored coupon, native/plain exports created by **Inkscape 1.4.4 (dcaf3e7, 2026-05-05)** on Windows, exact regeneration commands, and Inkscape's own bounding-box query output. Arial text B was converted to path geometry by Inkscape. Inkscape is a fixture-generation tool, not a runtime dependency.

Both exports import into **7 source regions, 6 selected components, and 3 holes**. The normalized source geometry has **2,671 vertices**. All seven source bounds agree with Inkscape's independently reported bounds within **0.000170834 mm**, below the 0.001 mm test threshold. Its decimal-rounded query results supplement analytic tests; they do not certify all curve points. The plain/native reports retain equivalent selection and area. The generated preview was visually inspected and generated SVGs parsed as XML.

| Reference | Evidence |
| --- | --- |
| Physical units | 96 px, 72 pt, 6 pc, 1 in, 2.54 cm, and 25.4 mm produce the same physical dimensions. |
| Compound O/B | O has area 300 mm² and one hole; converted B retains two holes. Reversing complete winding preserves the nonzero-filled region. |
| Fill rules | Same-winding nested contours fill under nonzero but form a hole under evenodd; opposite winding yields the expected nonzero hole. |
| Intersections | Overlapping subpaths and a bowtie resolve to their analytic areas before downstream geometry use. |
| Curves | Circle/rounded-rectangle areas, arc direction/large-arc/radius correction, and smooth-versus-explicit controls match references. |
| Full transform error | A dense independent cubic reference after shear, nonuniform SVG scale, and rotated/scaled job placement stays within the recorded curve-plus-input-snap budgets. |
| Stable selection | Disconnected subpaths retain the selected component through rotations of 27°, 90°, 180°, and 270°. Origin and scale changes preserve physical dimensions and selection. |
| Unsupported content | Live text, visible strokes, references, effects, open paths, relative units, CSS, ambiguous size, and dynamic content produce explicit errors. Hidden and transparent content cannot become unintended cuts. |
| Resource/precision failure | Tiny edges, new snapped contacts, excessive curve refinement, source size, and nesting depth fail explicitly. |

## Editable jobs and CLI contract

[`job.rs`](../../flat-v-carve/crates/cam-core/src/job.rs) defines JSON **schema version 1** with embedded source text/filename, import precision and placement, selected component IDs, stock settings, one operation, stable tool slots, tolerances, and an optional machine profile. Tool geometry, feeds, spindle speeds, stepdown/stepover, depth, allowances, stock thickness, and quality settings begin unset. Complete supplied cutter dimensions use M1's validation; missing settings are expected while editing. The current optional machine-profile fields are editable data, not an implemented M6 contract.

Jobs serialize only source/settings/selection, not a trusted geometry cache. Import/inspection rebuilds the normalized result from the embedded SVG. Tests save/reload identical JSON, preserve a subset and placement, delete the original file before inspection, edit the embedded SVG, and reject stale selection IDs. Unknown schema versions/fields and invalid supplied values are rejected. This format remains separate from the M1 procedural model JSON.

| Command | Output and meaning |
| --- | --- |
| `import artwork.svg --output job.json` | Creates an editable job; supports `--tolerance` and repeated `--select`. Unsupported artwork does not produce a successful partial job. |
| `inspect job.json --output preview.svg [--report report.json]` | Rebuilds geometry, renders selected/unselected regions, and lists unset machining fields in a JSON report. |
| `select job.json --select id --output selected.json` | Saves a new selection; omit IDs for an empty selection. |
| `validate-job job.json` | Writes machine-readable inspection JSON to stdout; incomplete but valid editing settings pass. |
| `plan job.json --output diagnostics.json` | Validates the job and returns `PLANNING_NOT_IMPLEMENTED`, missing settings, and no plan. Path generation begins in M3. |

Exit codes are 0 for successful editing/inspection, 1 for invalid SVG/jobs or unavailable planning, and 2 for command/I/O errors. Invalid inspected jobs, including malformed JSON, replace any previous preview/report with an error result. Failed imports leave existing output jobs untouched; callers must check exit status. File-read/I/O failures also require checking exit status. Reports do not claim machining readiness merely because input geometry is valid.

Limits are 2 MB SVG, 8 MB job JSON, 20,000 XML nodes, 64 nesting levels, 4,096 path commands or flattened vertices, and 32 cubic subdivision levels. Existing grid/range and downstream 4,096-edge limits remain. The pipeline uses pairwise topology checks and repeated Boolean operations; large-artwork performance has not been established. M3 should build its initial clearing strategy on these imported/selected regions and validate machining requirements before generating moves.
