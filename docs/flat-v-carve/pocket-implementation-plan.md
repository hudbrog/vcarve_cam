# Pocket operation implementation plan

Implementation specification, 2026-09-21. The core and editor are implemented.
Regression, browser workflow and performance evidence is recorded in [Pocket](pocket.md)
alongside the implemented contract and reproducible checks. Requested scope: closed, flat-bottom pockets, islands, one flat
endmill, layered offset clearing, plunge/ramp/helical entry, lead-in/lead-out,
optional wall finishing, and multiple pockets at the same depth in one operation.

## Existing implementation and design choice

The current `cam-core/src/pocket/` is the roughing engine for Flat V-carve.
Its context consumes `VcarveInput`, constructs a tapered `Target` from a V-bit,
and computes clearance as depth × slope + cutter radius + wall allowance.
Exposing that engine unchanged would retain tapered-wall semantics.

Add a native `operations/pocket/` planner with a constant-section target and
ordinary `PlannedOperation` output. Reuse region booleans/offsets, boundary
queries, ordering, milling motions and stock-sweep primitives. Extract small
target-independent routing/cleanup helpers where useful; do not make the new
operation depend on dummy V-bit settings or a fabricated V-carve input. Keep
the existing V-carve target and its regression fixtures intact.

Generalize `project/v5/resolve.rs::resolve_vcarve_region` into a filled-region
resolver accepting qualified component references, retaining a V-carve wrapper
if useful. Preserve owner/revision checks, placements, holes, cross-source union,
the finest selected grid, and source-error accounting.

## Proposed operation contract

| Area | First-version behavior |
| --- | --- |
| Selection | Filled components, using the same picking semantics as Flat V-carve. Their union is the pocket; holes in that union are islands. Separate artwork drawn inside a selected fill does not automatically become an island. |
| Multiple pockets | Disconnected selected regions share one top, bottom, tool and cutting settings. Plan every region; an inaccessible region prevents the whole operation from being marked complete. Different depths require separate operations. |
| Tool | One flat endmill with the existing milling assignment, copied cutting preset and capability checks. |
| Heights | Existing `HeightRef` top/bottom model. Bottom can be relative to operation top. A face-result reference must resolve to a preceding enabled face with sufficient coverage. |
| Clearing | Offset loops at bounded stepdowns, ending exactly at the requested bottom; handle disconnected regions separately. |
| Cutting values | Feed, plunge/ramp feed, spindle, maximum stepdown and stepover. Store each once, preferably in the existing assignment where already supported. Initially retain the existing clearing limit of stepover ≤ cutter radius. |
| Direction | Explicit climb/conventional choice, with correct traversal around both outer boundaries and islands. Entry and linking moves have separately checked semantics. |
| Allowance | Nonnegative radial stock to leave. Without finishing, this is the intended residual; with finishing, it is removed by boundary passes. No axial allowance in the initial version. |
| Wall finish | Same tool, optional explicit finish feed (otherwise cutting feed). Finish outer walls and island boundaries at bounded depths after roughing. Do not default to a full-depth side cut. |
| Entry | Explicit plunge, bounded linear ramp or helix. Require the corresponding tool capability; report when the requested entry cannot fit in any selected pocket. |
| Leads | Optional tangent-line or tangent-arc lead-in and lead-out, using existing `LeadSpec` dimensions/feed semantics. Apply at entry/exit of a cutting run and wall finish, with the entire cutter sweep inside the allowed region. |
| Travel | Start with clearance retracts between unproved connections. Add stay-down links only when continuous cutter clearance and allowable fresh engagement are established. |
| Limits | Bounded layers, loops, generated motions and verification work. Exhaustion yields an incomplete/inconclusive result that cannot export. |

Use an additive `OperationSettingsV5::Pocket(PocketSettingsV5)` variant. Keep
schema 5 if this follows the project's existing additive-operation policy;
test existing jobs unchanged and explicit refusal by readers without Pocket
support. Do not add a new public legacy document format. New operations may
remain incomplete and saveable; planning readiness reports missing inputs.

Initially require the pocket to stay within stock XY and its bottom within stock
thickness. Open-sided pockets, below-stock breakthrough, adaptive
clearing, multiple cutters, rest machining and floor finishing are later work.

## Geometry, coverage and motion rules

For selected region `P`, tool radius `r` and radial allowance `a`, rough tool
centers lie in `erode(P, r + a + numerical_guard)` at every depth. Finishing
uses the nominal wall offset with its own error budget. Holes expand under
erosion and must remain protected during entries, cuts and links.

Separate three outcomes: intentional allowance, geometrically unreachable
material (such as small internal corners), and unintended gaps in reachable
material. Compute the cutter-reachable target from admissible centers and
compare it with conservative removal reconstructed from actual motions.
Allow ordinary unreachable corner residual with an explicit diagnostic and
preview; missing reachable coverage beyond tolerance blocks completion.
Wholly inaccessible selections, exact-fit contacts without numerical margin,
and unresolved topology must not silently produce a successful empty pocket.

Repeated offsets alone do not prove complete clearing. Topology changes and
central residuals need a bounded cleanup pass or a clear incomplete result.
Check every layer, entry and connection for containment, depth, stepdown and
motion continuity. Report both floor and wall residual within the supported
coverage contract; never treat the display simulation as verification.

Prior stock history remains ordered removal evidence. It must not implicitly
publish an entire pocket as a reusable top plane, nor turn this feature into
rest machining. For stay-down clearance use conservative swept-volume evidence;
the current point-query stock history, especially for ramps, is not sufficient
on its own to establish a cleared corridor.

## Helix, leads and multiple-pocket routing

The generic `toolpath.rs::ArcMove` already supports circular XY motion with
linearly varying Z. Build helical entry from explicit bounded arc segments
(for example half-turns), preserving exact final depth and pitch. This avoids
introducing another motion representation. Confirm emitted G2/G3 with Z through
the existing numeric readback and verify Z interpolation in simulation/timing.

Proposed helix settings are cutter-center path radius, maximum ramp angle and
entry feed. Derive pitch from circumference and angle, capped by the allowed
axial increment; show the resulting swept diameter in the UI. Choose a
deterministic feasible center separately for each disconnected pocket. Require
explicit ramp capability and cutter-length clearance. For entry into solid
stock, constrain radius/engagement so the cutter does not leave a central plug;
the first version can conservatively cap the center-path radius below cutter
radius with a numerical margin. Larger helices require separately proven
pre-cleared access and are outside this first implementation.

Check the entire helix swept envelope, varying-depth engagement, islands and
the transition to the first clearing path. Include any necessary level cleanup
at the entry bottom. A requested helix that does not fit must produce an
operation/pocket-specific error; it must not silently become a plunge.

Treat entry and leads as distinct motions: descend using the chosen entry,
connect through a checked corridor to a tangent lead-in, cut, lead out into
allowed space, then retract or take a proven link. Reuse Profile lead geometry
where applicable, but replace its retained-side checks with pocket containment
and engagement checks. Validate requested lead lengths/radii without silently
dropping or shortening them. Checked stay-down links may join cutting runs;
they must not silently bypass explicitly requested lead behavior.

Use deterministic pocket ordering and retain resolved pocket identity in
diagnostics and motions. A reasonable first policy is to complete roughing and
finishing for one pocket before moving to the next, with a clearance retract
between disconnected pockets. Stage assembly must preserve that sequence.

Audit optional arc fitting explicitly: it currently admits every non-knife
stage. Either recheck fitted Pocket motions against containment/coverage budgets
or exclude Pocket stages from fitting until those checks exist. Planner-authored
helix and lead arcs are part of the requested feature and remain supported even
when optional fitting of clearing polylines is disabled.

## Implementation sequence

1. **Document model and geometry resolution.** Add settings, validation,
   creation defaults, reference inspection/repair, assignment/resource support
   and the shared filled-region resolver. Extend command-based editing and
   machining identity. Verify round trips, incomplete jobs, stale references,
   island preservation and cross-artwork union before generating motions.

2. **Minimum planner with verification.** Implement constant-section geometry,
   layered clearing, plunge entry and conservative retracts. Return native
   milling stages/motions, generation issues and coverage evidence. Include
   bounded residual cleanup, or refuse incomplete coverage. Complete the core
   rectangle, island, disconnected-region and narrow-feature tests here.

3. **Entry, leads, direction and finishing.** Add linear ramps and helical entry,
   tangent line/arc leads, requested traversal, allowance semantics and wall
   finishing at bounded depths. Cover multi-pocket routing with shared depths
   and individually resolved entry locations. Add checked
   stay-down links only after their clearance tests pass; retract fallback
   remains valid. Verify that rough and finish stage boundaries retain actual
   connecting motions and do not reorder the intended cuts.

4. **Ordered execution and export.** Register Pocket in v5 dispatch, scoped
   planning, tool/artwork dependency discovery, canonical fingerprints and
   stock history. Add `PocketRough`/`PocketFinish` roles to `sequence.rs`, the
   milling-role allowlist in `checks.rs`, output role names and display mappings.
   Wire generation/coverage failures into retained export admission. Exercise
   existing `cam collection plan/export`, machine tool mappings, strict numeric
   readback, prefix scope and retry of exact prepared bytes. No pocket-specific
   G-code dialect or separate CLI planner is needed.

5. **GUI and delivery.** Add Pocket to operation creation/list/routing;
   implement `pocket.rs`/`pocket_ui.rs` bindings using existing height, tool and
   cutting-value controls, plus helix and lead settings. Reuse multi-region
   filled selection and show islands, shared target depth, entry/lead paths,
   residuals and pocket-specific errors. Integrate undo/redo, save/reopen,
   tool library application, background planning, toolpaths and stock simulation
   in native and browser builds. Update the operation guide, README and backlog.

Primary integration files are `cam-core/src/project/v5/{mod,commands,references,
resources,inspection,resolve}.rs`, `cam-core/src/{sequence,checks}.rs`,
`cam-core/src/post/sequence.rs`, retained planning/export in `cam-service`, and
operation authoring, picking, tools, session and UI routing in `cam-gui`.
Exhaustive matches help find dispatch work; also audit wildcard/default branches
and string-based UI inventories that compilation will not catch.

## Acceptance evidence

- Geometry: rectangle, circle, concave shape, island, nested fill, disconnected
  pockets, overlapping selections from placed artwork, sharp corners, narrow
  neck and exact-fit/no-access cases. Check topology-change residuals explicitly.
- Machining: partial final stepdown, maximum permitted stepover, unsupported
  plunge, valid/invalid ramp and helix, fractional final helix turn, pitch and
  central-plug constraints, valid/invalid tangent leads near islands and walls,
  climb/conventional outer/island orientation, allowance retained/removed,
  layered finishing and motion/resource limits.
- Checks: reconstruct removal from motions; deliberately alter paths across an
  island/outside the pocket, omit a loop, over-deepen a pass and break continuity.
  Each applicable check must reject the altered result independently of intent.
- Workflow: Face → Pocket → Profile/Drill; invalidated face dependency; tool and
  geometry edits invalidate retained plans; display-only edits preserve reuse;
  prefix export excludes later operations; incomplete plans cannot export.
- Multiple pockets: two and many disconnected regions at one depth; stable
  ordering, clearance travel, distinct feasible helix centers, and one region
  too small for the requested entry. No selected region may be silently skipped.
- Helix output: arc center/direction, XY radius, Z interpolation, pitch, endpoint
  rounding, simulated stock and numeric readback agree with recorded motions.
- Product: selection and deselection, editing/undo, copied tool presets,
  save/reopen, simulation and LinuxCNC numeric readback on native/browser paths.
- Regression/performance: retain existing V-carve behavior and compare planning
  time/motion counts on a representative complex filled drawing. Record results
  rather than imposing an unmeasured speed target.

Run focused core/service/GUI tests per slice, then repository formatting,
Clippy, workspace tests and native/browser build checks for the completed feature.
Software acceptance and a later measured machining trial remain distinct.

## Implemented scope decisions

The user confirmed the initial scope and added lead-in/lead-out, helical entry
and multiple pockets sharing a depth. The implemented defaults are
filled-region selection, no axial allowance, informational reporting of
unavoidable corner residual, automatic entry placement and completing one pocket
before the next. Clearing runs from the inside outward; helix pitch is reduced
to land exactly at each layer in whole turns rather than using a fractional turn.
Link optimization and optional fitting of clearing paths should not delay the
requested entry, lead and multi-pocket behavior.

## Completion evidence (2026-09-21)

- Schema/commands/geometry: seven Pocket document tests, including saveable
  incomplete jobs, stale references, used-by inspection, placed overlapping
  artwork on the finest grid, copied tool/preset application and offline reset.
- Native planning/checks: twelve Pocket tests cover layered depth, island and
  finish coverage, inaccessible regions, limits, ramp/helix/lead fit, core/pitch
  constraints, both milling directions and spindle rotations, Face dependency,
  prefix scope, altered-motion refusal and helical numeric G-code readback.
- Retained execution: Pocket prefix preparation, identical-byte retry,
  display-only rename reuse, tool/source/helix invalidation and refusal to
  export an incomplete later operation.
- Product: Pocket editor controls and numeric drafts, selection, undo/recovery,
  library values, save/reopen, depth-interpolated helix playback and island stock.
  Real Chromium/WebGPU/WASM workflow passes generation, backward/forward stock
  replay, checked G-code download and saved-job reopening without console errors.
- Delivery: native and browser release builds pass; current workspace gate is
  778 tests passed, zero failed, four existing ignored tests across 74 binaries.
  Formatting, Clippy with warnings denied, doctests and diff whitespace checks
  pass. Representative release measurements are in the operation guide.
- CLI: the full fixture passes `collection plan`, machine application and
  checked prefix export to one program (3,651 motions / four stages). The
  existing precision escalation from three to four decimals is reported.

The implementation deliberately returns incomplete coverage instead of adding
an unproved residual cleanup pass. It uses clearance retracts rather than
unproved stay-down links, and keeps automatic fitting of Pocket clearing arcs
disabled. These are the conservative alternatives allowed by this specification.
