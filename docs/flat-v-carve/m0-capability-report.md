# M0 capability report

Date: 2026-09-05\
Status: complete on the initial native target. This records the M0 checkpoint; subsequent target/cutter work is covered by the [M1 capability report](m1-capability-report.md).

The isolated [Rust workspace](../../flat-v-carve/README.md) builds and passes **28 geometry fixtures and 14 automated tests**. The tested adapters provide the polygon and segment-Voronoi primitives needed for the next milestone. No project-specific C++ or GUI dependency was needed. This is geometry dependency evidence; target/tool models and machining verification remain future milestones.

## Reproduction and build identity

From `flat-v-carve/`:

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
cargo run --release --locked -p cam-app -- geometry-spike --output artifacts/m0
```

All commands passed. The implementation session used `CARGO_HOME=/tmp/flat-v-carve-cargo` to keep downloaded dependencies in a writable sandbox directory; ordinary development can use its usual Cargo cache. The final checks also passed with `--offline` after fetching the native dependencies.

| Item | Tested value |
| --- | --- |
| Host | Ubuntu 24.04.4 LTS under WSL2, Linux 6.6.87.2-microsoft-standard-WSL2 |
| Rust target | `x86_64-unknown-linux-gnu` |
| Compiler | `rustc 1.95.0 (59807616e 2026-04-14)` |
| Toolchain pin | `rust-toolchain.toml`, 1.95.0, minimal profile plus rustfmt/clippy |
| Polygon adapter | `clipper2-rust = 1.1.0`, no features enabled |
| Voronoi adapter | `boostvoronoi = 0.12.1`, no features enabled |
| Source intersection predicates | `robust = 1.2.0`, no features enabled |
| JSON | `serde = 1.0.229`, `serde_json = 1.0.151` |
| Dependency reproducibility | Exact direct pins plus included `Cargo.lock` |

Native dependency metadata resolves 35 packages including the two workspace crates. The graph contains no `cc`, `cmake`, `cxx`, `bindgen`, or FLTK packages. The transitive crate named `cpp_map` is Rust code; its name does not imply a C++ build. The platform filter matters when inspecting metadata offline: the lockfile also records dependencies for other, untested targets.

## Measured evidence

The [fixture definitions](../../flat-v-carve/fixtures/m0.json) are stored in the project. The [generated report](../../flat-v-carve/artifacts/m0/report.json) and per-fixture JSON/SVG files are reproducible local artifacts, excluded via `.gitignore`. Each includes its input, selected grid, measurements, diagnostics, and any derived geometry. The complete run writes 85 files totaling about 0.95 MB.

| Check | Observed result | Acceptance |
| --- | --- | --- |
| Union, intersection, difference, XOR of overlapping squares | Exact analytic areas: 150, 50, 50, 100 mm² | Area error ≤ 1e-9 mm²; expected components/holes |
| 10 mm square, 1 mm inward offset | Exactly 64 mm² | Area error ≤ 1e-9 mm² |
| 20 mm pocket, 4 mm square island, 1 mm inward offset | 288.85945252 mm²; analytic error 0.001045174 mm² | Area error ≤ 0.02 mm²; one component/one hole |
| L region, 1 mm inward offset | 36.21486313 mm²; analytic error 0.000261294 mm² | Area error ≤ 0.01 mm² |
| Square-island and L offset boundaries | Maximum sampled clearance residual 0.000297232 mm | ≤ 0.001 mm |
| 2 mm neck, 1.1 mm inward offset | Two components | Exactly two components/no holes |
| 4 mm channel, offsets 2 and 2.1 mm | Empty area with `OFFSET_EMPTY_AREA` | No silent interpretation as absence of exact-fit centers |
| 256-sided annulus, radii 10/3 mm, 1 mm inset | Area error against ideal annulus 0.035554484 mm²; sampled clearance residual 0.001357301 mm | ≤ 0.1 mm² and ≤ 0.01 mm respectively |
| Concave L Voronoi | Two finite curved edges; reconstructed curves retain common endpoints | At least two curved edges |
| Square-island Voronoi | Eight finite curved edges | At least four curved edges |
| All bundled Voronoi distance checks | Maximum sampled equidistance and nearest-site residual 4.441e-15 mm | ≤ geometry tolerance / 16 |
| Concave L curve linearization | Sampled chord error 0.000237621445 mm; declared bound 0.000237635562 mm | Both ≤ 0.00025 mm |

The square-island reference is `18² - (4² + 16×1 + π×1²) = 292 - π`. The L reference is `37 - π/4`. These distinguish disk offsets from mitered graphical offsets. The annulus input itself has chord error ≤ 0.000753 mm, below one quarter of that fixture's 0.01 mm geometry budget.

The 12 core tests also check translation, 90° and arbitrary rotation, dimensional scaling by 0.1 and 10 with tolerances scaled together, alternative integer scales, hierarchy through Boolean operations, analytic rectangle medial vertices, nonfinite/range rejection, bounded curve refinement, explicit resource exhaustion, and deterministic repeated output. Two CLI tests cover bundle export, exact single-fixture replay, invalid arguments/IDs, and a deliberately incorrect reference that must retain artifacts and exit with status 1.

## Adapter and precision decisions

`cam-core::geometry` owns points, segments, regions with ring hierarchy, diagnostics, source-feature associations, and finite curves. Vendor polygon containers and graph handles remain private to adapters. Voronoi source indices refer to normalized boundary segments; `(ring, edge)` associations and segment endpoint categories are exported explicitly. Mapping back to original SVG object IDs is M2 work.

All coordinates use millimeters. Clipper receives scaled `i64`; Voronoi receives the same coordinates as `i32`. The application deliberately limits each integer coordinate to **±1,000,000,000**, below the i32 limit. Integer orientation/intersection predicates and area accumulation use `i128`. Nonfinite values and out-of-range coordinates fail before conversion.

For geometry tolerance `e`, select the smallest decimal scale `s ≥ 4√2/e`, with supported scale in `[1, 1e12]` and tolerance at least `1e-9 mm`. Require `max_abs_coordinate_mm × s ≤ 1e9`. The largest Euclidean displacement from nearest-grid rounding is `√2/(2s) ≤ e/8`. At `e = 0.001 mm`, the chosen scale is 10,000 ticks/mm, coordinate limit ±100,000 mm, and snapping bound 0.000070711 mm. Both positive and negative range endpoints are exercised. Impossible range/resolution combinations return `PRECISION_RANGE`.

Exact consecutive and closing duplicate vertices are removed with a warning. Other distinct vertices are preserved. Source segments are checked using [robust orientation predicates](https://docs.rs/robust/1.2.0/robust/fn.orient2d.html), followed by exact checks after snapping. Collapsed edges, newly introduced endpoint contacts/T-junctions, intersections, collinear overlaps, zero-area rings, and orientation reversals are rejected. Raw fixture rings must be simple with disjoint boundaries; touching rings receive `NON_SIMPLE_BOUNDARY` because their fill topology needs explicit resolution. Adjacent boundary edges share endpoints normally. Clipper results carry their own hierarchy and may have components meeting at shared endpoints.

Inward offsets use round joins, polygon end type, and explicit arc tolerance `e/4` in scaled units. Clipper's integer output adds rounding uncertainty; the fixture measurements include it. The input snap budget, arc approximation budget, and downstream budgets remain separate. These are declared processing allowances and measured capability results, not a universal proof of Clipper's total error on arbitrary polygons. Boundary validation currently uses quadratic pair checking with a 4,096-edge cap. Offset circle resolution and curve subdivision have explicit limits; this spike is not a performance claim for large artwork.

## Bounded finite-curve evaluation

A curved segment Voronoi edge is a point/line parabola. In orthonormal coordinates along its segment directrix, with focus `(u_f, v_f)`, the bisector is:

```text
v(u) = ((u - u_f)^2 + v_f^2) / (2 v_f)
```

The adapter reconstructs the finite interval as a quadratic Bézier `(P0, P1, P2)`. It keeps the dependency's shared vertices, measures their discrepancy against the reconstructed parabola, and includes the maximum discrepancy in the error budget. Moving the two Bézier endpoints by at most that amount perturbs the whole curve by at most the same amount.

For `n` uniform parameter intervals, the exact-arithmetic bound between the quadratic and its piecewise-linear chords is:

```text
chord_error ≤ |P0 - 2 P1 + P2| / (4 n²)
```

Subdivision is selected from this bound, not an unbounded visual sampling heuristic. Artifacts record the chord bound, endpoint discrepancy, numerical reserve, and their sum. Reconstruction/numerical overhead must fit within `e/16`; the fixture linearization receives `e/4`. Requests below the available precision or beyond the segment budget return `CURVE_PRECISION` or `CURVE_LIMIT`. Infinite edges retain their source sites and optional endpoints but have no fabricated finite approximation.

The numerical reserve is `128 × f64::EPSILON × max(1, coordinate magnitude of the control points)`. This is an engineering margin, **not interval arithmetic**. Independent source-distance checks and denser chord checks challenge the construction on fixtures. Sampled measurements do not establish continuous stock or motion correctness; M5 must aggregate and validate the eventual machining error bounds.

## Rejections and next milestone

Minimal rejected inputs are bundled for collapsed tiny segments, quantization-created contacts and T-junctions, orientation reversal, self-intersections, overlapping edges, ambiguous touching boundaries, repeated nonadjacent vertices, and an impossible precision range. A 0.0002 mm collinear segment survives the default fixture grid; a distinct 0.00004 mm segment is explicitly rejected. Replay any one with:

```sh
cargo run --locked -p cam-app -- geometry-spike \
  --fixture artifacts/m0/repro/snap_creates_t_junction.json \
  --output artifacts/replay
```

No unresolved dependency failure was found in this suite. Keep the tested [Clipper](https://docs.rs/clipper2-rust/1.1.0/clipper2_rust/) and [Voronoi](https://docs.rs/boostvoronoi/0.12.1/boostvoronoi/) versions behind the adapters. The full diagram includes exterior and secondary edges; complete medial-axis selection remains M4. The subsequent [M1 report](m1-capability-report.md) covers target cross-sections, finite-tip cutter models, tool-center feasibility, and explicit line/point handling when area offsets collapse. Native Windows, WebAssembly, SVG parsing, machining output, and physical validation were not tested or implemented by M0.
