# GUI2 authoring, recovery and revision-2 workspace

Implemented in the production `flat-v-carve/crates/cam-gui` application on
2026-09-11. GUI2a's retained execution workflow is extended by GUI2b source
authoring and GUI2c working-context recovery. User acceptance remains pending.

## Authoring

A new SVG becomes a schema-5 job with one artwork item, one Flat V-carve
operation and two unset job tools. Stock, feeds, spindle values, tolerances,
machining geometry and machine configuration are not copied from a fixture.
Filled components are initially unselected. SVG unit interpretation, placement
and qualified component references use the existing core catalogue.

The inspector edits numeric placement, physical stock, work zero, start XY,
clearance, planning tolerances, tool geometry, cutting assignments and spindle/
plunge capability. Endmill-only and Combined modes are explicit. Finishing
assignments remain available when temporarily unused. Choosing Combined sets
bounded planner resource/quality policy; it does not fill machining values.
Complete local controls for all advanced policies remain GUI3b work.

Machine profiles are imported explicitly as schema 2; their process settings
are copied into the job. Controller T/H mappings are editable in Machine.
Applying a profile preserves setup's work-zero datum. Setup and Machine edit
the same clearance. The core export resolver now requests a controller mapping
only for the executing endmill in Endmill-only mode; nominal V-bit geometry
still participates in the target and execution identity.

Incomplete jobs with unset settings can be saved and reopened. Partial numeric
text and incomplete dimension groups are kept in recovery; they block portable
job saving and generation while active. Tool/stock dimension groups commit
atomically when complete and valid. Inputs and portable jobs are limited to 8 MB.

## Revision and recovery

Undo/Redo groups each focused field edit into a transaction and includes
component, mode, datum and tool changes. History is capped at 32 transactions
and a 16 MiB serialized-size budget per stack. The 9 MB recovery envelope trims
old history before writing and retains the current draft.

Recovery schema 3 stores raw text under actual artwork/operation IDs, the
selected inspector, filter, panel width, scroll offsets, Prepare/Simulate mode,
camera, path stage and requested simulation position. Simulation position is
reapplied only after regeneration produces the same execution fingerprint.
Confirmed native job-save identity is retained separately from partial text;
browser downloads never claim a confirmed disk save. Recovery contains no
usable worker handle, generated plan or checked-program authority.

Late computation, cancellation and save completions cannot overwrite a newer
document. Native recovery uses locked optimistic revisions and atomic sibling
replacement; browser recovery uses an IndexedDB transaction. Failed output
saves retain the exact checked bytes for retry. Old recovery schemas are not
migrated, consistent with the accepted compatibility policy.

## Visual implementation

The revision-2 concepts guide the graphite header, orange active mode, teal
Generate/selection treatment, light panels, left navigator, right contextual
inspector and gridded stock viewport. Setup, Artwork, Operations and Job Tools
route to real editors. Stock is visible before generation when explicitly
defined; generated heightfields retain the established simulation semantics.
Export is prepared from the retained execution and saved from Machine & Export.
The concepts' multi-source/operation lists and global library are later slices.

## Review recipe

Build/run instructions are in the [GUI README](../../flat-v-carve/crates/cam-gui/README.md).
Previous working review artifacts were preserved locally in `artifacts/gui2a/`.
Current artifacts are `artifacts/gui/native/cam-gui.exe` and `artifacts/gui/browser/`.

1. Import `fixtures/gui2/new-carving.svg`. Confirm empty machining fields and
   zero selected components. Save this incomplete job, then select components.
2. Use the inspector filter to reach individual fields. Set stock thickness 10,
   rectangle minimum XY (-10, -10), width 70 and length 50. Set start XY (0, 0),
   clearance 5, motion tolerance 0.01 and verification tolerance 0.05, all in mm.
3. Set maximum depth 1, allowance 0 and floor ridge 0.1. Choose Depth-dependent
   clearing. Set endmill diameter 3/cutting length 15, feed 1200/plunge 400
   mm/min, stepdown 0.5/stepover 1 mm, spindle 12000 RPM, CW and Plunge yes.
4. Set nominal V-bit geometry: 90 degrees, tip diameter 0.1, maximum diameter
   12 and cutting height 5 mm. Generate in Endmill-only mode.
5. In Machine, load `fixtures/gui2/machine.json`; inspect its copied values and
   mappings. Export, inspect the check report, then Save checked bytes.
6. Switch to Combined. Set finishing feed 1000/plunge 300 mm/min, stepdown 1,
   stepover 1, detail residual 0.1 mm, spindle 12000 RPM, CW and V-bit plunge yes.
   Inspect the V-bit mapping, regenerate and select Simulate. Compare After
   endmill, After V-bit and arbitrary earlier stock positions.
7. Edit placement/work zero, regenerate and save/reopen the authored job.
   Type `-` in depth, navigate away and return, Undo/Redo, restart and restore.
   Regenerate after restoration. Cancel generation and generate again.

These are deterministic review-fixture values, not material/tool recommendations.
The browser smoke tests automate the new-SVG and original flower tours, including
an injected denied save followed by downloading and hashing the actual bytes.

## Verification

The GUI release tests cover new/incomplete jobs, SVG unit and placement round
trips, both execution modes, explicit mappings, datum authority, native worker
IPC, history limits, context restoration and failed recovery replacement.
Core `project_v5` regression tests and workspace Clippy are also run. Actual
browser evidence is stored under `artifacts/gui/browser-smoke/` with state
records and screenshots. Final run locations are recorded below when complete.

Verified on 2026-09-11:

- Formatting and `git diff --check`: passed.
- Workspace Clippy with warnings denied: passed.
- GUI release suite: 35 tests passed, including native IPC and recovery I/O.
- Core `project_v5`: 13 tests passed.
- Native and browser review artifacts: built successfully.
- Flower browser tour: passed with zero captured console errors, including
  playback, arbitrary backward scrub, actual downloaded-byte hash, denied-save
  retry, recovery, worker cancellation and regeneration. Evidence:
  `artifacts/gui/browser-smoke/2026-09-11T20-44-19.247Z/`.
- New-SVG browser tour: passed with zero captured console errors, including
  explicit unset values, both modes, placement/datum, stock checkpoints, Undo,
  restart context and a configured portable job saved and reopened. Evidence:
  `artifacts/gui/browser-smoke/2026-09-11T20-44-23.179Z/`.

Screenshot review also corrected a clipped narrow-window simulation scrubber
and increased slider-track contrast against the new light panels.
Final screenshot review exposed and fixed missing GPU uploads for the separate
source-contour section. The authoring browser tour now checks actual cyan pixels
inside the viewport before generation and after recovery over physical stock.
That complete tour passed with zero captured console errors; final screenshots
and evidence are in `artifacts/gui/browser-smoke/2026-09-11T20-51-20.984Z/`.
Packaged browser manifest: `2a7c5714a0af`; native SHA256 is recorded beside the
executable in `SHA256SUMS`.

The support envelope remains Windows x86_64 and desktop Chromium with WebGPU.
Actual native file-dialog/IME interaction and device-loss manual tours have not
been newly qualified here. The incumbent UI remains available until replacement
workflows are accepted; new application development stays in `crates/cam-gui`.
