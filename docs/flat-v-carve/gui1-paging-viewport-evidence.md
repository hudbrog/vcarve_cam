# GUI1 paging, viewport and measurement continuation — 2026-09-11

This continuation closes the largest GUI1 implementation gap: motion and stock
data no longer travel as one JSON document, the viewport has selection fill,
a blade marker and DPI-aware display picking, and S/M/L timing, transfer and
memory numbers now exist. GUI1 as a whole and every production milestone remain
pending, and no H slice status changed.

Branch: `codex/gui1-viewport-perf` (worktree `D:\proj1\.worktrees\gui1-viewport-perf`),
based on `a75568f`. All changes are inside `flat-v-carve/experiments/gui1` plus
these documents. No CAM crate, fixture, incumbent UI or backend checklist changed.

## What changed

| Area | Before | Now |
| --- | --- | --- |
| Transport | The whole scene (vertices and stock cells) was serialized as JSON in the worker result file / postMessage string | One small metadata document plus one sectioned binary payload: `u32 metadata length \| metadata JSON \| payload`, sections aligned to 16 bytes |
| Browser channel | Worker parsed and reserialized the result string on the UI thread | Worker posts metadata as JSON and the payload as a transferred `ArrayBuffer`; the UI calls `receive_payload` then publishes a `ComputedBinary` event |
| Motion rendering | One retained vertex batch uploaded whenever the scene revision changed | Page table (`8,192` motions = 458,752 bytes per page); pages are fingerprinted, admitted nearest the playhead first, copied only when their fingerprint or identity changed, and evicted when the resident budget is exceeded |
| Stock display | Whole packed field copied per checkpoint | Tile-major packed grid (one contiguous 256 KiB tile); only tiles whose version changed since the last upload are copied |
| Checkpoints | Fixed prefix list; no arbitrary seeks | Transported checkpoints seed a bounded `Playback`; the display seeks anywhere by restoring the nearest checkpoint and replaying the suffix. Seed checkpoints are pinned so the replay window cannot be churned away |
| Picking | None | CPU spatial index over the transported motion vertices plus an exact projected-distance test; tolerance is declared in physical pixels and converted through the reported scale factor |
| Selection / blade | None | Translucent selection ribbon for the picked motion and an endmill/V-bit marker with the declared tool radius and shank line |
| Renderer failure | A UI-only injected failure flag | The injected flag remains, plus a real resource-recreation drill (pipelines/buffers rebuilt and pages re-copied from retained CPU data) and a real wgpu validation error captured through an error scope |
| Measurement | Individual samples | `--measure` writes deterministic S/M/L/flower numbers: transport, fingerprints, page copies, picking, scrubbing and whole-Rust-heap counters |
| Browser input | DOM comparison page only | `web/input-probe.html` drives real browser events into the running Rust application and reads a published state snapshot |

## Transport and paging (release build, Windows 11 / RTX 3090 / 1.25× DPI)

`cam-gui1-desktop.exe --measure docs/flat-v-carve/gui1-evidence/perf-measure.json flower`;
full numbers in [perf-measure.json](gui1-evidence/perf-measure.json).

| Workload | Motions | Motion pages | Metadata | Payload | JSON-equivalent estimate | First copy | Reload copy |
| --- | --- | --- | --- | --- | --- | --- | --- |
| S · 20,000 | 20,000 | 3 | 3.2 KiB | 7.1 MB | 5.5 MB (0.77×) | 3 pages / 0.11 ms staging | 0 pages |
| M · 200,000 | 200,000 | 25 | 4.4 KiB | 30.4 MB | 34.7 MB (1.14×) | 25 pages / 1.57 ms | 0 pages |
| L · 1,000,000 | 1,000,000 | 123 | 2.8 KiB | 58.4 MB | 153.4 MB (2.63×) | 123 pages / 9.49 ms | 0 pages |
| Flower reference | 22,883 | 3 | 67.6 KiB | 21.0 MB | 14.1 MB (0.67×) | 3 pages / 0.19 ms | 0 pages |

Reading these numbers:

- The JSON-equivalent column is measured on a bounded sample and extrapolated
  (vertex JSON ≈ 79 bytes per vertex, cell JSON ≈ 2.0 bytes per cell). It is a
  comparison, not a serialized copy of the whole scene, which is why it can be
  *smaller* than the binary payload: the payload also carries the tile-major
  stock grid, the replayable motion stream and 16-byte alignment. The dominant
  win is that the metadata document stays in the kilobyte range while the heavy
  part is a single binary transfer.
- `staging copy` is the CPU memcpy of exactly the byte ranges a GPU page copy
  would stage. Real device transfer time needs a GPU run and is not claimed.
- Reloading the identical payload copies nothing on either target: page
  fingerprints match, the payload identity is unchanged, and the camera uniform
  is the only buffer written. Idle frames and camera moves therefore upload no
  scene data.
- With a 16 MiB page budget the L workload admits 37 of 123 pages and reports
  the other 86 as omitted by budget instead of drawing them silently.

The `--measure` JSON-estimate for the flower is lower than the binary payload
because the flower's stock grid is 512 × 268 with 18 checkpoints, while the
payload also carries the 1.2 MB replayable motion stream; the earlier JSON
transport carried the same cells as decimal integers, which is what makes the
estimate larger for the synthetic M and L cases.

## Picking, selection and the viewport

`pick.rs` mirrors `scene.wgsl` exactly and converts a physical-pixel tolerance
through `pixels_per_point`, so a hit target does not shrink on a high-DPI
display. The candidate index is a camera-independent uniform grid; the final
test is the exact projected point-to-segment distance.

| Check | Result |
| --- | --- |
| Index versus brute force | 0 disagreements over 3,840 camera/rect/DPI combinations on the fixture, plus 40 real-geometry cursors per workload. Exact ties between coincident projections are counted separately from distance differences |
| DPI tolerance | Ten physical pixels is inside a twelve-pixel tolerance and thirty is outside at 1.0, 1.25, 1.5 and 2.0; the same cursor in points is inside at 100% and outside at 200% |
| Query cost | p95 0.004 ms (20k motions) / 0.066 ms (200k) / 0.42 ms (1M); index memory 0.8 / 7.0 / 34.7 MB |
| Index build | 1.2 ms / 11.7 ms / 45.0 ms, once per scene, measured rather than hidden |
| Integration | Interaction test at 150% and 100% DPI selects the expected transported motion and reports "No motion within" instead of keeping a stale selection |

The selection ribbon is a translucent quad around the picked segment; the blade
marker uses the simulator's declared tool geometry (disc for an endmill, cone
from the tip to the widest declared radius for a V-bit) plus a shank line to the
stock top. Overlay build cost is p95 2–3 µs.

## Stock scrubbing

The display owns a `Playback` seeded with the transported checkpoints and a
20 MiB checkpoint budget. Backward seeks restore the nearest retained
checkpoint; forward seeks replay from the current position; cold seeks (no
checkpoints, budget 0) replay from the initial prefix. Targets come from a fixed
`0x5eed` sequence.

| Workload | Checkpoints | Scrub step p95 | Backward p95 | Forward interval p95 | Cold p95 | Dirty tiles per seek |
| --- | --- | --- | --- | --- | --- | --- |
| S · 20,000 | 5 | 5.2 ms | 11 ms | 24 ms | 62 ms | 4 / 4 (1,024 KiB) |
| M · 200,000 | 9 | 40.7 ms | 67 ms | 150 ms | 869 ms | 4 / 4 |
| L · 1,000,000 | 5 | not replayable | — | — | — | 0 |
| Flower reference | 18 | 8.3 ms | 12 ms | 23 ms | 253 ms | 2 / 4 (512 KiB) |

The M probe replays up to 12.5k of its 200k motions per checkpoint interval on a
512-cell display grid (≈3.8 µs per motion), which is why a cold seek there costs
~0.9 s. That is the measured reason the display carries checkpoints; the earlier
plan target of a 250 ms M scrub is met for checkpoint-bounded seeks and is *not*
met for a cold one. L transports no replay stream at all: the display states
that arbitrary seeks are unavailable for that workload and keeps checkpoint
jumps. The 512-cell display preset is unchanged and still bounds the field.

## Renderer recovery and error handling

- **Resource recreation drill**: rebuilds the pipelines, bind group and buffers
  and re-copies every required page from the retained CPU payload. The document,
  selection and stock state are untouched, and the recovery count is shown in
  the status area.
- **Real wgpu validation error**: an invalid buffer creation inside a
  `Validation` error scope is submitted and its message is captured and shown;
  only the invalid request is dropped, the device stays usable. On WebGPU the
  error scope cannot resolve synchronously, and the UI reports that instead of
  pretending the probe passed.
- The earlier injected-failure control is retained so a deliberately blank
  viewport can be demonstrated without claiming device loss.
- Real driver device loss (TDR / `device.lost`) is still **unverified**; the
  drill proves the recovery path, not that the driver reports loss the same way.

## Browser integration probe

`web/input-probe.html` starts the real application and drives browser events
into it, reading the published state snapshot (`probe_state`):

1. A drop event with a real `DataTransfer` and a `File` loads the fixture job
   (37 motions, 3 pages).
2. `Ctrl+F` focuses a control; `compositionstart/update/end` events deliver IME
   text to the focused field, and the preedit/commit state and committed text
   are asserted.
3. The IndexedDB recovery store is read from the same origin.

`web/platform-probe.html` additionally records the browser's own storage
estimate, a bounded real IndexedDB quota attempt, the deleted-database reopen,
and the qualification facts (user agent, DPI, WebGPU presence, adapter info
where the browser exposes it, save-picker availability).

Both pages inject synthetic events. A real OS IME tour, a real OS drag gesture
and a real file-dialog cancel/confirm remain manual checks.

## Cross-target parity finding

`web/compare-wasm-simulation.mjs` now compares a native worker frame with the
browser package for the same request
([transport parity](gui1-evidence/simulation-transport-parity.json)):

| Scene | Rendered motion vertices | Packed cells compared | Replay records differing | Max coordinate delta |
| --- | --- | --- | --- | --- |
| Small contact-line | identical | 1,114,112 | 0 / 37 | 0 |
| Flower | identical | 4,718,592 | 41 / 22,883 | 1.11 × 10⁻¹⁶ mm |

The rendered f32 geometry, the section table, the tile versions, every packed
cell and the final field checksum agree. The **f64 planner output is not
bit-identical across targets**: 41 flower motions differ by one ULP in one
coordinate (visible in the engine's motion fingerprint and in the replay stream,
invisible in the rendered geometry at 1.11 × 10⁻¹⁶ mm). Cross-target bit parity
of the planner must therefore not be claimed; a tolerance-based comparison is
the honest form. The unchanged TypeScript heightfield comparison still passes
every cell/owner/version for the S and flower preview grids
([preview comparison](gui1-evidence/simulation-preview-comparison.json)).

## Memory

The experiment now installs a counting global allocator, so the report contains
whole-process Rust heap numbers instead of hand-tracked byte counts only.

| Workload | Heap before | Heap peak | Heap after |
| --- | --- | --- | --- |
| S · 20,000 | ~0 | 31.9 MB | 15.6 MB |
| M · 200,000 | ~0 | 142.4 MB | 73.9 MB |
| L · 1,000,000 | ~0 | 245.5 MB | 178.6 MB |
| Flower reference | ~0 | 117.1 MB | 42.1 MB |

These include the payload, the picker index, the display field, its checkpoint
set, intermediate JSON buffers and the measurement structures. They exclude the
JS heap, GPU memory and operating-system overhead; those remain unknown rather
than zero. The measured Rust heap after load (15.6 MB / 73.9 MB / 178.6 MB /
42.1 MB for S / M / L / flower) stays inside the provisional 512 MiB desktop
display target. The counting allocator is compiled for WASM as well and the
browser probe publishes its peak, but a browser *tab* total (JS heap + WASM heap
+ GPU allocations) is still not measurable through these APIs and is not
claimed.

## Checks executed

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked                     # 45 tests, 2 opt-in GPU goldens ignored
cargo build --release --locked
wasm-pack build --target web --release --out-dir pkg --out-name cam_gui1 -- --locked
node web/write-offline-manifest.mjs
node web/capture.mjs
node web/capture-build.mjs
node web/compare-simulation.mjs --preview-only
node web/compare-wasm-simulation.mjs
target\release\cam-gui1-desktop.exe --measure docs\flat-v-carve\gui1-evidence\perf-measure.json flower
```

State/behavior tests now include: paged transport structure, page residency and
reload behaviour, transported checkpoint rebuild equality, tile-dirty seeks,
DPI-aware picking against a brute-force oracle, motion-stream and tile-major
round trips, overlay geometry, and the earlier draft/recovery/file/simulation
suites.

## Still unverified

- Actual OS IME tour on both targets; the browser probe injects composition
  events, and the native harness injects egui IME events.
- Real OS drag-and-drop gesture on native and in the browser (the probe injects
  a real `DataTransfer`, not a user gesture) and real browser file-dialog
  confirm/cancel.
- Real GPU device loss; only the recreation drill and a captured validation
  error exist.
- Closed-loop frame-time measurements (the harness reports integration costs,
  not a two-minute sustained M playback with p95 frame intervals).
- GPU-side transfer time and GPU memory; only staged byte counts and Rust heap
  counters exist.
- Firefox, Linux and Safari rows; Chromium-only evidence so far. Real physical
  quota eviction cannot be forced through the storage APIs.
- The browser/system accessibility exclusions, prototype acceptance and the
  final framework outcome remain as recorded in
  [ui-framework-decision.md](ui-framework-decision.md).
