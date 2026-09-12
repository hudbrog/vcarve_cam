# GUI5 reusable tools and machine configuration

Status: GUI5 implementation complete on top of GUI4 commit `45d2bc1`.
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
- Clippy with warnings denied passed on the cleaned source.
- GUI5 browser tour passed profile, library CRUD, competing-writer conflict,
  job-tool, machine, real Generate/Simulate/Export and offline portable reopen
  workflows on browser build `2c1e7ade673c`. Final evidence:
  `flat-v-carve/artifacts/gui/browser-smoke/2026-09-12T08-47-22.821Z/evidence.json`.
- GUI3 and Flower browser regressions passed on the same final browser build.
  Evidence: `flat-v-carve/artifacts/gui/browser-smoke/2026-09-12T08-48-49.043Z/evidence.json`
  and `flat-v-carve/artifacts/gui/browser-smoke/2026-09-12T08-50-33.461Z/evidence.json`.
- The latest normal GUI4 browser regression passed after portable reopen,
  including replacement, repair, delete/Undo, ordering and batch import. Its
  evidence is under `flat-v-carve/artifacts/gui/browser-smoke/2026-09-12T06-53-15.792Z`.

## Remaining work

The intermittent GUI4 replacement failure was not reproduced in the final
normal-flow regression. The GUI5 browser tour also covers active tool mappings,
work zero, real generation/simulation/preparation, exact checked bytes, and
portable reopen without the external library. Native and browser artifacts
were built from the same source revision; the local ignored artifact directory
contains the exact build logs and screenshots used for review.

The remaining qualification boundary is physical machine cutting and broader
multi-operation workflows, which belong to later GUI6+ slices in the plan.
User review should use the recipe in `gui5-review.md` and record any usability
or machine-specific findings separately from these software fixtures.

Final artifact hashes from the cleaned source:

- Native `artifacts/gui/native/cam-gui.exe`: SHA-256
  `48dc32cde2e2334e19abde20e4c79cba7d5b3d33507006aa59d5d845ee084f18`.
- Browser `crates/cam-gui/pkg/cam_gui_bg.wasm`: SHA-256
  `aee5e3c7ea9d8a7b7650e939db686b52921711c0ef91ed62fc8f02395b35121e`.

Generated builds, logs and screenshots are local ignored artifacts; the source,
fixture and review recipe are included in this checkpoint.
