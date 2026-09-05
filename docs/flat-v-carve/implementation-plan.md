# Flat V-carve CAM: implementation plan

Date: 2026-09-05\
Status: M0–M4 complete on the initial Linux x86-64 target; M5–M8 planned.

Read [architecture](architecture.md) for agreed scope and [technical design](technical-design.md) for geometry and contracts. This plan orders work by uncertainty: establish the geometric foundation before investing in application polish or relying on machine output.

## 1. Delivery rules

- Keep the first implementation in a dedicated CAM project, separate from the existing website in this workspace.
- Build a Rust library and CLI first; the browser uses the same planning functions.
- Deliver inspectable geometry artifacts at each stage, including failing examples.
- Prefer a simple complete strategy over optimization that hides missing coverage.
- Treat library selection and verification methods as hypotheses until the fixtures pass.
- Use actual machine/tool parameters for the physical trial; do not invent feeds, tool lengths, or macro behavior.
- Update this document with evidence when a milestone completes. Unchecked boxes represent future work.

No calendar estimate is assigned yet. The dependency and geometry spike should establish baseline performance and expose the effort required for reliable rest machining.

## 2. Milestone summary

| ID | Deliverable | Depends on | Exit evidence |
| --- | --- | --- | --- |
| M0 ✓ | Rust dependency and geometry spike | None | [Completed capability report](m0-capability-report.md): native debug/release builds, 28 fixtures, 14 tests, documented precision behavior. |
| M1 ✓ | Target model, cutter models, and debug preview | M0 | [Completed capability report](m1-capability-report.md): 37 tests, 8 procedural previews, analytic dimensions/depths, finite-tip bounds, and exact-fit contacts. |
| M2 ✓ | SVG import and versioned jobs | M1 | [Completed capability report](m2-capability-report.md): 65 tests, native/plain Inkscape exports, portable jobs, dimensions/holes/selection preserved. |
| M3 ✓ | Endmill planner and recorded stock removal | M1; integrate M2 | [Completed capability report](m3-capability-report.md): 84 tests, 10 release fixtures, continuous clearance, and actual endmill stock. |
| M4 ✓ | V-bit paths and combined rest machining | M3 | [Completed capability report](m4-capability-report.md): 108 tests, 13 release fixtures, curved/rising detail, floor ridges, and retained final finishing. |
| M5 | Verification of continuous and rounded motions | M4 | Bounded overcut/residual checks and explicit inconclusive cases. |
| M6 | LinuxCNC postprocessor and machine-profile contract | M5 | Emitted-subset checks and LinuxCNC preview/simulation. |
| M7 | Local browser workflow | M2 and stable planning contracts; integrate M6 | Import-to-export parity with CLI and responsive planning. |
| M8 | Measured machining trial and usable release | M6, M7 | Test carving, measured deviations, reproducible installation and documented limits. |

Verification is developed alongside each planner. M5 completes and challenges it; it is not the first time paths are checked. M8 can start with CLI output while browser integration finishes, but release completion requires both.

## 3. M0: dependency and geometry spike

- [x] Create the isolated Cargo project with a small headless geometry executable.
- [x] Evaluate `clipper2-rust` and `boostvoronoi` through application-owned adapters.
- [x] Build the selected dependency features on the initial native target; avoid unrelated GUI/example dependencies.
- [x] Pin crate/toolchain versions after the build succeeds and record the tested OS/architecture.
- [x] Exercise Boolean operations, Euclidean inward offsets, holes, splitting regions, and vanishing offsets.
- [x] Construct segment Voronoi diagrams with straight and curved edges and recover source-feature associations.
- [x] Check shared endpoints, repeated vertices, collinear edges, tiny segments, and quantization-induced intersections.
- [x] Establish integer range/scale limits and a bounded curve-evaluation method.
- [x] Produce JSON/debug SVG artifacts and a concise capability report.

Completed 2026-09-05 in [`flat-v-carve/`](../../flat-v-carve/README.md). Rust 1.95.0, `clipper2-rust` 1.1.0, and `boostvoronoi` 0.12.1 passed on Ubuntu 24.04.4/WSL2, `x86_64-unknown-linux-gnu`. Evidence, analytic formulas, measured errors, precision/resource limits, and repro commands are in the [M0 capability report](m0-capability-report.md). The full Voronoi diagram and positive-area offsets supplied the foundation for M1's target/tool models and exact-fit line/point handling.

**Exit:** no project-specific C++ is required; the selected APIs support all required primitives; fixture errors are measured against analytic references or independently evaluated distances. Failures have minimal reproducers. If a dependency cannot meet a requirement, resolve that finding before building the planner around it.

## 4. M1: target and tool geometry

- [x] Implement the depth convention and nominal target `T(x)`/depth cross-sections.
- [x] Implement endmill and finite-tip V-bit models with dimension validation.
- [x] Implement admissible-center regions at a requested depth, including horizontal allowance.
- [x] Implement distance queries independent of offset construction for cross-checking.
- [x] Produce plan-view overlays and cross-sections for procedural shapes.
- [x] Show nominal target separately from cutter-unreachable features.
- [x] Handle valid center sets that collapse to a line or point, and reject incompatible cutting height.
- [x] Validate parameter changes without relying on a browser interface.

Completed 2026-09-05 with engine 0.2.0. Debug tests, release build, Clippy, and formatting pass; the release CLI passes all 28 M0 fixtures and all 8 M1 models. The [M1 capability report](m1-capability-report.md) records the 37-test suite, exact-fit and sub-grid cases, finite-tip reachability derivation, measured interval widths, and remaining representation limits. These previews describe target/cutter geometry; toolpaths and stock verification remain later milestones.

**Exit:** straight-channel depth/floor dimensions match the formulas; islands offset in the correct direction; finite-tip center offsets differ correctly from ideal floor boundaries; changing the integer scale within the supported budget does not erase features silently.

## 5. M2: SVG import and saved jobs

- [x] Select an SVG/XML parser against actual Inkscape exports and the supported-subset contract.
- [x] Support units, transforms, viewBox, closed path commands including arcs, and basic closed shapes.
- [x] Resolve compound paths, fill rules, visibility, and supported inherited fills.
- [x] Preserve source IDs and map normalized components to user selections.
- [x] Add explicit diagnostics for unsupported features and meaningful geometry repairs.
- [x] Add versioned job serialization with embedded artwork and editable incomplete settings.
- [x] Implement import/plan/inspect CLI entry points as their core operations become available.

Completed 2026-09-05 with engine 0.3.0. The release build, 65 integration tests, Clippy, and formatting pass. Both native and plain Inkscape exports preserve dimensions, compound holes, and selections; source bounds agree with Inkscape within 0.000171 mm. The [M2 capability report](m2-capability-report.md) records parser selection, supported features, precision budgets, portable job replay, diagnostics, and limits. `import`, `inspect`, `select`, and `validate-job` are implemented; `plan` reports explicit unavailability until M3 provides cutting paths.

**Exit:** round-tripped jobs preserve physical dimensions, selection, and normalized geometry within tolerance. Reversed winding, transformed groups, and compound letters behave correctly. Text/strokes that need Inkscape conversion are reported rather than machined accidentally.

## 6. M3: endmill planner

- [x] Start with conservative clearing of the deepest admissible region at each stepdown.
- [x] Extend to depth-dependent regions so shallow passes clear more of the upper stock.
- [x] Generate offset loops with valid stepover, island handling, and residual detection.
- [x] Implement supported ramps/direct plunges with explicit tool capability and feed requirements.
- [x] Use simple clearance-plane links between disconnected cuts.
- [x] Record stock removed by every cutting move, including entries and links.
- [x] Finish accessible floor regions to the requested depth.
- [x] Verify tool-center feasibility along whole segments and compare swept areas with the target.

**Exit:** paths preserve the nominal slopes plus allowance, never cross an island, and leave measurable stock for the V-bit. Cases with no endmill access produce an empty endmill stage and continue to the V-bit planner when appropriate. Missing pocket coverage and unsupported entries are visible diagnostics.

Completed 2026-09-05 with engine 0.4.0. The [M3 capability report](m3-capability-report.md) records the endmill-only completion contract, motion/stock checks, synthetic fixture matrix, explicit failure statuses, saved-plan replay, and numerical limits. `plan` produces recorded XYZ moves; `inspect` and `verify` recompute stock and clearance. Empty stages retain target stock for the M4 planner described below.

## 7. M4: V-bit finishing and rest machining

- [x] Extract the interior medial axis, retaining radii and curved-edge evaluation.
- [x] Generate full-depth boundary paths and variable-depth narrow-detail paths.
- [x] Split paths at depth-cap and finite-tip reachability transitions; verify family junctions.
- [x] Generate finite-spaced floor-clearing lanes using the allowed ridge height.
- [x] Add V-bit depth passes based on material actually left by the endmill.
- [x] Prune only path sections proved to be air cutting; do not constrain centers to residual polygons.
- [x] Simulate the combined sequence and add cleanup for uncovered reachable regions.
- [x] End the V-bit stage with a complete achievable boundary finish.
- [x] Bound cleanup iteration; return a diagnostic if coverage fails to converge.

**Exit:** wide floors, narrow channels, pointed ends, holes, and transitions are covered within declared tolerances. Pointed-bit floor ridges match the analytic straight-lane case. Zero-ridge requests that require pointed-bit area clearing are rejected. Finite-tip limitations are reported without pretending they are part of the nominal target.

Completed 2026-09-05 with engine 0.5.0. The [M4 capability report](m4-capability-report.md) records curved and rising medial paths, finite-tip/ridge behavior, conservative air pruning, actual variable-radius sweeps, bounded cleanup, final-family replay checks, and the sampled/slice scope of completion. Adaptive continuous stock-quality certification remains M5.

## 8. M5: verification and output precision

- [ ] Complete depth-slice swept-area construction for both tools and linear XYZ motion.
- [ ] Maintain conservative removal bounds for rest pruning and overcut checks.
- [ ] Refine slices and height-field cells where a requested error bound is unresolved.
- [ ] Validate segments analytically or with bounded subdivision; include entry and linking moves.
- [ ] Distinguish overcut, permitted floor ridges, unreachable detail, and other residual stock.
- [ ] Enforce the explicit detail-residual limit without misclassifying missed reachable material as unreachable.
- [ ] Add explicit failed and inconclusive report states with locations and measured bounds.
- [ ] Validate rounded machine coordinates and detect zero-length/reversed moves introduced by formatting.
- [ ] Check deterministic ordering and artifact invalidation after job/tool changes.
- [ ] Measure planning time, memory, segment count, and artifact size on representative artwork.

**Exit:** deliberately injected gouges, unsafe modeled-stock links, missed strips, and rounding errors are detected. Coarse grids cannot yield a false pass on narrow fixtures. Reducing resource limits yields an inconclusive result. Refinement demonstrates convergence on the analytic fixture set.

## 9. M6: LinuxCNC integration

- [ ] Obtain the actual M6 macro/configuration behavior and document its preconditions/postconditions.
- [ ] Set tool numbers and choose macro-managed or post-managed length compensation.
- [ ] Define work offset, clearance plane, spindle/feed settings, and output precision in the profile.
- [ ] Emit an explicit modal setup and linear moves with G61 initially.
- [ ] Group endmill work before V-bit work and support combined/per-tool programs.
- [ ] Restore required cutting state after M6 without overwriting macro-managed offsets.
- [ ] Implement a reader for the emitted numeric G-code subset and recheck its motion list.
- [ ] Inspect programs in LinuxCNC preview or a matching simulation configuration.

**Exit:** tool selection, compensation, units, work offset, spindle restart, and stage transitions match the documented profile. Independent per-tool files do not depend on a previous file's modal state. There are no guessed machine positions or hidden probing assumptions.

## 10. M7: local browser workflow

- [ ] Serve the interface and API from the native local application.
- [ ] Add SVG preview, physical dimensions, origin controls, and region selection.
- [ ] Add job/tool settings with Rust-generated validation results.
- [ ] Display target, toolpaths, stock after each tool, and error/residual overlays.
- [ ] Distinguish preview resolution from verification status.
- [ ] Run planning in background tasks with progress, cancellation, and stale-result protection.
- [ ] Save/load portable jobs and export only results matching current settings.
- [ ] Match the CLI's output for identical jobs and engine versions.

**Exit:** an ordinary job can be imported, configured, inspected, saved, reopened, and exported without editing JSON. Calculation does not freeze the UI, and changing settings invalidates old output visibly.

## 11. M8: physical validation and release

- [ ] Choose a small test coupon containing a broad pocket, acute point, tapered channel, and island.
- [ ] Record actual tool angle/tip dimensions, material, feeds, stock-top datum, and tool-change setup.
- [ ] Review the program in LinuxCNC and perform an appropriate air cut before the first material trial.
- [ ] Machine the coupon and inspect the floor, all wall families, corner transitions, and residual ridges.
- [ ] Measure deviations where practical and distinguish geometry errors from setup, cutter, and material effects.
- [ ] Adjust the design or implementation for demonstrated failures and rerun affected fixtures.
- [ ] Package the intended native target with browser assets and installation/startup instructions.
- [ ] Document supported SVG features, tool setup, verification meaning, and known limitations.

**Exit:** the coupon demonstrates the intended combined operation, and the recorded measurements are consistent with the declared model/tolerances and real setup. Installation and job reproduction are tested on the intended everyday host.

## 12. Fixture and verification strategy

| Fixture | Primary failure it should expose |
| --- | --- |
| Straight channels above/below the flat-floor threshold | Incorrect half-angle, depth cap, or width calculation. |
| Rectangle with acute triangular extension | Missing rising corner path or a seam at the depth cap. |
| Circular region and annulus | Curve approximation, offset direction, island handling. |
| Star with alternating wide/narrow arms | Lost branches, incomplete cleanup, topology changes. |
| Concave shape with point-to-segment Voronoi edges | Treating a curved medial edge as straight. |
| Tapered channel and narrow neck between broad pockets | Disconnected clearing, abrupt transitions, missed stock. |
| Pocket accessible to the V-bit but not the endmill | Incorrect assumptions about prior roughing. |
| Tool fitting a channel exactly, or a center region collapsing to one point | Discarding valid cuts when polygon offsets have zero area. |
| Finite-tip V-bit near a sharp terminal point | Virtual-apex confusion and hidden unreachable detail. |
| Parallel floor passes with known spacing | Incorrect ridge-height calculation. |
| Inkscape compound letters and nested groups | Fill rules, transforms, source selection, physical units. |
| Repeated/collinear edges and tiny slivers | Quantization, degenerate input, silent topology changes. |
| Long move with valid endpoints but an intervening island | Endpoint-only motion validation. |
| Deliberately coarse output precision | Gouging or broken connectivity after serialization. |
| M6 macro changing modal state | Incorrect assumptions at the second-tool transition. |

Use analytic examples, measured geometry errors, and invariant tests. Include translation/rotation invariance and scaling tests that scale dimensional settings and tolerances together. Use golden artifacts sparingly for interfaces and small readable motion sequences; avoid huge exact G-code snapshots that conceal the reason for a failure.

Compare the target and actual sweeps through independent calculations where possible. A preview produced by the same mistaken formula as the planner is not independent evidence. Benchmarks inform optimization after correctness; define practical latency and memory targets from the representative jobs rather than promising unmeasured performance.

## 13. Optimization backlog

After the combined workflow is reliable, evaluate path ordering, verified in-stock links, accelerated stock analysis, multiple clearance tools, arc fitting, bounded LinuxCNC blending, and WebAssembly. Each optimization retains the same fixtures and verification requirements. A changed strategy must not silently weaken the user's finish tolerance or remove small details.
