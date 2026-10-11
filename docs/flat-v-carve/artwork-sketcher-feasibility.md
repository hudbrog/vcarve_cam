# Artwork sketcher feasibility — 11 October 2026

**Recommendation: proceed with Arael for the next implementation milestone.**
Its Rust/native/WASM integration fits this project better, and the measured
constraint, geometry and reference contracts support that choice. Keep PlaneGCS
as a comparison backend in the experiment. Neither library's success flag is
permission to accept geometry: independent residual and topology checks remain
part of our adapter.

This is an executed feasibility study, not a production editor or a release
qualification. Application code, document schema, production Cargo dependencies
and SVG replacement behavior have not changed.

## Reproducible evidence

- [Experiment and reproduction instructions](../../flat-v-carve/experiments/sketcher-feasibility/README.md).
- [Captured measurements and assertions](../../flat-v-carve/experiments/sketcher-feasibility/results/qualification.json).
- [Neutral fixtures](../../flat-v-carve/experiments/sketcher-feasibility/fixtures.mjs),
  [independent validator](../../flat-v-carve/experiments/sketcher-feasibility/validate.mjs),
  [geometry/provenance prototype](../../flat-v-carve/experiments/sketcher-feasibility/src/geometry.rs).

| Input | Pinned version/source |
| --- | --- |
| Arael and arael-sketch-solver | 0.9.0, commit `18d86e1fda91814a4a42d00001f3e8461c638bb4` |
| PlaneGCS native source | Salusoft wrapper commit `9d26baef0b240fe0eef4186d4e5e5e66ec3620b7`; bundled FreeCAD revision `5f8eac49f31626354ee69bf40616bac801ca5560` |
| PlaneGCS browser binary | Published `@salusoft89/planegcs` 1.3.0; SHA-256 recorded in results |
| Native C++ dependencies | Eigen 3.4.0, Boost 1.86.0, nlohmann/json 3.12.0 |
| Toolchains | Rust 1.95.0; wasm-bindgen 0.2.128; MSVC 19.29.30159, Windows SDK 10.0.19041.0 |

Arael uses the **same pinned Rust source on both targets**. PlaneGCS native was
built from source and browser used the published artifact. Its package version
matches the source checkout, but the npm binary's exact build provenance has not
been independently reproduced. Do not describe that comparison as proof of
identical PlaneGCS source/build settings across targets.

Measured on Windows build 26200, AMD Ryzen 9 8945HS, 16 logical processors,
approximately 61 GiB RAM; actual browser execution used the app's Chromium 144
browser with module Web Workers. This is one machine and one Chromium runtime,
not a browser compatibility matrix or a performance guarantee.

## Solver results

31 cases ran per backend/target, except the recorded PlaneGCS browser allocation
failure. The set has 28 intended-valid cases, a contradictory case, a degenerate
case and an unrefined Arael characterization case. It covers lines, circles,
circular arcs, fixed geometry, coincidence, joins, horizontal/vertical,
parallel/perpendicular, equality, tangency, distances, angle, midpoint,
point-on-line/circle and symmetry about a line. It also covers underconstraint,
small/large dimensions, difficult initial values, dimension edits and dragging.

| Contract | Arael | PlaneGCS |
| --- | --- | --- |
| Valid fixtures | 28/28 on native and browser with our refinement step | 28/28 native; 27/27 executed browser cases, one OOM |
| Independent residual limit | `1e-6 mm` for tested distances and length-scaled angular errors | Same |
| Expected DOF | Pass | Pass on executed cases |
| Native/browser determined solution parity | Maximum tested parameter delta `5.7e-14` | Zero on executed determined cases |
| Contradictory dimensions | Independent check rejects; backend can report `Converged` | Independent check rejects; solver also reports failure and conflicting tags |
| Redundant dimensions | Correct DOF; bounded deletion probes identify alternatives | Correct DOF and native redundant tags |
| Degenerate line | Aborted/non-finite derivative diagnostic; rejected | Failed solve/collapsed geometry; rejected |
| Drag with mandatory 10 mm length | Length preserved; small cursor lag; free DOF restored on release | Length preserved; free DOF restored on release |
| Browser cancellation | Worker terminated, UI timer continued | Same |
| Native cancellation | Helper process terminated | Same |

The 1 nm residual limit is a strict numerical probe, not a promised machining
accuracy. The current import tolerance in the geometry fixture is 0.001 mm.
Screen snapping, solver residuals, topology joins and CAM flattening must remain
separate tolerances.

### Arael settings and diagnostics

Default point-to-point coincidence left a `7.21e-6 mm` residual while reporting
convergence. Reducing drift weight alone did not meet the strict limit and also
made a different underconstrained distance case less robust. Repeating the
high-level solve did not help because its cost threshold allowed an early exit.

The adapter retains upstream drift defaults, then runs a **maximum 16-iteration
refinement through public `lm_solve`**, with cost-threshold early exit disabled,
after updating derived state. It needs no upstream patch. Coincidence then
measures `7.21e-12 mm` on both targets. Cold solve timings include this refinement;
warm dimension and drag timings use the upstream warm session. Every resulting
state is independently checked. Production should refine only when necessary,
with explicit residual, iteration, time and cancellation policy; failure keeps
the draft, never silently accepts a best-effort shape.

The contradictory-distance fixture is especially useful: numeric convergence
can still leave a 5 mm constraint error. DOF measures local freedom, not validity.
Arael's rank/probe/blocker APIs are promising, but a stable-ID diagnostic adapter
has not been completed. The experiment tests explicit single-constraint deletion
probes for the two small redundant/conflicting examples; it does **not** establish
a general minimal conflict set or prove inconsistency from non-convergence.
No user constraint is deleted automatically.

## Timing and resources

Warm p95 in milliseconds, 21 successive dimension edits per size:

| Fixture | Arael native | Arael browser | PlaneGCS native | PlaneGCS browser |
| --- | ---: | ---: | ---: | ---: |
| 25 entities / 55 constraints | 0.033 | 0.5 | 0.094 | 0.2 |
| 100 entities / 220 constraints | 0.162 | 0.7 | 0.587 | 0.6 |
| 500 entities / 1100 constraints | 0.343 | 1.2 | 2.697 | OOM |
| Connected 100 lines / 299 constraint blocks | 0.135 | 0.7 | 1.620 | 2.6 |

The first three cases are repeated **disconnected** constrained rectangles and
circles. The connected case is an anchored staircase of lines with joins,
orientation and length constraints. Neither represents all complicated sketch
graphs. A join contributes two scalar equations, so constraint-block counts are
not matrix row counts. Timings qualify these adapters and settings, not the
maximum possible performance of either solver.

Both meet the provisional 50 ms warm p95 target at 100 entities. The 21-step
constrained drag case measures 0.042/0.7 ms native/browser for Arael and
0.019/0.2 ms for PlaneGCS. Arael's maximum direction lag is below 0.001 rad;
mandatory length residual remains below the probe threshold.

Browser cases run in fresh workers to isolate fatal allocation failures. First
solves therefore include cold WASM execution/JIT effects: connected-100 is about
39.7 ms Arael and 24.6 ms PlaneGCS, excluding worker/module startup. Median local
worker/module-ready time is about 45.7 and 24.7 ms respectively. These are local
loads with warm OS/browser caches, not Internet download measurements.

Arael's largest observed WASM linear memory is 5,570,560 bytes. PlaneGCS's tested
artifact has 22,085,632 bytes and aborts at 500 entities even in a fresh worker.
Its checked-in linker settings do not enable memory growth. This is a failure
of the **tested browser package/configuration**, not proof that PlaneGCS cannot
support 500 entities. A rebuild with controlled memory growth/limits is the next
PlaneGCS action if we retain it as a production candidate.

The Arael probe WASM is 4,294,984 bytes and includes the Rust fixture harness,
CAM importer and geometry tests; the PlaneGCS solver-only WASM is 484,003 bytes.
These are uncompressed, unoptimized-for-size artifacts with different contents.
They are **not** a fair estimate of incremental production download size. Native
peak RSS, transport serialization cost and sustained editing memory growth have
not been qualified. The result file records measured linear-memory allocations,
not total browser process memory.

Expensive diagnostics belong off the drawing path. The native PlaneGCS probe's
explicit additional `diagnose()` reaches roughly 665 ms for the 500-entity case.
Browser diagnostic timing measures exposed getters, not an equivalent additional
full diagnosis; the work can occur inside the solve. Do not compare those two
diagnostic columns as equivalent operations.

Browser termination took about 25–27 ms with five UI timer ticks. Deliberately
delayed real worker replies arrive out of order; the old request is rejected,
and document/sketch revision guards are checked separately. Native helper
processes were killed after a 75 ms timer (about 92 ms including startup/exit).
This does not prove cooperative interruption of an in-process native Rust solve.

## Geometry adapter and selection preservation

The prototype uses the **existing CAM importer, Region and ContourCatalogue**.
It imports explicit faces and named wires separately, then assembles one typed
catalogue with persistent identities. It does not add another flattening or
polygon normalization implementation.

Tests pass identically in native Rust and the browser:

- A plate with two unequal circular holes and a nested island produces two
  filled components, four closed contours, five centerlines and two exact drill
  centers. Construction geometry and derived centroid markers are excluded.
- The filled region has zero XOR area against an equivalent compound SVG;
  independent expected area and open line/arc endpoints are checked. Existing
  flattening and source-snap bounds are retained.
- Negative local coordinates and translated/rotated/scaled placement preserve
  analytic circle centers, diameters and chain endpoints without a second flip.
- Spatially exchanging the unequal holes demonstrates that raw SVG `hole-0`
  changes meaning. Persistent `hole-a` contour/center selections keep their
  intended owner, geometry kind and identity.
- Dimension edits carry region/contour/chain/point references to the new source
  revision. Changed output revisions differ; construction-only edits leave the
  derived geometry revision unchanged.
- Changed edge membership, removed outputs, construction toggles that remove
  output, and changed nesting leave affected selections unresolved. Foreign
  owners and stale input references are rejected. Reference JSON roundtrip and
  restoration against the original snapshot are checked.
- The real `ContourCatalogue::resolve_anchor` accepts an unchanged anchor and
  rejects it after its contour fingerprint changes. A preserved contour selection
  does not silently preserve its old manual start/tab anchor.

**Materialization decision:** use a typed output/provenance layer. Generated SVG
can remain an internal normalization bridge, but emitting one SVG and accepting
its importer IDs is insufficient. Separate passes avoid duplicate closed
boundaries while retaining analytic centers. Centerlines and closed contours are
distinct existing operation views of a boundary, not accidental duplicate IDs.

The page importer rejects geometry outside its viewport. This spike uses a fixed
200 mm envelope representing local `[-100,100]²`, with the offset compensated
inside import options. The user's placement is unchanged. Production must define
the supported local envelope or extract a page-independent normalization entry
point. It must not recenter artwork or resize stock when a sketch changes.

The prototype is intentionally bounded: persistent wire membership and containing
face relationships are supplied explicitly. Automatic wire discovery, nesting
validation, crossings, near-touching cases, general topology-change diagnostics,
empty sketches and invalid draft recovery still need implementation. The adapter
is not yet wired into saved sketch jobs, operation planners or retained-export
freshness. Hash inequality and reference checks are demonstrated; an actual
production apply/undo/replan/export transaction remains a later integration gate.

## Integration decision and remaining gates

Arael's [solver package](https://github.com/harakas/arael/tree/18d86e1fda91814a4a42d00001f3e8461c638bb4/arael-sketch-solver)
is Rust and declares MIT licensing. Its solver does not require egui. The example
editor uses eframe 0.31 while this project uses 0.33, so reuse the numerical
package and interaction ideas rather than transplanting its editor wholesale.
The experiment builds the Rust sparse backend without an application FFI layer.

The [PlaneGCS source](https://github.com/Salusoft89/planegcs/tree/9d26baef0b240fe0eef4186d4e5e5e66ec3620b7/planegcs)
introduces C++/Eigen/Boost plus a separate browser module. Its source headers say
LGPL-2.1-or-later; npm metadata says LGPL-2.0-or-later. Record exact file notices
and resolve distribution packaging before adoption. The experiment's native
helper process avoids introducing unsafe Rust FFI into application code.

For the next milestone:

1. Build a small solver-independent saved model and an Arael adapter pinned to
   this qualified source. Keep backend handles and Arael serialization out of
   the job format. Persist accepted coordinates and validate them on reopen.
2. Turn residual acceptance, stable constraint IDs, refinement, branch continuity
   and diagnosis into an explicit adapter contract. Extend to mixed constraints,
   arc endpoint tangency, angle wrap boundaries, structural edits and long drags.
3. Implement shared typed materialization with verified topology/provenance.
   Route catalogue, selected-region resolution and bounds through that boundary.
4. Implement the atomic apply command: provenance-based reference updates,
   unresolved-anchor reporting, undo, and actual retained-plan invalidation.
5. Qualify native in-process cancellation, production worker reuse/coalescing,
   remaining browsers, release size and sustained resource limits before release.

These are implementation gates for the selected direction. The measurements
give no reason to accept PlaneGCS's extra integration cost solely for closer
FreeCAD behavior.
