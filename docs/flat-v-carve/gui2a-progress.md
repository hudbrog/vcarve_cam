# GUI2a implementation

Status: implementation and verification in progress; user acceptance pending.

The new application lives in `flat-v-carve/crates/cam-gui`, a first-class member
of the main Cargo workspace. Its native binary is `cam-gui`; its browser entry
is `web/index.html`. Both start the new application directly. GUI1 stays frozen
under `experiments/gui1` as framework qualification evidence. New product work
must not return to that experiment or depend on its crate.

The user explicitly permits breaking old saved-job and machine-setting formats.
GUI2a therefore uses schema-5 jobs and schema-2 machine profiles directly, with
one SVG and one Flat V-carve operation admitted for the first slice. The real
flower fixture was converted once as regression data. No runtime legacy bridge
or saved-data migration is required.

The implemented loop covers open, explicit applied machine configuration, real
cutting and geometry edits, retained generation, cumulative heightfield playback
with worker-side arbitrary seeking, checked preparation from that retained plan,
exact-byte save/retry, job save/reopen, partial field text, Undo/Redo and recovery.
Requests and prepared output are bound to edit revisions. Native cancellation
kills and reaps the compute child; browser cancellation terminates its Worker.

The retained service now exposes a checked read-only plan getter for display.
Export precision escalation also fixes a core readback bug: the independent
comparison uses the decimal grid actually emitted, rather than the profile's
minimum precision. A regression covers both output layouts.

Native integration tests compare all 22,883 flower motions with the established
planner and compare stock cells at checkpoints and arbitrary forward/backward
prefixes. A subprocess test covers the real retained worker, save denial/retry,
save/reopen and forced termination of non-yielding work. The browser smoke test
drives canvas controls through Chromium and checks downloaded bytes and recovery.
Test definitions alone are not acceptance evidence; record actual runs below.

Verified on 2026-09-11 after promotion:

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo test -p cam-gui --release --locked`: 27 passed, including the actual
  native worker and the full flower regression.
- Core `sequence_bundle`: 4 passed; service `retained`: 5 passed.
- `scripts/build-gui.ps1` native and browser targets: built successfully.
- Chromium/WebGPU workflow: passed real editing/generation, stock checkpoint
  selection, arbitrary backward scrub, playback into V-bit, checked output,
  exact downloaded bytes after denied save, save/reopen, partial-text recovery,
  Worker cancellation and generation/preparation in a fresh Worker. Zero
  captured console errors. Evidence and screenshot are local review artifacts
  under `artifacts/gui/browser-smoke/2026-09-11T19-49-34.868Z/`.

The browser harness initially assumed a scrub position would precede the endmill
boundary after changing depth. That assertion was corrected to require an
arbitrary backward position distinct from both stage endpoints. It now checks
the actual edited execution. These checks do not substitute for user acceptance.

Build and review instructions are in the [runtime README](../../flat-v-carve/crates/cam-gui/README.md).
CI includes workspace checks and native/browser review artifacts. The incumbent
`cam.exe` CLI/service and old browser UI remain available during replacement.
Deprecating their UI entry point follows user acceptance of the replacement's
needed workflows; promoting the new crate does not declare that cutover complete.

GUI2b source authoring and GUI2c broader context/recovery work remain separate
increments. Full GUI2 acceptance, actual OS IME/device-loss tours and unqualified
platforms retain the limits documented in the framework decision.
