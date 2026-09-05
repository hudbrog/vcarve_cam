# M3 endmill planning capability report

Date: 2026-09-05  
Engine: 0.4.0  
Scope: endmill stage, continuous segment clearance, and stock comparisons at planned depth slices.

M3 implements both conservative deepest-region clearing and depth-dependent clearing of the M1 target from M2 SVG jobs. The [implementation checklist](implementation-plan.md#6-m3-endmill-planner) is complete for this stage. V-bit finishing/rest machining, adaptive combined-volume verification, and LinuxCNC output remain M4–M6 work.

## Build and test evidence

The pinned Rust 1.95.0 workspace builds on `x86_64-unknown-linux-gnu`, Ubuntu 24.04 under WSL2. All **84 integration tests** pass: 12 M0 core, 20 M1 core, 24 M2 core, 16 M3 core, and 12 CLI tests across the milestones. Clippy passes with `-D warnings`, and formatting passes. Windows-native and WebAssembly compilation have not been established.

The existing pinned geometry dependencies remain unchanged. M3 adds [`sha2` 0.10.9](https://docs.rs/sha2/0.10.9/sha2/) for deterministic SHA-256 artifact identities and enables `serde_json`'s exact float round-trip feature. Fingerprints detect stale or accidentally edited artifacts; they are not signatures or an authentication mechanism.

Core tests cover analytic capsule area bounds and ramp removal, final depth including a nonintegral last step, rotation/translation, the two clearing strategies, holes/disconnected regions, supported ramps, empty stages, exact-fit contacts, missing numerical margin, unsupported entries, resource exhaustion, deleted reachable clearing, wrong feeds, nonfinite/disconnected motions, in-stock rapids, wall-allowance violations, and an island crossing with individually valid endpoints. Saved-plan tests corrupt cached analysis, settings, motions, generation issues, and engine versions. CLI tests exercise planning, preview/report replay after deleting the original job file, failure invalidation, and exit codes.

## Actual planning contract

With normalized removal region `P`, target half-angle slope `m = tan(theta/2)`, endmill radius `R`, allowance `a`, and layer depth `d`, admissible centers are:

```text
C(d) = erode(P, d*m + R + a)
```

The ordinary strategy evaluates `C(d)` at each layer. The conservative strategy evaluates `C(D)` for every layer, intentionally leaving more upper stock for the V-bit. Layers end at `min(i*stepdown, D)`, including an exact final pass at `D`.

The first loop region is eroded by an additional `2*geometry_tolerance_mm`. Subsequent contours are offsets of that guarded region by integer multiples of stepover, preserving outer and hole boundaries. Stepover must exceed four geometry tolerances and be at most `R`. This first implementation uses separate closed loops, with no travel optimization or speculative in-stock links. A loop whose entire segment clearance cannot be established is skipped with a diagnostic; stock comparison determines whether requested floor remains.

For each candidate cutting segment, the verifier independently minimizes its Euclidean distance to all original normalized boundary segments. It checks interior classification as well as distance, subtracts a floating-point reserve, and requires clearance of at least `d*m + R + a`. Using the deepest endpoint of a linear ramp is conservative for every point along that ramp. The verifier does not rely on the polygon offset engine's assertion of feasibility. It also validates identities, continuity, layer ordering/depth, explicit feeds, motion type rules, coordinate range, and initial/final clearance.

Direct plunges require `plunge_capable: true` in the endmill dimensions and an explicit plunge feed. They descend in increments no larger than the configured stepdown. Ramps require `ramp_capable: true` and explicit entry angle/feed; they alternate along the longest loop edge with each traverse dropping no more than half the stepdown and no more than the requested angle allows. The half-step limit bounds the accumulated depth change when a zigzag revisits a point. Every entry begins with a feed approach from clearance to stock top. Each loop ends with a vertical retract, then XY travel at clearance to the next loop.

There are no tool changes, in-stock horizontal links, V-bit cutting moves, postprocessing, or G-code in M3. Retractions retrace the already occupied vertical cutter column. The model assumes flat stock, the configured clearance plane, and the supplied cutter capabilities; cutting loads, holders, fixtures, and machine configuration are outside this stage.

## Stock from recorded moves

The motion list is the stock model's source. At a queried positive depth `t`, each actual cutting move is clipped to the interval where its tip reaches `t`. Its endmill cross-section sweeps a capsule along that clipped XY segment. This includes ramps and plunges; a plunge contributes a disk, and a ramp contributes only the part that reaches the slice. Feed approach above stock, clearance XY travel, and retracts do not add material removal.

For an individual capsule, the lower polygon uses convex hulls of inscribed endpoint circles of radius `R - 2*snap_bound`. The upper polygon uses circumscribed circles of radius `(R + 2*snap_bound)/cos(pi/n)`. Integer convex-hull orientation is exact within the common coordinate range. The reported radial error includes polygon and snapping terms. Tests bracket the independent analytic area `pi*R² + 2*R*segment_length` for a disk and a diagonal segment.

The slice report unions those polygons over **all recorded cutting moves reaching the slice**, including later, deeper passes. Its fields are:

| Field | Meaning |
| --- | --- |
| `nominal_section` | `erode(P, t*m)`, independent of cutter and allowance. |
| `requested_centers` | The strategy's admissible center area. |
| `accessible_floor` | Requested center area dilated by the endmill radius. |
| `removal.lower` / `removal.upper` | Accumulated capsule approximations from the recorded sequence. |
| `remaining_target` | Nominal section minus lower removal; includes slopes, allowance, and endmill-inaccessible detail. |
| `missing_floor_beyond_tolerance` | Accessible floor eroded by the XY coverage tolerance, minus lower removal. |
| `possible_overcut` | Upper removal minus nominal section. |
| `contributing_motion_ids` | Actual moves whose tip reaches this slice. |

`removed_depth_at` independently solves the interval in which a point lies inside the moving XY disk and maximizes the linear tip depth over that interval. For a ramp from `(0,0,0)` to `(10,0,-2)` with radius 1 mm, it returns 1.2 mm at `(5,0)`, 1 mm at the tangent point `(5,1)`, and zero at `(5,1.01)`. Slice clipping at 1 mm begins at `(5,0)`, rather than incorrectly crediting the entire entry traverse with full-depth removal.

The individual capsule construction has an explicit geometric bracket. Repeated Clipper Boolean operations and offset topology are engineering approximations on the shared integer grid; M3 does **not** turn those into a formal accumulated interval proof. The whole-segment clearance check supplies a separate continuous no-gouge test against normalized geometry. The report does not imply adaptive volume/depth certification between slices, guaranteed detection of every subgrid feature, or completed finish-quality analysis.

## Status and editable inputs

| Status | Interpretation |
| --- | --- |
| `complete` | Continuous motion checks pass and the requested endmill-accessible floor at every planned slice is covered within the declared XY tolerance. |
| `empty` | No endmill center access at any planned layer; target stock remains available to the future V-bit stage. |
| `incomplete` | Requested floor is missing, an entry is unsupported, or a loop was rejected. Actual available moves and stock remain inspectable. |
| `inconclusive` | Exact-fit contacts, insufficient numerical margin, unresolved center geometry, resource exhaustion, or sweep-overcut uncertainty prevents a completion claim. |

Malformed or unsafe moves are rejected with an error instead of becoming a valid plan with a warning. Geometry/backend failures also return explicit errors; no fallback silently marks work complete. Multiple problems retain their diagnostics, and uncertainty takes precedence over a missing-coverage status. Generation issues survive saving/reloading and cannot be erased by an apparently successful cached analysis.

Job schema 2 adds `endmill_planning` and optional per-tool `ramp_capable`. Schema 1 migrates to schema 2 with new fields unset. Import still invents no machining settings. M3 requires stock thickness, maximum depth, horizontal allowance, endmill geometry/feed/spindle/stepdown/stepover, V-bit geometry defining the target angle, geometry/motion/verification tolerances, and explicit start XY, clearance, strategy, entry, and limits. Finite cutter lengths/heights and stock depth remain validated. V-bit cutting settings and final ridge/detail limits can remain unset for M4.

Motion tolerance must cover the offset arc and snapping budgets. The M3 slice coverage tolerance must be at least eight geometry tolerances; it is an XY comparison tolerance, not the adaptive depth/volume uncertainty contract planned for M5. Precision problems request refinement rather than silently changing the requested tolerance.

A separate schema-1 `endmill_plan` artifact embeds the job, engine version, spindle speed, motions, generation issues, and derived analysis. SHA-256 identities bind the engine/geometry adapter contract, job, moves, and generation issues. `inspect` and `verify` reject incompatible or edited identities, rebuild geometry, check the actual moves, and recompute the stock report. Serialized analysis and spindle fields are not trusted. Full float round-tripping preserves the motion coordinates used by the fingerprint.


## Release fixture results

The release binary generated, inspected, and verified all **10/10** fixtures with the expected status and exit code. Recomputed verification matched the plan analysis in every case. The complete cases have zero reported missing floor beyond tolerance and zero possible-overcut area at both slices.

| Fixture | Status | Motions | Lower removal at final depth, mm² | Remaining target at final depth, mm² |
| --- | --- | ---: | ---: | ---: |
| rectangle | complete | 76 | 370.717 | 45.283 |
| island | complete | 866 | 650.083 | 93.361 |
| disconnected | complete | 102 | 322.022 | 61.978 |
| ramp | complete | 89 | 370.717 | 45.283 |
| deepest-region | complete | 68 | 370.717 | 45.283 |
| no-access | empty | 0 | 0.000 | 0.000 |
| exact-fit | inconclusive | 8 | 0.000 | 64.000 |
| narrow-margin | inconclusive | 0 | 0.000 | 80.160 |
| unsupported-entry | incomplete | 0 | 0.000 | 416.000 |
| resource-limit | inconclusive | 8 | 0.000 | 416.000 |

At the 1 mm slice, depth-dependent rectangle clearing removes 454.632 mm², versus 370.717 mm² for deepest-region clearing. Both remove 370.717 mm² at the final 2 mm slice. The island fixture leaves its preserved hole untouched and has a minimum independently checked center margin of 0.008700 mm beyond nominal clearance plus the requested allowance. These are numerical fixture measurements, not machining observations.

The resource-limited case retains one complete excursion (8 moves), with 170.473 mm² missing at the first layer and no removal at the final layer. The zero-margin exact-fit case clears its shallow layer and leaves the final layer unresolved. Unsupported entry retains no cutting moves. These outcomes persist through plan inspection and verification.

Local reproducible outputs are under `flat-v-carve/artifacts/m3/`: aggregate `report.json`, plus each fixture’s `plan.json`, `preview.svg`, `report.json`, and `verification.json`. The island SVG was also rasterized with installed Inkscape for visual inspection.

## Reproduce and inspect

Run the commands in the [workspace README](../../flat-v-carve/README.md#m3-endmill-planning-and-stock) or [fixture README](../../flat-v-carve/fixtures/m3/README.md). The ten M3 fixtures use explicit **synthetic test settings**, including feeds and spindle speed. They do not establish machining defaults.

`plan job.json --output plan.json` writes the stage artifact. `inspect plan.json --output preview.svg --report report.json` recomputes its analysis and shows paths, removed stock, and remaining target. `verify plan.json --output verification.json` writes the recomputed report without a preview. Complete/empty stages exit 0; incomplete/inconclusive stages retain output and exit 1. Invalid inputs write failure diagnostics; argument/I/O errors exit 2. `valid: true` in an inspection means the artifact loaded and was analyzed; callers must still inspect `analysis.status` and the exit code.

Preview panels show blue cuts, orange entries, dashed clearance links, green removed stock, amber remaining target, pink missed floor, and purple possible overcut. Motion elements have IDs, kinds, and Z endpoints in SVG titles. Previews display the first 16 slices; JSON retains all slices. Generated artifacts are ignored by Git and can be reproduced from committed fixtures.

Resource limits are explicitly configured within 1–256 layers, 1–1,024 loops per layer, and 1–100,000 motions. A capsule is limited to 1,024 circle sides, and the existing 4,096-edge polygon limit remains. Plans are limited to 128 MB on load. Exhausted planning budgets retain only complete excursions that end at clearance and mark the result inconclusive. Geometry/stock construction failures reject the plan. Performance on large production artwork remains unestablished; all reported evidence is from the bounded fixture/test workload.
