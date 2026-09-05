# Real artwork and scalability — engine 0.7.2

The immediate import failure is fixed. The unchanged `real_data/flower_box.svg` now imports at 0.005 mm geometry tolerance and completes M4 combined planning. Import has been measured through 100 copies (994,300 boundary vertices). This is not evidence that complete CAM is ready for artwork 100 times more complex: motion count, continuous verification, and preview/report size remain separate constraints.

## Reproducer and precision

Source: `real_data/flower_box.svg`, 27,172 bytes, SHA-256 `a7c41b2a53056ef10286662ac79007bf524d8b87c7d03c3718280b19ec534633`. It contains one compound filled path on a 210 × 297 mm page. Import resolves 15 selected components and 9,943 boundary vertices, with total area 5,361.846017305 mm². Artwork bounds are X 4.9174–175.994 mm, Y 205.5868–288.3212 mm in the existing lower-left page coordinate convention.

The original 4,096-vertex limit rejected this file before planning. After increasing capacity, tiny closing segments also exposed a blanket prohibition on grid-coincident consecutive vertices. These are now coalesced only after checking raw and snapped edge relationships. Coalescence is reported, remains within the existing grid-snap bound, and cannot erase a ring, create a nonlocal contact, or change a proper crossing. A contracted attached micro-loop is also rejected. No source paths were edited or simplified to fit a lower detail level.

At the chosen settings the source flattening bound is 0.00125 mm, the source-grid snap bound is approximately 0.00003536 mm, and the work-grid snap bound is approximately 0.00007071 mm. These conversion bounds remain separate from motion and stock verification tolerances.

## Implemented changes

- A deterministic bounding-volume hierarchy selects candidates for raw/snapped topology checks, normalized-boundary validation, containment, nearest distances, and continuous motion clearance. Exact intersection predicates and analytic narrow-phase formulas remain authoritative. A positive sweep margin may now be conservatively capped at 1 mm when farther edges are excluded; this does not weaken a clearance acceptance test.
- SVG selection uses one polygon union, component provenance uses indexed overlap candidates followed by exact intersections, hole assignment is indexed by parent, and selection IDs use ordered-set membership.
- Stock slices collect cutter footprints in bounded batches and merge balanced intermediate unions, replacing a full accumulated-stock union after every motion.
- Repeated quality samples use cached cutter-footprint indexes and the existing analytic endmill/V-bit point-removal formulas. Indexed queries are compared against full scans around endpoints, flanks, flat tips, and translated coordinates.
- Native limits are 32 MB SVG, 200,000 XML nodes, two million flattened/boundary vertices, and 64 MB job JSON. Separate guards bound dense topology candidate pairs and potential fill-arrangement growth. Individual ellipse and open-polyline limits remain smaller; this is not an unlimited geometry API.
- Explicit V-bit job budgets may reach 65,536 paths and one million motions, curve segments, or quality samples. Saved fixture settings are unchanged; exhausted budgets still produce a diagnostic or an inconclusive plan.
- Plan files use compact JSON and omit derived analysis caches. Inspection reconstructs reports from authenticated motions. The CLI identifies artifact type without allocating an extra complete JSON tree. Saving refuses plans larger than the existing 128 MB reload limit. The browser fixture service retains its smaller file limit until native integration.

Job schema 3 and plan schema 1 remain in use. Engine 0.7.2 invalidates older plans, as required when geometry generation changes. Saved jobs remain usable.

## Native Windows import measurements

Windows x64, pinned Rust 1.95.0 MSVC, release build, sequential runs. Each case repeats the real path at unchanged physical size and 0.005 mm tolerance on separate page tiles. Timings cover `import_svg`; memory is process peak working set observed by the Windows process API. These are single-run measurements, not percentile guarantees.

| Copies | Components | Boundary vertices | Import seconds | Peak working set |
| --- | ---: | ---: | ---: | ---: |
| 1 | 15 | 9,943 | 0.063 | 6.5 MB |
| 10 | 150 | 99,430 | 0.614 | 37.3 MB |
| 100 | 1,500 | 994,300 | 5.994 | 307.3 MB |

The harness checks replicated component counts and area (relative comparison tolerance 0.000001), and records the source hash. Separated copies exercise high input volume, component selection, and geometry normalization. They do not exercise one densely connected million-edge shape or arbitrary overlapping arrangements. A separate 12,000-vertex single-boundary regression covers connected input above the old limit.

From `flat-v-carve`, with PowerShell 7 and a fresh output directory:

```powershell
cargo build --release --locked --workspace --examples --bins
./scripts/benchmark-import.ps1 -OutputDirectory artifacts/import-scalability-new
```

Recorded JSON: `flat-v-carve/artifacts/import-scalability-0.7.2/summary.json`.

## Flower pipeline result

The run uses the earlier illustrative tool setup: 8 mm stock, 2 mm depth cap, 4 mm endmill, pointed 90° V-bit, 1 mm stepdown, zero wall allowance, 0.15 mm allowed floor ridge, 0.005 mm motion tolerance, and 0.05 mm verification tolerance. Tool feeds/spindle settings are inherited test settings. Budgets are recorded in `artifacts/flower-box-scalability/complete-budget.job.json`, including 500,000 V-bit motions, 32,768 candidate paths, 250,000 medial segments, and 250,000 quality samples.

| Measurement | Result |
| --- | ---: |
| Standalone endmill planning | 23.012 s |
| Combined planning, including endmill | 113.224 s |
| Endmill motions | 16,361 |
| V-bit motions | 280,261 |
| Required/executed final paths | 13,504 / 13,504 |
| Medial branches | 13,475 |
| Quality samples | 103,965 |
| Maximum sampled reachable shortfall | 0.072885 mm |
| Missing combined accessible floor | 0 mm² |
| Compact saved plan | 82,325,842 bytes |

M4 combined status is **complete**, with no generation issues. The endmill alone reports 0.268141 mm² of missing floor at the bottom layer; combined V-bit finishing satisfies the M4 floor and sampled-quality checks. M4 is not a continuous stock certificate. The complete saved plan was reopened through `cam inspect`, which authenticated its fingerprints and regenerated the same complete status.

The earlier 250,000-motion run stopped partway through the final family and correctly reported inconclusive. Its old, pretty-printed plan with geometry caches was 271,980,358 bytes, too large to reopen. The complete run now fits the existing reload limit; the file-size comparison includes different motion counts and is not a same-plan compression ratio.

Most V-bit motions are associated with short medial paths: 148,504 in depth passes and 75,374 in final medial finishing. Each branch currently gets its own excursion, including approach, plunge, and retract. Boundary finishing has 29 paths. Removing tiny artwork features or skipping required final branches merely to reduce these counts would change the result; the next work should improve traversal and prove any omitted cutting redundant.

Useful local artifacts:

- `artifacts/flower-box-scalability/artwork.svg`: normalized imported artwork.
- `artifacts/flower-box-scalability/complete-budget.summary.json`: compact timing, path-family, and coverage measurements.
- `artifacts/flower-box-scalability/complete-budget.plan.json`: authenticated, reloadable combined plan.
- `artifacts/flower-box-scalability/toolpaths.svg` and `inspection.json`: regenerated path, stock, and sampled-quality evidence. The current SVG is about 54.7 MB; it exposes a remaining preview scaling problem.

The benchmark example returns success when it records a plan, including a partial plan; inspect its `combined_status` and diagnostics. Production `cam plan` retains the documented completion-dependent exit code.

The authenticated M5 trial used 50,000 adaptive cells, 512 depth bands, and original coordinates. It returned **inconclusive** after 49,999 evaluated cells, with 18 unresolved cells and `M5_DEPTH_BAND_LIMIT`. The overcut upper bound is approximately 5.25 × 10⁻¹⁰ mm, but reachable-residual and floor-ridge upper bounds remain 2 mm in unprocessed areas, so finish quality is not certified. Rounded output has not been checked and no flower G-code was generated. Full findings and a compact extract are in `verification-50000.json` and `verification-summary.json`; `verification-50000.svg` locates unresolved areas. The full JSON report is about 134.4 MB because it still includes detailed M4 evidence, reinforcing the report-size follow-up.

## Regression checks

All 153 Rust tests pass in both native Windows debug and release builds, including new high-vertex/component, safe-coalescence, spatial-query, batched-union, and cached-stock regressions. Strict Clippy and formatting checks pass. Compact plans are authenticated and their reports regenerated in CLI and core replay tests; injecting cached analysis cannot change acceptance. All eight M6 fixture expectations and saved-byte readbacks also pass, retaining expected failed/inconclusive cases without G-code publication.

## Remaining scale work

1. Connect compatible medial branches into continuous traversals with verified links, preserve required depth passes/final finish, and measure entry count and total cutting/travel distance. Investigate tessellation-induced branches using explicit geometric error bounds rather than arbitrary small-feature deletion.
2. Partition independent components for planning/stock work and reuse immutable target/stock data across generation, authentication, and inspection. The current end-to-end process rebuilds several stages.
3. Measure and accelerate M5 continuous verification on real curved artwork; retain whole-cell bounds and explicit inconclusive outcomes. Import benchmarks do not cover this stage.
4. Replace one SVG element per motion/sample with batched drawing and selectable levels of display detail. Store shared candidate geometry once rather than repeating it in every execution record; provide streaming reports/artifacts before raising their byte ceilings again.
5. Establish separate 10× and 100× end-to-end acceptance cases, including a highly connected artwork case. Record peak memory, time by stage, cut/rapid/entry counts, artifact sizes, and original/formatted-coordinate verification results. Match analytical and adversarial correctness fixtures before accepting speed improvements.

The measured import scaling is approximately proportional to input size over these cases. Full CAM at 10×–100× complexity remains an explicit engineering target, not a completed capability claim.
