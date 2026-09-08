# U8: 3D stock simulator (animated material-removal preview)

Date: 2026-09-07
Status: phase 1 (technical-risk spike) complete; phase 2 core build complete and verified live (see §10) — M5 cross-check test, browser-checks entry, and flower-scale measurement remain

A browser-side 3D material-removal view driven by the recorded motions of a
current plan task: the stock starts as a slab, the recorded endmill/V-bit
moves carve it with an animated tool, and the operator can play, speed up,
and scroll through the process or jump to the finished result — the way
FreeCAD's CAM simulator does, so the result can be inspected visually and the
cutting process reviewed. This document specifies the design, records
decisions and budgets, and scopes delivery as a technical risk-alleviation
spike followed by the animated simulator build (§10).

## 1. Goal and non-goals

Goals (phase 1):

- Render the stock as a 3D surface carved by the recorded cutting motions,
  with playback: play/pause, a scrub bar, a speed multiplier, stepping, and
  seek-to-end for the static finished result.
- Sustain at least 15× real-time playback (§9): the carving pipeline must
  keep the surface synchronized with the moving tool at 15× model speed for
  real jobs, not merely render frames.
- Animate the tool (endmill cylinder / V-bit profile) along the interpolated
  motion positions.
- Run entirely from data the service already exposes; no Rust, schema, or
  HTTP contract changes.
- Keep the main thread free of bulk work; simulation runs in a Web Worker.
- One new pinned runtime dependency: `three` (plus pinned `@types/three` as a
  dev dependency), matching the project's exact-version policy.

Non-goals (all phases):

- No verification claims. M5 remains the only authority for overcut, residual,
  and quality bounds. The simulator is a visual aid with quantized,
  resolution-bounded geometry and is labeled *Visual preview* in the UI, per
  the product baseline's display rule.
- No new planning, feeds/speeds, or geometric-feasibility logic in TypeScript.
  The simulator applies exactly the motions the Rust engine recorded; it never
  invents, extends, or reorders cuts. Playback timing is a display model built
  from recorded feeds with stated assumptions, not a machine-time prediction.
- No new serialized job fields. The stock XY rectangle is display-only and
  derived (see §3); the job keeps storing thickness only.
- No undercut/5-axis support. The representation is a heightfield and the
  planners are 2.5D, so this is a structural match, not a limitation to work
  around later.

## 2. Relationship to the product baseline

The web UI baseline already reserves this feature: "A 3D orbit view, stock
surface, and arbitrary section query are full-release features", and requires
that "A low-resolution mesh is labeled Visual preview; changing display
resolution must not alter a verification result." It also permits feed-based
scrubbing once timing is credible; our timing uses recorded feeds plus
displayed assumptions (§5.3) and stays labeled as a model.

One baseline sentence needs revision when this ships: "Playback … never
simulates new cuts in client geometry code." The intent — playback must not
fabricate cuts beyond the recorded motions — is preserved here, because the
heightfield is driven strictly by the recorded motion records. The wording
should be updated to say playback renders recorded cuts at a labeled visual
resolution and creates no verification or planning claim.

## 3. Data sources (no service changes)

| Input | Source | Notes |
| --- | --- | --- |
| Motions | `GET /api/v1/tasks/{id}/motions/{offset}` pages | Existing paged endpoint over the service-owned plan file; page 0 via the plan result. All pages are consumed; the 2D preview's truncated subset is not sufficient. |
| Tool profiles | Job document already in the browser | `job.tools[].geometry`: endmill diameter/cutting length; V-bit included angle, tip diameter, max cutting diameter, cutting height. |
| Stock thickness | `job.stock.thickness_mm` | Required for planning, so always present when a plan task exists. |
| Stock XY rectangle (display-only) | Union of `nominalTarget` region bounds over the result's `stockSlices`, inflated by the largest active tool radius plus 1 mm, snapped up to whole cells | Available from result page 0 before motion paging starts. Cut sweeps stay inside target ⊕ tool radius; rapid travel outside the rectangle does not cut and is not covered. |

The simulator is available only while the current plan task is live in the
service (the paged-motion file must exist), the same lifetime rule as the
existing 2D motion preview. Fixture mode (no Rust service) shows the mode as
unavailable with an explanation, consistent with other computed views.

## 4. Simulation model

### 4.1 Tiled Uint16 heightfield

- One depth value per cell, stored as `Uint16` quantized depth:
  `n = floor(depth / q)`, `q = thickness / 65535` (≈1.5 µm at 100 mm).
  Floor-rounding makes the preview very slightly conservative. Update is
  `n = min(n, floor(depth/q))`; uncut cells are `0`.
- The field is a grid of 256×256-cell tiles. Tiles are unallocated (implicitly
  flat stock top) until a sweep touches them; only touched tiles allocate
  `Uint16Array` heights (128 KB) and `Uint8Array` stage owners (64 KB).
- A parallel `Uint8` owner channel records which tool role last lowered each
  cell (`0` uncut, `1` endmill, `2` V-bit), set exactly when a stamp lowers the
  height. This drives per-stage surface coloring at no extra sweep cost.

### 4.2 Resolution and budgets

- Target cell size: `min(0.1 mm, smallestCuttingDetail / 4)`, where the detail
  is the smallest of endmill diameter and V-bit tip diameter among tools that
  actually appear in cutting motions.
- Hard cap: the field's long side is at most 8192 texels (900 mm sheet →
  ≈0.11 mm cells; ≈89 MB heights worst case). If the target cell would exceed
  the cap, the cell size grows to fit and the UI reports the used resolution.
- Dirty-cell budget: 64 M cells (≤192 MB including owners, reachable only by
  full-sheet carving at cap resolution). If exceeded, the cell size grows to
  `sqrt(area / budget)` and the UI reports the coarsening.
- Caps are named constants in one module with the rationale beside them;
  runtime texture limits (`MAX_TEXTURE_SIZE`) lower the 8192 cap if smaller.

Worst case for the supported 900×600×100 mm job at cap resolution is ≈44.6 M
dirty cells ≈ 89 MB heights + 45 MB owners. Typical carves touch a small
fraction of the sheet and stay in the tens of megabytes.

### 4.3 Sweeps

Only `plunge`, `ramp`, and `cut` motions cut. `approach` and rapid moves are
skipped in phase 1 (collision flagging is a later extension).

- Endmill (flat cylinder, radius `r`): exact, no subdivision. For each cell in
  the segment's swept AABB, compute the parameter interval of the segment
  within distance `r` of the cell (quadratic), evaluate the linearly
  interpolated tip Z at the interval's endpoints, take the minimum. This is
  exact for sloped ramps. Zero-XY plunges stamp the disc at depth.
- V-bit (truncated cone): exact as well — the spike replaced the originally
  planned substep stamping with an analytic sweep. Per cell, the cut surface
  along the segment is `z(t) + max(0, dist(t) − tipRadius) / tan(halfAngle)`:
  the cone widens upward, so the cut is deepest at the tip flat and shallower
  with lateral distance. That surface is a linear term plus a convex one, so
  its minimum over the coverage interval (solved with the cutting radius,
  like the endmill) sits at an interval endpoint, the cell's projection onto
  the segment, or the tip-flat kink. Evaluating those candidates gives the
  exact deepest cut with no subdivision; beyond `maxCutRadius` the shank does
  not cut.
- Partial segments: every sweep accepts an optional sub-interval
  `[t0, t1]` of the motion, so playback can cut a motion fractionally as the
  tool traverses it.
- Defensive clamps: profiles never cut above the stock top or below the stock
  bottom; motions referencing unknown tools fail the run with a clear error
  rather than guessing.

### 4.4 Determinism and equivalence

Application is a pure fold over the motion sequence: the field at motion
index *i* is independent of how playback reached *i*. `seek(end)` therefore
reproduces the one-shot full application bit-for-bit, and this equivalence is
tested (§8).

## 5. Worker, motion store, and playback timing

### 5.1 Pure engine

New pure module `web/src/sim/engine.ts` (field, sweeps, timing, stats) with no
DOM or worker dependencies, so vitest can run it directly in Node.

### 5.2 Compact motion store

After a batch of pages arrives, the worker converts motions to structure-of-
arrays form immediately and discards the JSON objects: `Float32` start/end
XYZ, `Float32` feed, and packed `Uint8/Uint16` kind/tool-role/layer/operation
metadata — roughly 30 bytes per motion (≈30 MB at the 1M-motion budget).
Per-motion durations and a `Float64` prefix-sum of cumulative seconds are
built once, giving O(log n) time→motion lookup by binary search. The store is
duplicated by zero-copy transfer to the main thread for pose interpolation;
the worker keeps its own copy for sweeps.

### 5.3 Timing model (display only)

- Cutting motions take `travel length / recorded feed`. Motions with a null
  feed use an assumed default (1000 mm/min) and are counted; the count is
  shown in the panel as an assumption.
- Rapid and approach moves use a configurable rapid rate (default 3000
  mm/min, editable in the panel). A *cutting-time-only* toggle collapses
  rapid durations for faster review.
- The speed multiplier is logarithmic (0.25× to 200×); pause, single-motion
  step, and direct scrubbing by time or motion index are always available.
  Tool-change dwell, spindle spin-up, and acceleration are not modeled; the
  panel states this.

### 5.4 Protocol

Messages are validated with zod schemas in `web/src/sim/protocol.ts`,
following the existing contracts directory conventions.

- main → worker: `init {stock, cellMm, tools, budgetCells}`;
  `motions {batch, last}`; `seek {index, fraction}` (advance or rewind the
  applied prefix to motion `index`, cutting the current motion up to
  `fraction`); `cancel`.
- worker → main: `inited {cellMm, cols, rows, coarsened}`;
  `store {arrays…}` (transferred compact store, sent once on `last`);
  `progress {applied, dirtyCells, elapsedMs}`; `seekDone {appliedIndex,
  dirtyTiles, stats}` — dirty tiles are deltas since the previous emit;
  `error {code, message}`.

Playback runs a requestAnimationFrame clock on the main thread: it integrates
time at the current speed, interpolates the tool pose locally from the store,
and issues at most one in-flight `seek` at a time (natural backpressure; the
worker never blocks the clock). Forward seeks apply the delta motions;
fractional cutting keeps the surface synchronized with the tool tip.

The clock never runs ahead of a slower worker: model time advances only up
to the applied prefix plus a small look-ahead window. A requested speed above
the engine's sustained rate therefore settles at the highest sustainable
speed instead of desynchronizing surface and tool, and the panel shows the
effective speed whenever it stays below the requested one. Long jumps (End,
Start, big scrubs) run as an adaptive seek ladder: intermediate seeks of a
chunk size tuned toward ~100 ms of engine work, so the surface and
percentages update continuously and a newer target redirects the jump
mid-flight.

### 5.5 Rewind: undo log

The originally planned full-dirty-tile snapshots degenerate at flower scale —
every late snapshot copies the whole ~46 MB field, so a 64 MB cap retains
about one and rewind breaks (measured in the spike). The engine instead
exposes per-epoch change notification: `beginEpoch` starts an interval and
`onTileChange` reports each tile once, before its first modification in the
epoch. The worker keeps an undo log — one map of pre-change tile contents
per interval, taken every `max(1024, motionCount / 256)` motions — and
rewind undoes intervals from the newest down to the target, then re-applies.

Measured: rewind within the retained window is 36–88 ms. Continuous milling
has poor temporal locality (even serpentine finishing revisits all tiles),
so a 64 MB log retains roughly the last sixth of a flower-scale stream;
older targets fall back to bounded full re-application (1.8–2.0 s to 10 % on
the heaviest synthetic, sub-second on typical jobs, with visible progress).
Log memory is byte-capped with oldest-first eviction; exceeding the cap only
lengthens far rewinds, never fails. Compressing undo entries is the recorded
phase-2 lever to widen the fast window.

### 5.6 Cancellation and re-entry

Leaving the step or replacing the task terminates the worker and discards
state; nothing about the simulator blocks editing or planning. Re-entering
re-pages motions and fast-forwards to the requested position; the final
result is reached by a single seek-to-end.

## 6. Rendering (three.js)

Decision: use `three`, pinned to an exact release, with pinned `@types/three`.
It is the standard, actively maintained WebGL library; the project gains
mature camera controls, scene lifecycle, and geometry helpers instead of
maintaining a hand-rolled renderer, at ~150 KB gzipped. The bundle manifest
changes; CI hash checks apply as usual. The stock surface stays a small
custom shader so the heightfield remains texture-driven:

- Textures: `THREE.DataTexture` with `RedFormat` + `HalfFloatType` for depth
  (R16F half-float bits via `DataUtils.toHalfFloat` — the same two bytes per
  texel as the originally planned normalized R16) and `RedFormat` +
  `UnsignedByteType` for owner, both at field resolution. The spike found
  that vertex-stage sampling of normalized R16 (`UnsignedShortType`) returns
  zero on the tested ANGLE/D3D11 stack — colors rendered while displacement
  stayed flat — and R16F vertex fetch is byte-exact; R16F is the phase-2
  format. Tile deltas upload through the public
  `renderer.copyTextureToTexture(src, dst, srcRegion, dstPosition)` path
  (argument order per three 0.185): with a CPU-side `DataTexture` source the
  renderer performs a strided `texSubImage2D` from the full field array, so
  no separate staging texture is needed.
- Stock surface: one static grid `BufferGeometry` (vertex density
  `min(field, 2560 long side)`) with a `ShaderMaterial` that displaces
  vertices from the depth texture and computes per-pixel normals from it in
  the fragment shader, so shading detail exceeds mesh density. Vertex texture
  fetch and R16 sampling are ES 3.0 core.
- Vertical walls: stock-border skirts from border texels down to the stock
  bottom, plus interior cliff skirts (quads where adjacent texels differ by
  more than `max(3 cells, 0.3 mm)`, with a quad-count budget) so endmill
  walls read as walls instead of chamfers. V-bit grooves are inherently
  sloped and need no help.
- Tool: endmill as a cylinder, V-bit as a lathe profile generated from the
  actual tip/angle/max-diameter geometry, positioned at the interpolated pose
  each frame; slow cosmetic spin (explicitly not modeled RPM). Solid by
  default with a transparency toggle.
- Lights/camera: one directional light plus hemisphere ambient; `OrbitControls`
  with damping — drag to orbit, wheel to dolly (log scale, matching the 2D
  viewport's feel), shift-drag/middle to pan, Fit button, auto-fit on load;
  perspective ≈35° FOV. An optional *track tool* toggle pans the orbit target
  with the tool.
- Appearance: uncut stock is a neutral material; endmill-cut and V-bit-cut
  surfaces take the existing geometry-color language (blue/teal) with a
  legend; dark background; no textures that could masquerade as finish
  quality.
- Z exaggeration slider (1–10×, default 1×) applies consistently to surface,
  tool, and skirts; the current factor is always visible. Display only.
- Canvas handles device pixel ratio and resize; WebGL context loss restores
  textures from retained tile data; no WebGL2 → the mode is disabled with a
  notice naming the requirement.

## 7. UI integration

- Delivered as a viewport mode on the existing *Plan & inspect* step, not a
  new navigator step: a "Simulate 3D" toggle swaps the shared center viewport
  from the 2D SVG view to the 3D canvas, keeping the large-viewport-stays
  baseline rule. The 2D view and its layers remain one click away.
- Transport bar over the canvas: play/pause, scrub bar (time or motion
  index), speed slider with numeric entry, step buttons, seek-to-start/end.
  Current position readout: motion index, XYZ, tool, feed, elapsed model
  time.
- The Plan panel gains a *Stock simulation* section while the toggle is
  active: run state and progress (motions paged / applied, dirty cells,
  elapsed), the used cell size with coarsening notices, removed-volume stats
  per stage, the color legend, timing assumptions (null-feed count, rapid
  rate, cutting-time-only toggle), and display controls (fit, Z exaggeration,
  stage coloring, tool transparency, track tool).
- The result is labeled *Visual preview* with its resolution; text states
  that verification (`verify`) remains the authority for cuts and quality.
- Availability: enabled only when the current combined or endmill task has
  succeeded and is still live in the service; otherwise disabled with the
  reason.

## 8. Validation

- Unit tests (vitest, pure engine module, analytic expectations):
  - quantization is monotone and floor-rounds deeper;
  - tile allocation is lazy and bounded; dirty tracking matches stamps;
  - endmill plunge disc area ≈ `πr²` and cut capsule area within grid error;
    ramp result matches a finely subdivided reference;
  - V-bit stamp depth matches the cone formula at sample radii, flat-tip and
    `maxCutRadius` clamps included;
  - removed volume of a single plunge ≈ `πr²·depth`;
  - resolution selection respects the detail rule, 8192 cap, and budget;
  - compact-store conversion preserves every motion field; duration and
    prefix-sum lookups are exact at boundaries; pose interpolation matches
    endpoints and midpoints;
  - partial-segment sweeps equal clamped full sweeps;
  - determinism: incremental `seek` to the end reproduces the one-shot field
    bit-for-bit; snapshot restore plus re-apply equals direct application;
    snapshot eviction never breaks rewind;
  - throughput floor: a synthetic worst-case stream (widest endmill, high
    feed, finest cells) applies at ≥10× the cell-update rate implied by the
    15× requirement (§9), with the generous margin absorbing CI runner
    variance.
- Integration test (`check:live` suite, small fixture job): plan → page all
  motions → run the engine in-process → verify with M5 → assert the engine's
  removed volume/depth-band areas fall inside the report's published
  intervals (`residual_volume_mm3`, `overcut_volume_mm3`, band areas) plus a
  stated grid-quantization allowance. Exact interval semantics are resolved
  against `cam-core` verification code when writing the test; the invariant
  is "engine bounds contain the visual simulation".
- Browser checks: add a simulator section to `browser-checks.md` (load flower
  job, orbit/zoom/pan, playback at several speeds, scrub forward and
  backward, stage colors, exaggeration, coarsening notice, disabled states,
  context-loss recovery if practical).
- Standard gates stay green: `pnpm typecheck`, `pnpm test`, `pnpm
  check:contracts`, `check:live`; the bundle manifest changes only by the
  new pinned dependency and our own code.

## 9. Performance budgets and measurement

Acceptance targets on the reference Windows machine (flower job, the heaviest
real case):

- motion paging + zod parse + store build ≤ 20 s end-to-end with visible
  progress; typical box-lid jobs ≤ 3 s;
- one-shot seek-to-end ≤ 10 s flower (this bounds maximum-speed playback:
  200× over a 60 s cutting model needs ~0.3 s of apply per model second);
- sustained playback uploads ≤ 8 MB of tile deltas per frame typical;
- peak simulator memory ≤ 300 MB worker-side on flower (§4.2 worst case plus
  stores and snapshots);
- orbit and playback ≥ 25 fps at 1440p on integrated graphics.

Sustained-rate requirement (nonfunctional): playback must sustain at least
15× real-time model speed on the reference machine for real jobs — the
applied prefix may lag the playback clock by at most one frame of model time
at that speed. The worst-case arithmetic has ample margin: a 6 mm endmill at
3000 mm/min fed at 15× sweeps 4500 mm²/s, ≈450 k cell updates/s at 0.1 mm
cells — single-digit millions of simple typed-array operations per second,
roughly two orders of magnitude below established JS typed-array throughput.
Tile uploads at that rate are ≈150 mm² per frame, a few 256×256 tiles,
inside the per-frame delta budget above. The flower measurement records the
sustained multiplier actually achieved; the one-shot ≤10 s budget already
implies well over 100× for a job with tens of minutes of model time.

Measurements (paging, store build, sweep, tile/memory, rewind cost, fps) are
recorded in this document when the phase-1 spike completes and again when the
built simulator lands, following the M5/flower reporting practice. If TS
sweep speed misses the budget, the fallback is a
WASM core built from a small isolated Rust module — explicitly out of scope
for phase 1 (the workspace has not tested WASM), decided only on measured
need.

### Phase-1 spike measurements (2026-09-07, reference Windows machine)

Experiments 1 and 2 are complete. Code: `web/src/sim/engine.ts` (production
intent), `web/tests/sim/engine.test.ts` (17 tests: analytic disc/capsule/cone
expectations, a ramp sandwich between midpoint and deep-end stamping,
determinism, partial-segment equivalence, throughput floor),
`web/scripts/sim-bench.mjs`, and `web/tests/sim/parse-throughput.test.ts`.
All standard gates pass (typecheck, 118/118 vitest tests).

Sweep throughput (AABB cell visits per second; the 15×-implied requirement is
450 k/s and the tested floor is 4.5 M/s):

| Case | Rate | Gate |
| --- | --- | --- |
| Endmill serpentine, d6, 0.1 mm cells | 108 M cells/s | PASS (24× floor) |
| V-bit detail traffic, 60°/0.2 mm tip, 0.1 mm | 79 M cells/s | PASS (18× floor) |
| V-bit with real rest-machining writes | ≈55 M cells/s effective | PASS (12× floor) |

Flower-scale one-shot on a 900×600 sheet at the 0.1099 mm texel-capped cell
(grid 8192×5462):

| Scenario | Motions | Wall | Gate ≤10 s | Sustained headroom |
| --- | --- | --- | --- | --- |
| Flower-like load | 300,468 (508 m detail path) | 6.0 s | PASS | ≈6,000× |
| Deliberate 2–4× worst case | 700,468 (1,186 m detail path) | 20.6 s | exceeded | ≈3,800× |

Dirty cells 14.7 M; field memory ≈11 MB (heap stable; process RSS 163 MB
including the Node baseline). Sustained multipliers use a cutting-time-only
model and are large because the synthetic path lengths imply hours of model
cutting; the point is the two-to-three-order margin over 15×, not the exact
figure.

Decisions closed by these measurements:

- **No WASM core.** TypeScript sustains 55–108 M cells/s, 120–240× the
  15×-implied worst case. The WASM fallback is dropped from the plan.
- **Main-thread parsing stays.** `JSON.parse` runs at 230 MB/s and zod page
  validation at 292 MB/s; the projected ~100 MB flower stream parses and
  validates in ≈0.8 s against the 20 s gate. The existing `planResult` page
  walk in `web/src/service/http.ts` (which already pages the full motion
  stream with progress and identity checks) is reused as-is; worker-side
  parsing is dropped.
- **V-bit sweeps are analytic** (§4.3): exact, and much faster than the
  planned substep stamping.
- **Seek-to-end comfort bound:** jobs with 2–4× the flower detail load can
  exceed the 10 s one-shot figure (20.6 s measured). Playback progress is
  visible throughout, so this is a UX note, not a blocker; phase 2 may offer
  a coarser quick-preview resolution if such jobs turn out to matter.

Experiments 3 (three.js rendering feasibility in the target browsers) and 4
(playback pipeline end-to-end) remain.

### Phase-1 spike measurements — experiments 3–4 (2026-09-07, reference machine)

Experiment 3 ran in the browser against the dev-only page
`web/spike.html` + `web/src/sim/spike/main.ts` (three 0.185.1, WebGL2 via
ANGLE on the RTX 3090, 1920×1080 viewport): a 400×300×12 mm scenario with an
endmill pocket and three V-bit flower passes, 4096×3072 field, worst-case
2561×1921-vertex grid. Experiment 4 ran in Node via
`web/scripts/sim-playback-bench.mjs` on the flower-like 300k-motion stream.

| Check | Result | Gate |
| --- | --- | --- |
| WebGL2 + vertex texture units | ANGLE/D3D11, 16 VTM units | available |
| Vertex-stage R16 (normalized) sampling | returns 0 (displacement flat) | FAIL → format change |
| Vertex-stage R16F (half-float) sampling | byte-exact (42 = 42) | PASS |
| Initial full upload, 12.6 M texels | 145–296 ms | fine |
| Region upload, 256×256 R16F tiles | 0.024–0.087 ms per tile | PASS |
| Live playback, 1,144 motions, 30× requested | 683 frames, 29.7× sustained, 1.6 tiles/frame, ≈1.1 ms GPU upload/frame | PASS ≥15× |
| Orbit fps at 9.87 M triangles | 70.2 fps | PASS ≥25 (RTX 3090; iGPU pending) |
| Cliff skirts | 4,413 quads at 0.5 mm threshold (400k budget untouched) | fine |
| Per-frame seeks, 15×/60×/200× (Node) | 660–8,400× sustained | PASS |
| Undo-log rewind within retained window | 36–88 ms | PASS |
| Far rewind (beyond 64 MB log) | 1.8–2.0 s bounded replay to 10 % | accepted UX |

Visual confirmation: the finished view shows a genuinely recessed pocket
with cliff walls and V-bit grooves overlapping it — grooves shallower than
the pocket floor correctly disappear inside it, and the walls and
surrounding stock show the carving. The animation visibly carves both colors
and depth during playback.

Decisions closed by these measurements:

- **Heights texture is R16F half-float**, not normalized R16: vertex-stage
  sampling of normalized 16-bit textures returns zero on the tested
  ANGLE/D3D11 stack. Same memory, byte-exact fetches.
- **Region uploads use the public `copyTextureToTexture` CPU-source path**
  from a full-field CPU `DataTexture`; no staging texture and no direct-GL
  fallback are needed for correctness (direct `texSubImage2D` stays recorded
  as the performance fallback).
- **Rewind is the undo-log design** (§5.5); full-dirty-tile snapshots are
  dropped.
- The readback-verification probe used by the spike shows mismatches
  confined to the first/last readout row at some region origins across both
  texture formats. Mid-region and whole-region samples are byte-exact, the
  vertex probe is exact, and live rendering is visually correct, so this is
  treated as a harness artifact; production code does not use that probe.
- Open items carried to phase 2: integrated-GPU fps validation, undo-entry
  compression, and root-causing the probe edge artifact if readback
  verification is ever wanted in tests.

## 10. Delivery phases

### Phase 1 — technical risk alleviation (spike)

Four experiments, each ending in a recorded measurement and an explicit
decision. The engine work in experiment 1 is production-intent code with its
unit tests (§8), because phase 2 needs it verbatim; the rendering and
pipeline harnesses are dev-only pages excluded from the production bundle.
Experiments 1–2 run in any environment; 3 needs the target browsers; 4
reuses experiment 1's stream.

1. **Sweep-engine throughput** (risks: §11.1 and the 15× requirement).
   Implement the tiled Uint16 field and both sweeps in
   `web/src/sim/engine.ts` with their unit tests. Benchmark with vitest
   bench or a script: (a) a synthetic worst case — 6 mm endmill at
   3000 mm/min continuous passes at 0.1 mm cells — reporting cell updates/s;
   (b) a flower-scale synthetic stream (500k–1M motions including V-bit
   detail work) timed for one-shot seek-to-end and sustained multiplier.
   Gates: ≥4.5 M cell updates/s (10× the 15×-implied worst case) and
   flower-scale one-shot ≤10 s on the reference machine. A miss decides
   WASM-core-vs-coarser-cells *here*, before any UI work.
   **Done (2026-09-07): gates passed; decisions closed in §9.**
2. **Motion paging and parse cost** (risk: §11.2). Generate flower-scale
   motion JSON (≈100 MB, page-shaped) and measure `JSON.parse` plus zod page
   validation throughput, main thread vs worker. Walk the real paged
   endpoint once against a small fixture plan to confirm the page-walk
   assumptions. Gate: paging + parse ≤20 s; a miss adopts worker-side
   parsing (transferred page text) in phase 2.
   **Done (2026-09-07): ≈0.8 s projected for 100 MB; the existing
   `planResult` page walk is reused (§9).**
3. **three.js rendering feasibility** (risks: §11.3, §11.5, §11.6).
   Dev-only spike page: R16/owner `DataTexture`s plus the displaced-grid
   `ShaderMaterial` at the 2560-vertex-cap worst case, per-pixel normals,
   `OrbitControls`, cliff skirts over a synthetic pocket. Verify R16 vertex
   texture fetch on the target browsers; verify and measure the
   `copyTextureToTexture` staging-upload path for tile deltas (fallback:
   direct `texSubImage2D` through the renderer's GL context). Gates:
   ≥25 fps orbit at 1440p on integrated graphics (record the GPU), skirt
   wall quality acceptable at 0.1 mm cells, and the upload path chosen with
   a measured per-frame cost at the 15× delta rate.
   **Done (2026-09-07): measured on the reference machine's RTX 3090 —
   70 fps, R16F format decision, public upload path; see §9. Integrated-GPU
   validation remains open.**
4. **Playback pipeline end-to-end** (risks: §11.4, §11.7). rAF clock with
   the clamp behavior of §5.4, single in-flight seek, delta emission, and
   snapshots, driven by experiment 1's flower-scale stream. Measure the
   sustained multiplier actually achieved, rewind latency across full-range
   scrubs, and snapshot memory against the cap. Gates: sustained ≥15× with
   applied-prefix lag ≤ one frame of model time, full-range rewind ≤1 s,
   snapshots within the byte cap.
   **Done (2026-09-07): 29.7× sustained in-browser and 660–8,400× in Node;
   the ≤1 s far-rewind gate is replaced by the undo-log trade-off in §5.5
   (fast recent window, bounded replay with progress beyond it) — see §9.**

Exit criteria: numbers recorded in §9; the open decisions (WASM or not,
parse location, upload path, mesh cap) recorded here with the evidence; the
phase-2 estimate re-baselined on measurements instead of arithmetic.

### Phase 2 — animated simulator build (re-baselined on phase-1 results)

1. Add pinned `three` + `@types/three`; verify bundle manifest and typecheck
   gates. **Done.**
2. Complete the engine per §4–§5: compact Float64 store with feed-based
   timing (`web/src/sim/store.ts`), undo-log seek state machine
   (`web/src/sim/session.ts`) — bit-exact against one-shot application,
   including intra-interval rewinds — with unit tests (`tests/sim/`).
   **Done.** Float64 coordinates keep §4.4's bit-for-bit guarantee; Float32
   had flipped quantization levels at boundaries (caught by the tests).
3. `web/src/sim/protocol.ts` + `simWorker.ts`: messages, seek with delta
   emission, backpressure (single in-flight seek), termination on unmount.
   **Done.** Pointed V-bits (tip diameter 0) are accepted — caught live.
4. Motion paging: `usePlanning` already loads the complete stream through
   `planResult`; the simulator consumes it directly. **Done — no new code.**
5. `web/src/sim/renderer.ts` + `SimViewport.tsx`: R16F height texture,
   displaced grid with per-pixel normals, border + cliff skirts, tool meshes
   from actual geometry, partial tile uploads, transport bar (play/pause,
   scrub, step, speed 0.25–200×, Z exaggeration, track tool, stage colors).
   **Done; three.js and the worker load as separate chunks only when the
   simulation opens.**
6. App integration: "Stock simulation · 3D" section on the Plan step, center
   viewport swap, availability gating, fixture-mode notice. **Done.**
7. M5 cross-check integration test; `browser-checks.md` update; flower-job
   measurement recorded in §9. **Done (2026-09-08): the cross-check test
   (`web/integration/simulator-crosscheck.test.ts`) plans and verifies the
   curved-medial fixture through a live service and holds the simulator's
   removed area inside every published depth-band interval — including the
   converged bands at ±1.5 mm²; the browser-checks simulator section and the
   flower measurements are recorded. Integrated-GPU fps validation remains
   hardware-pending (an AMD 780M is present but selecting it needs an OS
   per-app graphics preference).**

Live verification (2026-09-07, `cam serve` 0.7.6 + built UI, m4
curved-medial fixture, 565 motions): job → plan → simulation runs with zero
errors; end state 100% applied, 432 mm³ removed (endmill 0.2 + V-bit
0.3 cm³); scrub back to 1:00/3:17 rewinds to 34% applied with 211 mm³ —
the undo log working through the real worker in the app.

Correctness review fixes (2026-09-08, commit `e48d45d`): the V-bit sweep no
longer rejects cells outside the start disc (moving cuts crossed them later,
making End and incremental playback disagree); the pristine-rebuild rewind
bumps cleared tiles so eviction paths emit clearing deltas; sloped V-bit
cuts include the closed-form shifted cone stationary point (previously the
XY projection was assumed minimal even when Z descends along the segment);
and undo entries snapshot the incremental statistics so per-stage volumes
survive rewinds exactly instead of being re-attributed to final owners. All
four carry regression tests, including a brute-force property test for the
sloped-cut minimum.

Flower-scale measurements (2026-09-08, engine 0.7.7 with the sub-3-second
planner, real flower job, 22,893 motions, 0.025 mm cells): simulator startup
after "Open 3D simulation" is ~70 ms (the main thread builds the compact
store once and transfers array copies; cloning motion objects previously
froze the UI ~10 s). The fixture-sized End jump completes in 0.74 s; the
flower End jump at the 0.025 mm cell runs about 1.5–2 minutes of engine
work, executed through the adaptive seek ladder with continuously updating
percentages and surface (no frozen display). A coarser default detail for
very fine V-bit tips (tip/2 instead of tip/4) is the recorded lever should
that jump time matter; normal-speed playback is unaffected because seeks
stay small.

### Phase 3 — extensions (unordered proposals): rapid/approach collision flags
(the heightfield makes these a cheap byproduct); cross-section plane; result
mesh export (STL/GLB); per-operation coloring; zoom-region full-resolution
detail (clipmap-style); refine the display stock rectangle from a loaded
verification report's `domain`.

## 11. Risks

- Large-job TS throughput: resolved by measurement — 55–108 M cells/s,
  120–240× the 15×-implied rate; the WASM fallback is dropped (§9). Playback
  still degrades gracefully if a worker ever lags: the rAF clock is never
  blocked and seeks are backpressured.
- Main-thread zod parse of ~100 MB of motion JSON (flower): resolved by
  measurement — ≈0.8 s projected; worker-side parsing is dropped and the
  existing `planResult` page walk is reused (§9).
- Partial texture uploads during playback: resolved by measurement — the
  public `copyTextureToTexture` CPU-source path works at 0.024–0.087 ms per
  256×256 tile with R16F (§9); direct `texSubImage2D` remains the recorded
  performance fallback. Vertex-stage sampling of normalized R16 returns zero
  on ANGLE/D3D11, which is why heights use R16F.
- Snapshot memory growth on huge jobs: byte-capped with eviction; rewind cost
  is re-application work, never an error.
- Wall fidelity vs mesh density: cliff skirts with threshold and budget;
  V-grooves are sloped by construction.
- iGPU fill/vertex cost: 70 fps measured on the reference RTX 3090 at the
  9.87 M-triangle worst case; the ≥25 fps integrated-graphics gate is still
  unmeasured. Mesh stays capped at 2560 with per-pixel normals doing the
  detail work; chunked culling is the recorded fallback.
- `three` upgrade churn: pinned exact version; upgrades are deliberate
  manifest-reviewed changes, same as every other dependency here.
- Requested speed above the engine's sustained rate: the clock clamps to
  what the engine sustains and shows the effective speed (§5.4); the 15×
  floor is a measured budget with a tested margin, not a hope.
- Scope creep toward verification: resisted by contract — the UI copy and
  this document both fix the simulator as a labeled visual preview.
