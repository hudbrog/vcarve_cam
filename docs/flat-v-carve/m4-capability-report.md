# M4 combined V-bit planning capability report

Date: 2026-09-05\
Engine: 0.5.0\
Scope: endmill followed by V-bit planning, continuous segment clearance, recorded combined stock, and sampled finish quality.

M4 implements the combined planner from the [implementation checklist](implementation-plan.md#7-m4-v-bit-finishing-and-rest-machining). It handles broad floors, islands, disconnected regions, rising narrow details, curved medial branches, and finite flat tips. A complete result requires the final achievable boundary/detail family even when earlier cutting already removed its material.

**M4 completion is bounded by its stated checks:** required path families, continuous segment clearance, accessible-floor slice coverage, and quality at the reported samples. Sampled residual maxima are not global error bounds. Adaptive height-field/slice certification remains M5; machine output remains M6.

## Build and test evidence

The pinned Rust 1.95.0 workspace builds in debug and release on Ubuntu 24.04 under WSL2, `x86_64-unknown-linux-gnu`. All **108 integration tests** pass: 12 M0 core, 20 M1 core, 24 M2 core, 16 M3 core, 21 M4 core, and 15 CLI tests. Clippy passes with `-D warnings`, and formatting passes. M4 adds no dependencies. Windows-native and WebAssembly compilation remain unestablished.

The new tests compare tapered sweep area against an analytic cone-hull area, point removal against a stationary-point solution and dense independent reference, and pointed/flat-tip ridges against the straight-lane formula. They check whole-segment clearance across an island, curved XYZ approximation, source radii and cap junctions, empty roughing stages, exact-fit lines/points, a 120° V-bit, and rotated/translated artwork.

Adversarial cases delete floor lanes, prior depth passes, and final finishing; inject invalid coordinates, gouges, feeds, entries, and tool order; exhaust motion/sample budgets; tighten finite-tip detail limits; and corrupt saved identities or cached analysis. CLI tests replay a self-contained artifact after deleting its input job and verify status, exit codes, explicit stage selection, and invalid-preview replacement.

## Candidate geometry and execution

For normalized removal region `P`, boundary clearance `rho`, nominal depth cap `D`, V-bit flat-tip radius `rt`, and half-angle slope `m = tan(theta/2)`, M4 uses:

```text
guard = 2 * geometry_tolerance
guarded full-depth centers = erode(P, rt + D*m + guard)
safe tip depth at c = clamp((rho(c) - rt - guard) / m, 0, D)
```

The full segment Voronoi diagram supplies interior medial branches with source-feature associations and straight/quadratic curve evaluation. Secondary edges and edges whose two source projections coincide are excluded. Source distances must agree with an independent query against the original normalized boundary.

Analytic threshold roots split each branch at radii `rt + guard` and `rt + guard + D*m`. Branch portions deeper than the cap are handled by the full-depth floor/boundary family. Narrow portions retain rising/falling XYZ profiles. Exact cap contact lines remain medial paths; isolated contact points receive explicit plunge candidates. Guarded contact depth may be slightly shallower than the mathematical cap, within the declared numerical budget.

Curved paths use bounded subdivision. The quadratic midpoint gives the exact chord deviation on each parameter interval. Curve reconstruction reserve plus chord error must fit `motion_tolerance * min(m, 1) / 8`; a second continuous clearance test reserves the depth-interpolation budget. Reconstruction errors that cannot meet the requested tolerance and exhausted subdivision budgets produce diagnostics rather than silent coarsening. Tests independently evaluate the retained curves and measure their distance from the executed XYZ segments.

Floor lanes cross the full guarded center region, including separate components and holes. Their spacing is at most both the user's stepover and 80% of the analytic ridge limit:

```text
straight-lane ridge h = max(0, (spacing/2 - rt) / m)
maximum spacing for allowed ridge h = 2 * (rt + h*m)
```

Full-depth boundary loops supplement the floor lanes. A pointed cutter with zero allowed ridge is rejected when positive floor area remains for it to clear. Zero-ridge line/point contacts remain supported. Finite flat tips can clear floor area with overlapping lanes.

The endmill stage runs first and retains its actual moves, even if it is empty, incomplete, or inconclusive. A logical tool-transition marker follows at the common clearance plane. This is a planning record, with no machine tool-change command or macro assumptions.

V-bit profiles execute at successive depth caps ending at `D`. Profile segments crossing a pass depth are split at that crossing. An excursion consists of XY travel at clearance, feed approach to stock top, explicit depth-limited plunges, cutting along the XYZ profile, and vertical retract. V-bit entry requires an explicit `plunge_capable: true`, plunge feed, cutting feed, spindle speed, and maximum stepdown. M4 does not implement V-bit entry ramps. Retractions retrace the occupied cutter column.

Only proven air cutting can be omitted. Each candidate segment's entire V-bit footprint, including its upper flanks, must fit inside an actual prior endmill sweep extending at least to the required depth. This deliberately conservative proof uses one prior sweep per segment. Floor lanes are divided into three collinear sections so an interior section can be omitted while its wall ends remain. The planner does not restrict tool centers to leftover-stock polygons or prune against speculative removals.

After the initial passes, sampled reachable misses and floor-slice gaps provide cleanup centers. Each iteration tries at most 16 centers with explicit depth passes; configured iteration/motion limits bound the work. Remaining gaps become incomplete results. The release fixture set needs no cleanup additions; it establishes completion of the initial families, while deletion tests establish residual detection. Large or difficult artwork can still fail to converge.

The final stage executes every full-depth boundary, rising medial, and contact candidate without air pruning. Replay regenerates the expected family, compares actual excursions with their execution records, checks depth-pass progression, and rejects missing final families or work scheduled after final finishing. Earlier stock removal cannot substitute for this finishing requirement.

## Independent clearance and actual stock

For a linear cutting move, XY center and surface radius `rt + depth*m` both vary linearly. The independent boundary query splits its parameter interval where the nearest point on each original boundary segment changes between an endpoint and its supporting line. In each interval, squared distance minus squared cutter radius is quadratic. Evaluating interval ends and any interior quadratic minimum checks the whole motion, including between valid endpoints. A floating-point reserve is subtracted before accepting clearance. A rising path can pass this check even when testing its entire length with the deepest endpoint radius would incorrectly reject it.

The verifier also checks finite coordinates, common grid range, IDs, tool/operation identity, exact continuity, explicit feeds, clearance travel, entry capability, stepdown, depth cap, cutter height, and stage order. Invalid cutting motions reject the artifact. The endmill stage retains its independent M3 checks.

At depth slice `t`, a V-bit move is clipped to the portion where tip depth `d` reaches `t`. Its cross-section has radius `rt + (d-t)*m`. The swept section is the convex hull of its endpoint disks, including unequal-radius disks and a zero-radius apex. Inscribed/circumscribed circle polygons and snapping reserves bracket each individual sweep. The lower construction trims an apex by a bounded amount instead of dropping the entire taper. The analytic fixture with an apex 10 mm from a radius-2 mm disk checks area `2*sqrt(96) + 4*acos(-0.2)`.

Combined stock unions the lower/upper sweeps of all recorded endmill and V-bit cutting moves that reach the slice. Plunges and ramps contribute their actual occupied material. Approach and clearance motions do not receive removal credit. Each slice retains contributing motion IDs, nominal section, lower/upper removal, remaining target, and possible overcut.

An independent point query maximizes removal along each actual linear XYZ move. It evaluates endpoints, closest approach, flat-tip interval boundaries, and the analytic flank stationary point. It does not estimate maxima by sampling the move parameter. Pointwise combined removal is the maximum of actual endmill and V-bit removal.

Individual sweep polygons have explicit geometric brackets. Repeated polygon Boolean/offset errors and topology are not a formal accumulated interval proof. Continuous center clearance supplies a separate no-gouge check against normalized source geometry; it does not certify the original artwork's approximation or all finish-quality errors between samples.

## Quality and artifact contract

The report keeps nominal target depth, actual removal, permitted floor ridge, unavoidable cutter-limited detail bounds, and missed reachable material separate. Finite-tip capability uses the independent M1 reachability search. If the V-bit tip radius exceeds the endmill radius, endmill capability also contributes. Bounds that straddle a quality criterion or exhaust the search budget produce an inconclusive result.

Quality samples combine a configured cell-center lattice with actual cutting endpoints and midpoints. Exceeding the sample budget does not silently enlarge sample spacing. Reported maxima apply only to these points.

Accessible floor combines both tools' full-depth center regions dilated by their tip radii, including positive-radius contact line/point sweeps. Coverage is checked against actual lower removal at:

```text
floor_check_depth = max(0, D - allowed_floor_ridge - verification_tolerance/2)
missing_floor = erode(accessible_floor, verification_tolerance) - lower_removal
```

The half-tolerance depth budget is explicitly reported separately from the allowed physical ridge. The XY erosion provides the slice comparison tolerance. Verification tolerance must cover eight geometry tolerances in both XY and depth. Consequently, positive nominal-depth remaining area can be permitted ridge material, while the floor-check slice and samples still pass.

| Result | Meaning |
| --- | --- |
| `complete` | Continuous motion checks, required final family, accessible-floor slice coverage, and sampled quality pass. |
| `empty` | Both stages contain no cutting and their checks do not identify remaining required work. |
| `incomplete` | Reachable stock or required final finishing remains, cutter-limited detail exceeds its limit, or entry is unsupported. |
| `inconclusive` | Numerical bounds, center geometry, possible-overcut polygons, or resource limits prevent completion. |
| Rejected input/artifact | Missing or inconsistent settings, impossible zero-ridge area clearing, geometry failures, unsafe motions, or stale identities prevent a valid plan. |

Uncertainty takes precedence over incomplete coverage. Endmill-only incompleteness does not automatically fail the combined plan: the V-bit may finish the leftover stock, as the exact-fit fixture demonstrates. All actual endmill moves must still pass their safety checks.

Job schema **3** adds optional `vbit_planning` and generic tool-slot `plunge_capable`. Schema 1/2 jobs migrate with new fields unset. Endmill slot capability, if supplied, must agree with its existing geometry capability flag. Import invents no machining parameters. Combined planning requires explicit ridge/detail limits and V-bit cutting/entry settings in addition to M3 inputs.

The separate schema-1 `combined_plan` embeds the endmill plan/job, transition, V-bit motions, path execution records, generation issues, and derived analysis. SHA-256 identities bind the engine/job and both motion stages. `inspect`/`verify` validate identities, regenerate geometry, and recompute stock and quality; cached analysis/spindle fields are not trusted. Fingerprints detect stale artifacts, not maliciously authenticated changes. Engine 0.4 plan artifacts must be regenerated with 0.5; job migration remains supported.

## Release fixture results

All **13/13 fixtures** matched expected status and exit code. All 12 produced combined plans were inspected and verified, with recomputed analysis exactly matching the saved analysis. The ten complete cases have zero reported missing accessible floor and zero possible-overcut polygon area at every checked slice.

| Fixture | Combined status | Endmill moves | V-bit moves | Final paths executed/expected | Air executions omitted | Max sampled residual, mm |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| wide-floor | complete | 76 | 1,850 | 5/5 | 63 | 0.114925 |
| island | complete | 866 | 5,189 | 6/6 | 26 | 0.116514 |
| disconnected | complete | 102 | 3,832 | 10/10 | 80 | 0.114925 |
| narrow-channel | complete | 0 | 84 | 5/5 | 0 | 0.010000 |
| finite-tip | complete | 76 | 882 | 5/5 | 28 | 0.156447 |
| ramp-roughing | complete | 89 | 1,850 | 5/5 | 63 | 0.114925 |
| exact-fit | complete | 8 | 538 | 5/5 | 17 | 0.095294 |
| curved-medial | complete | 52 | 833 | 10/10 | 14 | 0.010000 |
| contact-line | complete | 0 | 84 | 5/5 | 0 | 0.010000 |
| contact-point | complete | 0 | 76 | 5/5 | 0 | 0.010000 |
| unsupported-entry | incomplete | 76 | 0 | 0/5 | 0 | 1.500000 |
| resource-limit | inconclusive | 76 | 9 | 0/5 | 0 | 1.500000 |
| zero-ridge | rejected | — | — | — | — | — |

The finite-tip fixture permits 0.5 mm cutter-limited detail. Its maximum sampled reachable shortfall is 0.010000 mm, while the maximum sampled lower bound on unavoidable residual is 0.137389 mm. These statistics have different meanings and must not be added as if they occur at the same point. Tightening the detail allowance to zero fails in the core test.

The curved fixture retains nine medial branches, including two quadratic branches, and joins ten final boundary/detail paths. The island fixture has 2,488 quality samples and a positive independently checked V-bit center-margin lower bound of approximately 0.00002494 mm. The resource-limited plan retains one complete nine-move V-bit excursion, with 41.093 mm² of missing accessible floor and no final finish. Unsupported entry preserves the endmill stock and reports the same missing floor.

On this fixture run, release planning ranged from about 0.035 seconds for the narrow channel to 5.43 seconds for the island; island verification took 4.17 seconds. These are single-run local observations, not production benchmarks. The output polygon validator now skips disjoint edge bounding boxes before exact intersection checks; raw input topology validation and the common 4,096-edge limit remain unchanged.

Local outputs are under `flat-v-carve/artifacts/m4/`: aggregate `report.json`, plus per-fixture `plan.json`, `preview.svg`, inspection `report.json`, and `verification.json`. The rejected zero-ridge case has a failure report instead of a plan. Curved-medial, finite-tip, and island previews were rendered with installed Inkscape and visually inspected. They show retained final paths, preserved islands, combined stock, and finite-tip corner residuals. Generated artifacts are ignored by Git.

## Reproduce and limits

See the [workspace commands](../../flat-v-carve/README.md#m4-combined-finishing-and-rest-machining) and [13 fixture definitions](../../flat-v-carve/fixtures/m4/README.md). Fixtures use explicit synthetic test dimensions, feeds, spindle speeds, and capabilities, not machining defaults or physical trial data.

```sh
cd flat-v-carve
cargo build --release --locked -p cam-app
target/release/cam plan fixtures/m4/curved-medial.json --output artifacts/m4/curved-medial/plan.json
target/release/cam inspect artifacts/m4/curved-medial/plan.json \
  --output artifacts/m4/curved-medial/preview.svg --report artifacts/m4/curved-medial/report.json
target/release/cam verify artifacts/m4/curved-medial/plan.json \
  --output artifacts/m4/curved-medial/verification.json
```

`plan` selects the combined pipeline when `vbit_planning` is configured. `--stage endmill` still produces only M3 roughing; `--stage combined` explicitly requires M4 settings. Complete/empty exits 0; incomplete/inconclusive and rejected inputs exit 1 with diagnostic artifacts; argument/I/O errors exit 2. Inspection `valid: true` means the artifact loaded and was analyzed, so callers must also check `analysis.status`.

Configured limits are 4,096 candidates, 100,000 V-bit motions, 65,536 medial segments, 256 depth passes, eight cleanup iterations, 50,000 quality samples, 100,000 reachability cells per query, and 32 nominal stock slices. These are upper bounds, not recommended working settings. An individual capsule has at most 1,024 circle sides, the common polygon cap is 4,096 edges, and loaded plans are limited to 128 MB. Planning may reject geometry before motion generation when those limits cannot represent it. Motion exhaustion retains complete excursions ending at clearance.

M4 has no path-order optimization, pruning against prior V-bit sweeps, adaptive global stock certification, rounded-coordinate validation, G-code, holder/fixture collision model, or machining trial. Those limitations remain visible in the plan and roadmap.
