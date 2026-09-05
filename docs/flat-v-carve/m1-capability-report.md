# M1 capability report

Date: 2026-09-05\
Status: complete on the initial Linux x86-64 target. Engine version: **0.2.0**.

M1 implements the nominal carving target, validated endmill/finite-tip V-bit geometry, tool-center regions, independent clearance queries, and editable headless JSON/SVG previews. It meets the [M1 exit criteria](implementation-plan.md#4-m1-target-and-tool-geometry). Path planning, entry moves, stock simulation, and machine output remain later milestones.

## Reproduce

From [`flat-v-carve/`](../../flat-v-carve/README.md):

```sh
cargo build --workspace --release --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
cargo run --release --locked -p cam-app -- geometry-spike --output artifacts/m0
cargo run --release --locked -p cam-app -- target-demo --output artifacts/m1
cargo run --locked -p cam-app -- validate-model \
  --input fixtures/m1/finite_tip_corner.json
cargo run --release --locked -p cam-app -- target-preview \
  --input fixtures/m1/finite_tip_corner.json --output artifacts/corner
```

All checks passed with Rust 1.95.0 on Ubuntu 24.04.4/WSL2, `x86_64-unknown-linux-gnu`. The cached build used `CARGO_HOME=/tmp/flat-v-carve-cargo` and `--offline`. Direct dependency versions are unchanged from M0. Windows-native and WebAssembly builds are untested.

The workspace has **37 passing integration tests**: 12 M0 geometry tests, 20 M1 model/geometry tests, and 5 CLI tests. The release executable passes **28/28 M0 fixtures** and produces **8/8 complete M1 previews**. All eight generated SVGs parse as XML; the corner and isolated-point SVGs were also visually inspected.

## Implemented contracts

| Component | Implementation |
| --- | --- |
| Units and cutters | [`model.rs`](../../flat-v-carve/crates/cam-core/src/model.rs): validated lengths/depths/angles, physical tip plane, cutting dimensions, usable height/length, and plunge capability. |
| Independent clearance | [`geometry/query.rs`](../../flat-v-carve/crates/cam-core/src/geometry/query.rs): point containment, signed boundary distance, and minimum distance over an entire segment. |
| Target and center access | [`target/`](../../flat-v-carve/crates/cam-core/src/target/): nominal depth, depth sections, allowance, cutter-center feasibility, contacts, and finite-tip reachability. |
| Serializable preview | [`preview.rs`](../../flat-v-carve/crates/cam-core/src/preview.rs): strict M1 input validation and in-memory plan/profile results. |
| CLI and SVG | [`cam-app/src/`](../../flat-v-carve/crates/cam-app/src/): `validate-model`, `target-preview`, and eight-model `target-demo`. |

Depth is nonnegative downward and machine Z is `-depth`. With `m = tan(included_angle/2)`, the nominal target is `T(x) = min(D, rho(x)/m)` inside the opening and zero outside or in holes. At depth `d`, required boundary clearances are:

| Region | Required clearance |
| --- | --- |
| Nominal depth section | `d*m` |
| Endmill center | `d*m + endmill_radius + wall_allowance` |
| V-bit center | `d*m + tip_radius` |

Endmill compensation is applied once. V-bit dimensions must satisfy `tip_radius + cutting_height*m <= max_cutting_radius`; the tip may have zero diameter, but usable dimensions must be positive and finite. The target and cutter angles must match. A V-bit shorter than the requested depth cap is rejected even when querying a shallower section. The complete M1 preview also requires sufficient endmill cutting length for the cap. Unsupported dimensions do not redefine or clamp the nominal target.

Independent distance queries operate on normalized polygon segments without calling the offset or Voronoi libraries. Query points are not snapped to the polygon grid. Segment-distance queries inspect the whole segment, including crossings of an intervening island. These are geometric primitives, not complete XYZ motion verification.

## Analytic and degeneracy evidence

The [model tests](../../flat-v-carve/crates/cam-core/tests/target_models.rs) compare calculations against geometric references rather than only checking serialized output.

| Case | Reference and observed behavior |
| --- | --- |
| Straight channels | Included angles 30°, 60°, 90°, and 120° reproduce depth `min(D, W/(2*m))` and floor width `max(0, W-2*D*m)` within the declared test tolerance. |
| Wide-channel demo | A 20 × 10 mm opening at `D=2`, 90° has a 16 × 6 mm nominal floor. A 4 mm endmill plus 0.5 mm allowance leaves an 11 × 1 mm center region. |
| Narrow-channel demo | A 4 mm-wide opening with a 3 mm cap reaches only 2 mm nominal depth at 90°. Its pointed V-bit center set at depth 2 is a line; the 6 mm endmill has no access. |
| Finite-tip offset | A 1 mm tip diameter adds 0.5 mm to center clearance relative to the nominal depth section. Editing tip diameter preserves the nominal target. |
| Preserved island | Inward removal offsets expand the preserved hole. Independent containment checks reject points in the added island margin. |
| Exact-fit line | A 20 × 8 mm rectangle, `D=2`, 90°, and 4 mm endmill retain the 12 mm center segment from `(4,4)` to `(16,4)` with zero area. Increasing diameter to 4.00001 mm yields no access. |
| Isolated centers | An 8 × 8 mm square retains `(4,4)`; a 6–8–10 triangle with 2 mm endmill at depth 1 retains its incenter `(2,2)`. |
| Mixed components | An exact-fit line coexists with a separate positive-area center region of 16 mm². |
| Sub-grid region | A 3.99996 mm endmill in the line fixture leaves a center strip 0.00004 mm wide. The automatic grid reports `CENTER_SET_UNRESOLVED`; at 1,000,000 ticks/mm the area is `12.00004 * 0.00004 = 0.0004800016 mm²`, within `1e-9 mm²`. |
| Wall reachability | At 0.25 mm from a straight wall with a 0.5 mm tip radius and 90° cone, admissible center depth at that point is zero but achievable removal is 0.25 mm using the flank. |
| Right-angle corner | For `(s,s)` near the corner, reachable depth is `max(0, s-r_t*(1-1/sqrt(2)))` at 90°, before the cap or other walls intervene. Computed intervals enclose the analytic result. |
| Acute wedge | For a wedge of half-angle `phi`, on-axis distance `x`, and slope `m`, reachable depth is `max(0, ((x+r_t)*sin(phi)-r_t)/m)` before other constraints intervene; checked at 15°, 30°, and 45°. |
| Invariance | Target and reachability results survive translation, a 90° rotation, and dimensional scaling by 0.1 and 10 with scaled tolerances. |

`CenterSet` carries polygon area plus zero-margin segments and points. Interior Voronoi vertices identify point contacts; primary edges between parallel boundary segments identify constant-clearance line contacts. Independent distance checks validate the entire retained line. Floating-point reserves classify contact; the geometry tolerance is not used to admit an oversized tool.

Positive-clearance Voronoi vertices and primary-edge midpoints absent from the offset polygon trigger an unresolved representation diagnostic. A nonempty input with no interior Voronoi vertices is also explicitly unresolved. These witnesses catch the demonstrated lost features, but do not prove arbitrary offset topology or error bounds. The authoritative center predicate remains signed boundary clearance. Full medial-axis extraction is M4, and verification of motion and machining margins is M5. Some reported contact points can be redundant with area-boundary junctions.

## Finite-tip reachability and numerical bounds

The [technical design](technical-design.md#34-m1-finite-tip-capability-preview) derives the capability calculation:

```text
M(x, r_t) = max signed_clearance(q), for |q-x| <= r_t
A_v(x)   = clamp((M(x, r_t)-r_t)/m, 0, D)
```

A deterministic branch-and-bound search uses sampled feasible centers for lower bounds and the 1-Lipschitz property of signed distance for cell upper bounds. It stops when the reachable-depth interval is within the requested preview tolerance. Resource or numerical limits preserve the interval and return an unresolved status. A test with a one-cell budget confirms that a difficult corner cannot falsely claim resolution; refinement tightens the bounds.

The eight release demos contain **9 profiles and 1,161 samples**. All request 0.001 mm depth uncertainty with a 20,000-cell budget per sample. The largest computed interval is **0.000809011 mm** in `finite_tip_corner`, requiring at most **77 cells**. Other demo intervals are below `1.2e-12 mm`. For the 0.5 mm tip radius, the analytic uncapped right-corner residual is `0.5*(1-1/sqrt(2)) = 0.146446609 mm`; the largest sampled residual lower bound is **0.145894967 mm**.

These bounds refer to the normalized polygon and idealized cutter. Floating-point reserves are engineering margins, not formal outward-rounded interval arithmetic. The report records input depth uncertainty from snapping separately as `min(D, snap_bound/m)`. A complete preview means the requested sampled geometry resolution was met; it is not a machining verification result.

The SVG uses blue for nominal geometry, orange for V-bit access/removal, teal for endmill centers, and pink profile markers for finite-tip residual above the requested preview tolerance. Connecting profile samples is visual interpolation. No claim of coverage between samples or over the entire artwork is made. V-bit residual does not account for possible endmill removal, toolpath visitation, entries, holders, or a real stock surface.

## Editing, diagnostics, and next boundary

Each generated model directory contains a replayable `input.json`, a `report.json` with build identity and numerical results, and `preview.svg`. The eight [input models](../../flat-v-carve/fixtures/m1/) are synthetic procedural polygons with illustrative cutter dimensions. They establish no machining feeds or physical tool defaults.

Model inputs validate schema version, dimensions, range, geometry, profile identifiers, and work limits. Limits are 64 depth sections, 16 profiles, 2–1,025 samples per profile, 4,096 samples per preview, and 1–1,000,000 reachability cells per sample. Existing polygon limits from M0 still apply. `validate-model` checks settings without evaluating all center sets or profiles; use `target-preview` to check the resulting geometry.

The [CLI tests](../../flat-v-carve/crates/cam-app/tests/cli.rs) verify deterministic replay, invalid tool edits, explicit inconclusive exit status, and replacement of a previous successful SVG by an error view when a parsed model fails validation. M1 returns exit code 1 for invalid settings or an inconclusive preview, and 2 for command/JSON/I/O errors. Malformed JSON is a parse error before preview generation; its exit status must be checked when invoking the CLI.

M2 will add Inkscape SVG normalization and versioned, editable jobs. The strict M1 model input is a geometry experiment format and does not implement that job contract. M3–M5 will build and verify cutting sequences using these primitives. No G-code or machine operation is implemented by M1.
