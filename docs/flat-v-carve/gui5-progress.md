# GUI5 reusable tools and machine configuration

Status: GUI5 implementation checkpoint on top of GUI4 commit `45d2bc1`.
Technical completion and final regression verification remain in progress.
The GUI4 native/browser review builds are preserved in
`flat-v-carve/artifacts/gui4` with their source commit.

Scope follows UI plan section 14.7 and sections 8.4/12.3:

- GUI5a: named profiles per operation/role, complete copied baselines including
  unset values, Applied/Modified/Custom, Reset without library, reviewed Reapply.
- GUI5b: separate library edit buffer, add/edit/duplicate tools and profiles,
  conditional saves/conflict comparison, explicit apply, import/export, job Undo
  independent of globally committed library edits.
- GUI5c: job-tool usage projection and shared geometry edits, one applied machine,
  reusable configuration editing, explicit active-scope T/H mappings and work
  zero, effective identity revalidation and portable reopen.
- Verify all resource workflows against real Generate/Simulate/Export, repeat
  previous GUI workflows, and publish runnable native/browser review builds.

Implemented: core physical-tool identity fix and separate Add/Use commands;
revisioned catalog model; independent native/IndexedDB persistence; retained
worker resource commands; production library/job-tool windows, named profile
Apply/Reset/Reapply actions, conflict comparison and explicit overwrite,
import/export, copied geometry editing, and reusable machine controls.

## Verification recorded before this checkpoint

- GUI suite: 64 passing tests, including native worker processes, resource
  persistence conflicts, offline profile reset, late results and job Undo.
  Local log: `flat-v-carve/artifacts/gui/gui5-tests.txt`.
- Core resource suites: 20 passing tests. Local log:
  `flat-v-carve/artifacts/gui/gui5-core-tests.txt`.
- Clippy with warnings denied passed before the latest diagnostic tracing.
- GUI5 browser tour passed profile, library CRUD, competing-writer conflict,
  job-tool, machine, real Generate/Simulate/Export and offline portable reopen
  workflows on browser build `2c1e7ade673c`. Local evidence:
  `flat-v-carve/artifacts/gui/browser-smoke/2026-09-12T06-43-21.803Z/evidence.json`.
- GUI3 and Flower browser regressions passed on build `2e1a3afb3b63`.
- The latest browser build with bounded diagnostic event tracing compiled
  successfully as `210116d098ed`; it has not completed the browser tours.

## Remaining work

The normal GUI4 browser tour intermittently times out replacing SVG after
portable reopen. A run with optional I/O tracing passed, but that does not
resolve the normal-flow failure. The cause is still under investigation.
The latest failure state is in local artifacts under
`browser-smoke/2026-09-12T06-43-23.628Z`; the checkpoint retains a bounded
30-event App diagnostic trace and optional smoke `--trace-io` instrumentation.

Resolve this failure, then run the strengthened GUI5 mapping/work-zero tour,
repeat affected regressions, and produce matching native/browser review builds.
The latest native artifact predates the diagnostic browser build. Complete the
final visual audit and evidence report before marking GUI5 complete.
Generated builds, logs and screenshots are local ignored artifacts; the source,
fixture and review recipe are included in this checkpoint.
