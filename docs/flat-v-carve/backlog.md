# Remaining work

Reviewed against the source and tests on 2026-09-20. This list carries forward
unfinished work from the retired plans; it is not a promise to implement every
optional extension. Completed slices and their historical evidence remain in
Git history.

## Integration and release

- **LinuxCNC validation and a measured machining trial.** Obtain the actual M6
  macro, tool table and INI/HAL configuration; check their declared contract,
  travel limits and datum against the generated program. Run matching preview,
  an appropriate air cut and a measured coupon with a pocket, acute point,
  tapered channel and island. Record tools, material, workholding, feeds and
  measured deviations. Software readback and the initial reported machine trial
  do not complete this acceptance. See [LinuxCNC output](linuxcnc.md).
- **Release qualification.** Native/browser/portable builds already exist.
  Qualify clean-install launch, upgrades/recovery, paths with spaces, offline
  use, actual file-dialog failures and real OS input on intended targets.
  Explicitly record unsupported platforms, real device-loss coverage and
  sustained frame-time/total browser/GPU memory measurements. The legacy React
  UI has already been removed; another cutover implementation is unnecessary.

## Confirmed implementation gaps

| Work | Current evidence and completion condition |
| --- | --- |
| Operation-scoped CLI selection | `cam-app/src/collection_cli.rs` has no `select` command. Add explicit operation/owner/revision-bound selection if CLI authoring is required; use the same document commands as GUI selection. |
| Legacy benchmark/reproduction scripts | `benchmark-m5.ps1`, `check-m6.ps1`, `benchmark-flower.ps1`, `benchmark-settings.mjs` and `analyze-motions.mjs` still depend on removed commands or plan-artifact shapes. Port to supported collection/engine entry points or retire them, then verify against current fixtures. |
| SVG warnings in artwork UI | `project/v5/artwork.rs` exposes fatal `import_error` but does not preserve importer warnings in the catalogue. Carry nonfatal diagnostics into artwork inspection and display them with their owning source. |
| Machine touch-off reference and end parking | `AppliedMachineConfiguration` and `post/sequence.rs::SequenceProfile` have no dedicated touch-off-reference or parking field. Define how touch-off relates to setup work zero; for parking, specify coordinate frame and safe retract, emit once per program and extend readback. Keep this distinct from existing M6 return/start positioning. |
| Authoritative assembly/rapid checks | `cam-gui/src/sim_checks.rs` is display-only. A future core check needs authoritative assembly data, diagnostic bounds and independent validation before it can gate export. Do not promote a coarse display sample into proof. |
| Thin-feature engraving semantics | The Flat V-carve target remains clearance-limited (`vcarve/settings.rs`). Decide whether separate constant-depth centerline engraving, a minimum contact depth, or explicit refusal is wanted for features narrower than the tip. Preserve the current tapered-target contract until that decision and tests exist. |
| Prepared-program inspection and richer delivery | Core supports ordered one-program/sequential bundles. A general GUI view of decoded post-generated positioning/process state and broader multi-file destination/partial-write handling from the original delivery plan remain separate work. Reuse exact retained bytes, preserve prefix scope and never invent M6 internals. |

## Optional extensions

- **Facing interaction:** draggable explicit entry, ramped row entry for ramp-only
  tools, and a deliberate camera-fit policy. Current min/max/explicit/alternating
  entry and stock/coverage calculations are implemented; fitting toolpath extents
  is a display trade-off, not evidence that artwork is rescaled.
- **Simulation:** acceleration/junction-limited timing, tool-change duration,
  custom holder segment editing, arbitrary-angle sections, hatching, improved
  overlapping transparency and incremental wall rebuilds when measurements
  justify them. A solid stock mesh and detached-part simulation need separate
  modeling work.
- **Drilling:** nearest-neighbor ordering, conical-tip stock modeling, optional
  canned G81/G83 output and synchronized tapping/reaming/boring. Current expanded
  cycles, pecks, dwell and full-diameter depth are implemented.
- **Machining:** multiple clearance cutters, open/adaptive/rest pocketing,
  bounded pocket residual cleanup and optimized links, ramped tab
  shoulders, multiple radial/axial profile finishing passes, profile helical entries
  and sacrificial knife alignment leads. Each needs its own geometry/check
  contract rather than being inferred from a UI control.
  Closed flat-bottom Pocket with islands, shared-depth regions, plunge/ramp/helix,
  tangent leads and layered wall finishing is implemented; see [Pocket](pocket.md).
- **Performance:** measure the medial subdivision budget before relaxing it;
  consider end-to-end execution paging and sparse checkpoint transfer before
  admitting the originally proposed million-motion display workload;
  qualify 10×/100× full-pipeline workloads (past 100× results were import-only),
  component-local planning and threaded WASM with the necessary isolation and
  scoped-parallelism changes. Arc fitting, bounded knife blending, path ordering
  and verified links already exist.
- **Conveniences:** Center on stock, drag-to-reorder operations, additional
  resource lifecycle actions and removal of redundant Create knife outlines.
  Extra fonts/textures and carrying source curves through the entire polygon
  pipeline are optional design choices, not incomplete required milestones.

Useful implementation anchors are linked from [job-model.md](job-model.md) and
[gui-architecture.md](gui-architecture.md). Close backlog items with current code
and focused evidence; do not revive stale milestone checklists.
