# GUI1 simulation continuation — 2026-09-11

The experiment now has a narrow Rust port of the existing TypeScript heightfield,
plus a bounded native/browser stock preview. This closes the initial port-feasibility
gap. GUI1 as a whole and production GUI2 remain pending. The earlier prototype was
committed as `81a9fed`; this continuation changes only the experiment and evidence.
No H milestone, CAM algorithm, fixture, or incumbent TypeScript code was changed.

## Numerical comparison

`web/compare-simulation.mjs` executes the original `web/src/sim/engine.ts` and
`setup.ts` using Node's built-in TypeScript type erasure. Both implementations
receive the same released Rust plan, tools, stock rectangle, grid and integer
motion prefixes. Setup is compared separately before integration. Native output
is compared with **every cell's Uint16 depth and tool owner**, plus tile versions,
checksums, motion counts and stage volumes. Aggregate volumes use a relative
tolerance of 1e-10; depths, owners and versions must match exactly.

| Case | Grid and prefixes | Result |
| --- | --- | --- |
| Seed `0x5eed`, 80 mixed moves | 400 × 400 at 0.025 mm; prefixes 0, 20, 80, 7, 55, 80 | Zero depth/owner/version differences |
| Unchanged small contact-line | 248 × 167 at 0.1 mm; initial, final, backward-to-initial and final again | Zero differences; empty roughing retained |
| Unchanged flower | 7,392 × 3,858 at 0.025 mm; prefixes 0, 7,048, 22,883, 3,524, 22,883 | Zero differences, including backward replay |

The flower's final reference field has 8,545,542 cut cells, checksum `3f22cb5a`,
and 4,380.694269987111 mm³ removed (3,399.0174614114794 roughing;
981.6768087497704 finishing). These are approximate display integrals, not
machining verification. Field tile storage at that prefix was 52,692,800 bytes.
Replay has at most four copy-on-write checkpoints and a conservative 64 MiB
checkpoint budget. Tests also verify exact replay with a zero-byte cache budget.

The first native flower finishing interval took 45.15 s versus 61.83 s in TS;
replay from an earlier prefix took 53.77 s versus 57.10 s. These are individual
integration timings, not sustained/p95 benchmarks. The full reference grid is
therefore not presented as an interactive scrubber.

Independent Rust tests cover analytical discs, pointed and finite-tip cones,
ramp coverage intervals, fractional integration, non-cutting moves, owner changes,
grid admission and bounded backward replay. Randomized comparisons also cross tile
edges and include off-stock moves, both ramp directions, over-thickness cuts and
cutting-height-limited V-bits. This is software evidence, not a physical trial.

## Clickable preview and limits

Loading a real reference builds a separate, explicitly labeled preview on the
disposable compute process/Worker. It uses approximately 512 cells at most on
the stock's long side, never a finer grid than the reference, and at most 18
integer motion checkpoints including initial, roughing boundary and final stock.
All intervening motions are integrated; only retained prefixes are exposed.
Uncheck **Stock preview** for the original motion-line timeline. The stage
selector filters paths; stock retains the cumulative prefix, including preceding
roughing when inspecting finishing paths. Arbitrary/fractional stock scrubbing
is not implemented in this UI.

The flower preview is 512 × 268 at 0.3609375 mm versus the reference 0.025 mm.
The UI shows both cell sizes, motion prefix and removed volume. Eighteen packed
depth/owner arrays total 9,879,552 bytes, below the hard 20 MiB retained-cell cap.
One field is uploaded to a persistent GPU storage buffer when the checkpoint
changes; camera changes update the camera uniform. Cell surfaces and stock
bottom/sides use real stock thickness. Fine V-bit detail is limited by this
explicitly coarser grid. Raster output never feeds CAM planning or export checks.

The small preview's 17 checkpoints and flower preview's 18 checkpoints match the
original TS engine on every cell. The **actual Rust WASM compute entry point**
also matches every packed cell/checksum against these native/TS-qualified preview
snapshots. Full-resolution flower parity was tested on native; full-resolution
WASM parity remains unverified.

Windows/Vulkan and Chromium/WebGPU at 125% DPI visibly rendered the flower stock.
Native isometric inspection and backward selection restored an earlier prefix;
browser backward selection restored motion zero with 0.00 mm³ removed. These
are smoke checks, not complete rendering/DPI or screen-reader qualification.

The first native interactive smoke exposed unbuffered JSON I/O (31.38 s total).
Transport now uses `BufWriter` with checked flush and `BufReader`. A subsequent
headless worker plus result-file-write sample took 3,569 ms for a 12,226,605-byte
flower scene; it excludes the parent's read/render. The subsequent native UI
reported a complete flower result in 3,136.1 ms and rendered the stock. These
individual samples are not a controlled speedup benchmark. Browser wrapping no longer
parses then reserializes the entire worker result before Rust deserialization.
The final browser smoke reported 11,404.1 ms and displayed all 18 flower
checkpoints; this includes worker startup, planning and result transport.
Whole-result copies and peak memory still need measurement; retained cell bytes
are not total process/tab memory.

## Reproduction and evidence

From `flat-v-carve/experiments/gui1`, after native and WASM release builds:

```powershell
cargo test --offline --locked
node web/compare-simulation.mjs
node web/compare-wasm-simulation.mjs
```

The full comparison takes several minutes and writes large cell dumps into
ignored `artifacts/simulation/`. `--preview-only` skips full-resolution real
fixtures but retains randomized and preview comparisons. Source/artifact hashes
accompany measurements. The numerical native capture predates the later UI and
transport-only changes in this continuation.

- [Full-grid comparison](gui1-evidence/simulation-comparison.json)
- [Preview comparison](gui1-evidence/simulation-preview-comparison.json)
- [WASM preview comparison](gui1-evidence/simulation-wasm-comparison.json)
- [Buffered native worker](gui1-evidence/simulation-buffered-worker.json)

Nine state/behavior/simulation tests pass. Two existing shell goldens were
regenerated for changed timeline text, visually inspected and compared again
with update mode off. Native/WASM release builds, formatting and warning-free
clippy pass. Shell goldens do not validate custom GPU stock geometry.

Remaining at this capture: keyboard/dialog focus/IME checks; real
drop/recovery/save failure and retry; actual device loss; required platform rows
and distribution checks. Picking, translucent selection, blade glyph, paged
motion/tile transport, arbitrary stock seeks, sustained S/M/L performance and
whole-process memory accounting remain unqualified. Final framework outcome
and prototype user acceptance are still pending.

Later updates: see the [files/input continuation](gui1-files-input-evidence.md)
for completed recovery and focus checks, and the [current decision](ui-framework-decision.md)
for the remaining work. The user excluded both native and browser screen-reader
support on 2026-09-11; screen-reader qualification is no longer a GUI1 gate.
The [paging/viewport continuation](gui1-paging-viewport-evidence.md) then
replaced the JSON scene result with a paged binary payload, re-ran this preview
comparison with zero cell/owner/version differences, and added a native-versus-
browser transport parity capture.
