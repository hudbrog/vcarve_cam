# GUI1 files, recovery and input continuation — 2026-09-11

This continuation stays inside the isolated egui experiment. It adds executable
file/recovery paths and input checks; it does **not** complete GUI1, select a
production persistence schema, or qualify native screen readers. Browser
screen-reader access remains the user's accepted exclusion.

## Implemented behavior

- Ctrl/Cmd+O opens a job; Ctrl/Cmd+S saves the raw inspector draft; Ctrl/Cmd+F
  focuses setting search. Global shortcuts pause during nonempty IME preedit.
  Raw fields have explicit source/operation/field IDs. Returning to an operation
  restores its last focused field and opens the relevant group.
- A pending file action displays a modal. Completion/cancellation restores the
  initiating control after egui retires its modal layer. Stale action IDs are
  ignored; a late draft import cannot replace newer edits.
- Native dialog and drop reads run on background threads with an 8 MB admission
  limit. Browser admission rejects multiple/oversized files before eframe reads
  them. Accepted JSON goes through the existing Rust open/planning adapter.
- Local recovery debounces changed raw text/workspace selection/job inputs by
  750 ms, with a 250 ms repaint wake-up while dirty. Writes are asynchronous;
  write latency is additional. Closing/crashing before acknowledgement can lose
  recent edits. Recovery does not rely on an unload callback.
- Native recovery uses `gui1-recovery/session.json` beside the experiment's
  executable, a stable OS-locked `session.lock`, and atomic sibling replacement.
  Browser recovery uses IndexedDB `cam-gui1-session-recovery`, one transactional
  record. Both compare the expected revision and preserve the prior record on
  failure. Failures stop automatic retries and retain the editable draft.
- Restart offers **Restore session** or **Keep current** before replacing the
  stored revision. Corrupt/unsupported records are preserved and reported.
  Restore retains raw invalid text and exact job JSON, then recalculates derived
  results. It never restores checked-output authority, workers or GPU objects.
- The experimental recovery envelope has a 9 MB cap; draft validation rejects
  unknown field identities. It is not the full future recovery contract with
  document identity, timestamps, portable-save identity and migrations.
- Failed saves retain the exact original payload and filename. **Retry previous
  save** uses those bytes even after another scene is loaded. Native writes flush
  a temporary sibling before replacement. Browser direct-save uses a writable
  file handle when available and compares readback bytes/SHA256; otherwise the
  Blob fallback reports only that a download was requested.
- A generated asset manifest and service worker cache the browser shell, WASM,
  platform glue and Worker. Recovery lives separately from disposable assets.
  Private protocol is now `gui1-spike-3`. Mixed-build client pinning and deployment
  upgrade/rollback behavior remain unqualified.

## Executed evidence

Windows 10.0.26200, native Vulkan/RTX 3090 at 125% DPI; Chromium 152.0.0.0
in the Codex in-app browser, WebGPU. Browser GPU identity remains unknown.
The [recorded results](gui1-evidence/files-input-results.json) and current
[artifact hashes](gui1-evidence/build-manifest.json) accompany this report.

| Check | Result and evidence boundary |
| --- | --- |
| Modal focus regression | egui harness renders an actual modal frame, completes cancellation, then asserts Maximum depth is focused and `-` survives. This caught and fixed a one-frame modal-layer bug. |
| Selection/reorder focus | Named-control harness asserts draft identity and restored field focus after reorder and operation navigation. |
| IME/keyboard harness | Injected preedit `工具` blocks Ctrl+O; commit keeps Unicode. Tab/Shift+Tab move between Maximum depth and Wall allowance. This is not an OS IME tour. |
| Real native dialog | Ordinary keyboard Ctrl+O opened an rfd Windows picker; Escape returned UIA focus to Maximum depth, with document text `-` and explicit cancellation status. |
| Real native restart | Autosaved revision 1 contained raw `-`; close/relaunch offered recovery; Restore session displayed `-` and its invalid-number issue. |
| Native transactions | Tests exercise two stores, stale revisions, actual OS lock contention, corrupt-record preservation and a late save acknowledgement while newer edits remain dirty. |
| Native destination failures | Real exclusive Windows file handle denies replacement; old bytes survive. Releasing the lock permits retry. A directory used as a destination also fails without losing retained output. |
| Checked flower retry | Actual Rust preparation produces one checked program; failed destination followed by successful atomic save retains SHA256 `c190feced004bb42e67a5da97e1c108b4897a750132a8008551bc1ce05d3997e`. Headless platform test, not a full dialog save tour. |
| Native drop adapter | A real temporary JSON file is read through `Port::drop_file`; the emitted exact job plans through shared Rust to 37 motions. OS drag gesture is not covered. |
| Browser persistence | `/web/platform-probe.html` passes real IndexedDB round trip, two concurrent transactions with exactly one winner, byte-limit transaction abort preserving the previous record, and actual Rust field validation. |
| Browser save failure/retry | Same probe injects a denied file-picker handle, retries with a writable/readback handle and verifies exact bytes/hash. This is explicitly an injected handle, not real disk permission or quota exhaustion. |
| Browser offline restart | Entered `-` in the real canvas; observed Recovery saved, revision 1. With tab networking disabled via CDP, reloaded the cached app, chose Restore session and observed `-` and its issue. Repeated after saving the small job at revision 2: the offline Worker recalculated 82 vertices / 37 motions and rendered the stock preview. Networking was restored afterward. |
| Browser drop event | Synthetic browser `DragEvent` carrying two `File`s is rejected with a visible message. One File containing the unchanged embedded small reference traverses eframe's File reader and the real Worker. This is not an OS drag gesture. |

Native UIA exposes labels/focus, but its indexed click reported unavailable
coordinate geometry in this environment; screenshot-backed clicks plus ordinary
keyboard input were used. This does not establish Narrator/NVDA usability.

## Reproduce

From `flat-v-carve/experiments/gui1`:

```powershell
cargo fmt --all -- --check
cargo clippy --offline --locked --all-targets -- -D warnings
cargo test --offline --locked
cargo test --offline --locked --test interaction -- --ignored
./launch.ps1 -Target native
./launch.ps1 -Target web
```

Native automated total: 17 behavior/state/file/simulation tests, plus two
separately executed GPU layout comparisons. The checked flower file test takes
about 35 seconds in the debug profile on this machine. Existing layout goldens
pass unchanged. Native and WASM release artifacts were rebuilt.

Open `/web/platform-probe.html` and choose **Run platform checks**. It creates and
cleans only its own randomly named probe database; it does not clear app recovery.
The displayed report labels real transactions and injected save handles separately.

For the manual recovery tour, type `-`, wait for **Recovery saved**, reload/restart,
and choose **Restore session**. For native focus, place the caret in Maximum depth,
press Ctrl+O and Escape, then type again. For save retry, enable denied-write
injection, attempt Save, disable injection and use **Retry previous save**.

## Remaining qualification

Actual OS IME commit/cancel tours on both targets and native screen-reader tours;
browser keyboard-only picker cancellation and real direct-save/fallback disk
outcomes; physical quota/eviction behavior; OS file drag gestures; shared-library
revision conflicts (the current revision test covers session recovery only).
Browser serialization/validation remains on the UI thread and needs memory and
latency measurements. Native replacement does not promise directory-fsync/power-
loss durability. Alternate platforms and packaging remain unqualified.

Other GUI1 gates remain in the framework decision: viewport overlays/picking/DPI,
renderer resource recovery, bounded motion/tile transport and sustained S/M/L
budgets, support-matrix qualification, and final user review/framework decision.
