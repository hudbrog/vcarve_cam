# M5 continuous stock and output-precision verification

Date: 2026-09-05\
Engine: 0.6.0\
Tested target: native Windows x64, `x86_64-pc-windows-msvc`, Rust 1.95.0.

M5 adds a separate bounded verification result to the M4 planner. `cam verify` on a combined plan authenticates and replays the saved artifact, compares the entire normalized target with actual recorded cutting sweeps, and optionally repeats that comparison after decimal coordinate formatting. A planning preview marked M4 `complete` is not an M5 pass.

## Acceptance and artifact contract

The report is under `verification` in the CLI JSON. `valid` says that the plan was readable and authenticated; callers must check `verification.status` and the exit code for geometric acceptance.

- `passed`: every terminal cell satisfies the relevant maximum-error criterion, the maximum-error enclosure widths satisfy `verification_tolerance_mm`, depth bands meet their refinement budget, and motion semantics pass.
- `failed`: a located point witness proves a criterion violation, or a motion/rounding contract is violated. Proven failures take precedence over unresolved cells elsewhere.
- `inconclusive`: bounds straddle a criterion or exceed the requested uncertainty and cannot be refined within the cell, depth, or arithmetic limits; depth-band exhaustion and unfinished generation records also prevent acceptance.

Overcut and other reachable residual are bounded by `verification_tolerance_mm`. Reachable residue at the nominal floor must satisfy `max_floor_ridge_mm`; independently established cutter-limited detail must satisfy `max_detail_residual_mm`. M5 does **not** add the verification tolerance to either user finish limit. Reports retain lower/upper bounds, locations, cell rectangles, motion IDs where applicable, refinement counts, and omitted-finding counts.

The core exposes `verify_motions` for independent challenges against raw motion lists and `verify_plan` for authenticated saved plans. The latter additionally checks existing execution records, the complete finishing family, and generation issues. Job, original motions, verification options, engine version, and authenticated plan identity bind the result. Rounded motions have their own fingerprint. Cached analysis never authorizes a plan. Deterministic reports contain no elapsed-time or memory measurements; those are recorded separately by the benchmark script.

Job schema 3 and plan schema 1 remain unchanged. Engine 0.6.0 invalidates plans made by earlier engines; regenerate them from saved jobs. Import, selection, and editing remain available without complete machining settings.

## Continuous bounds

Let `m = tan(included_angle / 2)`, `T(x)` be the normalized target depth, `A(x)` actual removal, and `H(x)` the best depth geometrically reachable by the configured cutters.

### Target and actual removal

The domain contains the target bounding box **and every cutting sweep's stock-top footprint**. Islands and exterior material remain in the comparison. Rapids, approaches, entries, and retracts receive semantic checks; plunges, ramps, and cuts contribute their actual XYZ motion to stock.

An independent boundary query encloses signed distance over each rectangular cell. It checks segment/rectangle intersections, including an island wholly contained in a cell. Distance to a segment is convex, so corner distances give an upper bound, while whole-segment distances to the box give a lower bound. Clamping the signed-distance interval divided by `m` to `[0,D]` encloses `T`.

For an endmill of radius `R`, the exact point-removal query at a cell center uses `R-rho` for a cell-wide lower bound and `R+rho` for an upper bound, where `rho` covers every point in the cell. This includes linear ramps and handles the discontinuity at a cylindrical sweep boundary without assuming the endmill stock surface is Lipschitz.

For the V-bit, actual removal is the maximum over each motion parameter of

```text
max(0, depth(t) - max(0, distance(x,center(t)) - tip_radius) / m).
```

Endpoint, closest-point, flat-tip transition, and stationary candidates solve the point query analytically. The swept removal surface is `1/m`-Lipschitz. Its unclamped surface is also concave for linear cutting motion; if all box corners are cut by that same sweep, corner minima enclose removal throughout the box. Distance to any boundary segment is a convex upper roof for the target (after division by `m`). Subtracting the concave sweep gives a convex residual upper roof whose maximum is at a box corner. This remains valid beyond segment endpoints and across changes of the closest target feature; it does not require a cell to fit beside one straight face. Nearby boundary candidates are found through the boundary index with an outward-rounded search distance. Different sweeps covering the four corners do not by themselves establish interior coverage.

Continuous variable-radius segment clearance proves a global overcut bound for admissible sweeps. Ambiguous clearance margins fall back to the adaptive stock comparison; a negative analytical *lower* margin is not by itself labeled a measured gouge. Independent motion point witnesses can establish failures immediately. All formulas include explicit floating-point reserves.

### Reachable versus unreachable material

The independent reachability solver maximizes signed boundary clearance over a disk of possible cutter centers. It returns lower/upper depth bounds, including when its own budget expires. This optimum is `1/m`-Lipschitz as the disk translates, so its bounds extend across the cell. A pointed V-bit has `H=T`, proving exactly zero cutter-limited residual.

If the V-bit tip radius is no greater than the endmill radius, its possible poses can reproduce every admissible endmill removal: translate its center toward the query by at most the radius difference. Otherwise both independent cutter reachability queries contribute. A failed cleanup search is never evidence of unreachability.

The separate comparisons are `max(A-T,0)` for overcut, `max(T-H,0)` for cutter-limited detail, and `max(H-A,0)` for missed reachable material. Floor and non-floor cells apply their separate criteria; cells crossing the floor boundary retain both possibilities until resolved. A second refinement pass tightens maximum-error enclosures when a criterion is already satisfied but its reported uncertainty is too wide. Both passes share the configured cell budget.

### Depth slices, areas, and volumes

The existing stock adapters construct inner/outer capsules for endmill motion and convex hulls of unequal endpoint disks for V-bit motion, clipping each linear XYZ move at the requested depth first. The existing rest-pruning proof requires whole-cutter containment in an actual endmill sweep.

M5 acceptance does not depend on repeated integer polygon unions. Terminal height-field cells instead supply independent lower/upper occupancy areas valid throughout each closed depth band. Bands include stock top, the depth cap, the requested ridge depth, and the deepest cutting endpoint; unresolved bands subdivide to the verification depth tolerance. Continuous XYZ sweeps do not require a separate band at every endpoint depth. Budget exhaustion retains the whole remaining band and returns `inconclusive`.

Cell integration also bounds residual and overcut volume. Area/volume bounds retain spatial uncertainty and are not claimed to meet a depth tolerance in square/cubic millimeters. Point/line cap contacts remain represented by motions and maximum-error checks even though their planar area is zero.

## Rounded coordinates and an exposed planner defect

`--decimal-places N` formats and parses every original XYZ endpoint using [Rust's fixed decimal formatting](https://doc.rust-lang.org/std/fmt/index.html#precision). It reports the coordinate quantum, observed coordinate changes, rounded motion fingerprint, and a complete second verification result. There is no default machine precision.

No zero-length moves are silently dropped. Formatting-induced collapse, lost required Z movement, a nonpositive direction dot product, disconnected moves, changed entry behavior, insufficient clearance, overcut, and finish error prevent acceptance. Original and rounded results remain separate, and the overall result requires both to pass when rounding is requested. This is coordinate verification, not an emitted G-code reader or machine-profile check; those belong to M6.

These checks exposed plunges approximately `1.6e-15` mm deep at medial threshold intersections. The planner now retains such endpoints at stock top when positive depth is smaller than the independent boundary-query arithmetic reserve. Meaningful cutting depths remain, and coarse formatting continues to fail explicitly.

The M4 `contact-line` and `contact-point` examples request zero floor ridge while their guarded cap motions leave about **0.01 mm**. Their M4 previews are complete under the older numerical allowance. M5 cannot accept their strict zero limit: original-coordinate bounds are inconclusive, and six-place rounded coordinates provide a failing cap witness. The fixture expectations record this stronger result; their requested finish settings were not relaxed.

## Regression and release evidence

The M5 core suite adds 16 tests and the CLI adds three. They cover whole-cell enclosures, refinement convergence at 0.1/0.05/0.02 mm, finite-tip detail, a deleted floor lane with repaired motion continuity, lowered paths, unsafe stock links, an island crossing with valid endpoints, translated-boundary rounding gouges, collapsed moves, zero-ridge cap contacts, resource exhaustion, analytic cone slices, deterministic reports, stale settings, and failed-preview replacement. All **127 integration tests** pass in both debug and release on native Windows. Debug/release builds, Clippy with `-D warnings`, and formatting also pass.

The [release fixture manifest](../../flat-v-carve/fixtures/m5/cases.json) has ten cases: six successful original/six-place coordinate checks, two deliberately unsatisfied zero-ridge contacts, one cell-budget exhaustion, and one coarse-rounding rejection. All ten expectations were met on Windows. JSON plans, verification reports, SVG findings, and benchmark output are generated under `flat-v-carve/artifacts/m5/`.

Run the repeatable Windows measurement from the Rust workspace:

```powershell
cargo build --release --workspace --locked
.\scripts\benchmark-m5.ps1
```

Measured on Windows build 26200.9168, PowerShell 7.6.5, Rust 1.95.0, MSVC 14.29.30133. Times are single-run wall times. Verify time includes plan authentication/replay, original and rounded verification, and JSON/SVG writing. Memory is the OS peak working set observed at 20 ms intervals; final/short-lived spikes can be missed. A zero observation is unavailable, not zero memory use.

| Fixture | Motions | Original / rounded cells | Plan seconds | Verify seconds | Observed verify peak MiB | Plan / report bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| narrow-channel | 84 | 11,447 / 11,447 | 0.1734 | 0.0711 | 7.3 | 127,380 / 148,879 |
| wide-floor | 1,923 | 26,673 / 26,673 | 0.8584 | 1.2173 | 18.3 | 1,852,872 / 934,390 |
| island | 6,055 | 88,901 / 88,901 | 5.0343 | 10.7276 | 43.4 | 5,185,344 / 2,247,278 |
| finite-tip | 955 | 51,530 / 51,538 | 0.1995 | 0.7577 | 13.7 | 1,224,021 / 770,046 |
| curved-medial | 879 | 70,946 / 70,978 | 0.1557 | 1.9135 | 11.2 | 981,075 / 622,614 |
| disconnected | 3,934 | 29,867 / 29,867 | 2.3494 | 4.4103 | 28.7 | 3,399,717 / 1,438,542 |

This is a measured fixture envelope, not a latency guarantee for arbitrary artwork. The island case spends substantial time recomputing M4 polygon previews during artifact replay; verification correctness does not depend on caching those previews.

## Scope and remaining limits

Bounds apply to the **rebuilt normalized polygon target**, modeled tools, and recorded linear motions. Source flattening and snapping depth error is reported separately. This does not certify arbitrary SVG topology or supply directed-rounding IEEE interval arithmetic. Repeated preview Boolean errors cannot create an M5 pass because those polygons are not acceptance evidence.

Large/ambiguous geometry can exhaust the explicit budgets. The report preserves unresolved locations and bounds; increasing a budget does not change the requested finish limit. M5 does not establish fixture/holder clearance, physical cutting loads, machine dynamics, macro motions, or G-code correctness. M6 must recheck authenticated plans at the actual output precision and implement the machine-profile/emitted-program contract.
