# GUI9 — larger jobs and deeper inspection

Status: GUI9a (larger jobs and responsive playback), GUI9b (finer stock
inspection) and GUI9c (robust display under load) implemented and checked;
ready for manual review. Starting commit: `521a00e` (the GUI8 worktree as
committed). GUI8's own user review is still pending; these increments start
from the GUI8 build and repeat the known-carve regression tour, so nothing here
marks GUI8 accepted.

## GUI9a — larger jobs and responsive playback

GUI9a raises the size of the job the display admits, makes a scrub cost the
same backwards and forwards, and makes the display state what it transferred
and which position it is actually showing. It does not change any machining
limit: admission is a display bound, and exceeding it is still refused by name
rather than truncated.

### What the user can now do

- Open, generate and scrub the GUI9 batch fixture — six copies of the accepted
  flower box carving, 137,717 motions — where the previous build refused
  anything above 100,000 motions.
- Scrub backwards and forwards with the same latency. The display restores the
  nearest exact checkpoint in either direction instead of replaying forward
  from wherever the playhead happened to be.
- Read what the display did: the scene payload, its motion pages and
  checkpoints, the size of the last stock update, how many motions that update
  replayed, and — while a request is in flight — the position that was
  requested next to the position still displayed.

### Contracts and implementation

**Admission is a named display bound** (`crates/cam-gui/src/session.rs`). The
whole execution still travels to the display process as one payload: two
28-byte vertices plus a 56-byte replayable motion record is 112 bytes per
motion, and the bounded stock checkpoints add at most `MAX_PREVIEW_BYTES`
(20 MiB). The native worker refuses a response above 128 MB, so the transfer
ceiling is roughly a million motions; `MOTION_LIMIT` is now the measured
display bound (250,000, the plan's M workload plus headroom). `admit_motions`
is the single admission check and its refusal names both the job's size and the
limit. It never weakens a machining limit, and nothing is truncated to fit.

**A seek restores the cheapest exact prefix** (`crates/cam-gui/src/sim.rs`).
`Playback::seek` previously restored a checkpoint only when the target was
*behind* the playhead; a forward jump replayed every motion in between. It now
compares the current position with the nearest checkpoint at or before the
target and restores whichever start is cheaper, in either direction. Both
sources are exact prefix states — transported seeds and fields already visited
— so the result is bit-identical to a cold replay, which
`sim::tests::a_forward_jump_restores_the_nearest_checkpoint_before_replaying`
and the batch harness both check. The same change makes the seek report its own
replay work (`SeekReport { from, replayed }`) instead of leaving it implicit.

**The display accounts for itself** (`crates/cam-gui/src/viewport.rs`,
`app.rs`). The playback bar gained two lines: the scene payload in MB with its
motion pages and checkpoints and the last stock update's transfer size and
replayed motion count, and a "Requested stock motion X of N — still showing
motion Y" line for the interval while a seek is outstanding, so a displayed
field is never attributed to a position it does not hold. Both values are
published in the browser probe snapshot (`requestedStock`, `timelineRows`), and
a failed or refused seek clears the request through
`Viewport::finish_stock_request` rather than leaving the readout stale.

**The timeline lays out a bounded window** (`crates/cam-gui/src/viewport.rs`).
The playback bar laid out one button per executed stage. It now lays out at
most `TIMELINE_WINDOW` (8) rows, with "N earlier stages" / "N later stages"
controls, and re-centres the window on the selected stage. Every review tour
before GUI9 uses only a handful of stages, so the established labels and layout
are unchanged inside the window;
`viewport::tests::the_timeline_lays_out_a_bounded_window_around_the_selected_stage`
pins the bound and the re-centring rule.

### The larger fixture

`fixtures/gui9/flower-box-batch.job.json` is the accepted GUI2 carving — the
same source bytes, cutters, feeds and 1 mm depth — placed as six identical
copies (3 × 2) on one explicitly stated 620 × 210 × 20 mm stock, with one Flat
V-carve operation per copy. It is generated from the committed source job by
`fixtures/gui9/create-batch-fixture.mjs`; the fixture's
[README](../../flat-v-carve/fixtures/gui9/README.md) records the recipe and the
measured plan size. Nothing about the machining intent changes: the batch is
the same cut repeated six times, which is what makes it a comparison of the
display rather than of the planner.

### Measurements

Release build, Windows 11 x86_64, desktop RTX 3090, 1.25× DPI, one process per
run. `cargo test --release -p cam-gui --test large_job --locked -- --ignored
--nocapture` with `CAM_GUI9_MEASURE_OUT` set; raw numbers in
[gui9a-large-job.json](../../flat-v-carve/artifacts/gui/gui9a-large-job.json)
and the run log in `artifacts/gui/gui9a-large-job.txt`. "Before" is the same
harness and the same fixture against the GUI8 `Playback::seek` (a forward jump
always replayed from the current playhead).

| Workload | Motions | Before p50 / p95 / max | Now p50 / p95 / max | Display budget |
| --- | --- | --- | --- | --- |
| Flower box reference | 22,883 | 6.58 / 70.44 / 112.42 ms | 1.61 / 11.57 / 11.68 ms | 250 ms p95 |
| Flower box batch 3 × 2 | 137,717 | 0.57 / 6.84 / 14.13 ms | 0.48 / 0.62 / 4.01 ms | 250 ms p95 |

The reference is the slower case because its display grid is finer (0.3829 mm
cells over a 200 × 100 mm stock, a four-tile grid, against a 0.05 mm reference
resolution) than the batch's (1.2109 mm cells over its stated 620 × 210 mm
stock, two tiles, 0.0757 mm reference), so each replayed motion touches about
ten times as many cells. That is exactly what the before column measures: 32
scrubs that jump forward across the playhead cost up to 112 ms. Restoring the
nearest checkpoint first removes that dependence.

| Transfer (batch 3 × 2) | Value |
| --- | --- |
| Scene payload | 31.9 MiB (33.4 MB) |
| Stock checkpoints | 14.0 MiB (28 frames, 512 KiB each) |
| Replayable motion stream | 7.4 MiB |
| Motion pages | 17 (8,192 motions each) |
| Longest checkpoint interval | 8,608 motions |
| Per-seek stock response | 524,384 bytes (the whole two-tile grid) |
| Display grid | 1.2109 mm cells (requested 0.0757 mm), 2 × 256 KiB tiles |

| Transfer (flower box reference) | Value |
| --- | --- |
| Scene payload | 21.0 MiB (22.0 MB) |
| Stock checkpoints | 18.0 MiB (18 frames, 1 MiB each) |
| Motion pages | 3 |
| Per-seek stock response | 1,048,672 bytes (the whole four-tile grid) |
| Display grid | 0.3829 mm cells (requested 0.05 mm), 4 × 256 KiB tiles |

The 32-scrub pattern transferred 16.0 MiB (16.8 MB) for the batch and 32.0 MiB
(33.6 MB) for the reference. Each response is bounded by the display grid —
`MAX_SIDE` 512 is at most four 256 KiB tiles — and the renderer already copies
only the tiles whose version changed to the GPU. The wire format is not sparse
yet; that is recorded as a limit rather than implied by the GPU-side saving.

### Browser run

The GUI9 review package was driven end to end in real Edge over CDP/WebGPU
against the packaged browser bundle (`node crates/cam-gui/web/smoke.mjs --gui9
--browser=edge --url=…`, log in `artifacts/gui/gui9a-browser-smoke.txt`,
screenshots and `evidence.json` under `artifacts/gui/browser-smoke/`):

| Check | Observed |
| --- | --- |
| Larger fixture opened | 6 operations, 620 × 210 × 20 mm stock (the previous display limit of 100,000 motions is recorded next to it) |
| Larger generation admitted | 137,717 motions from the WASM Worker, export-ready |
| Windowed timeline and forward scrub | 8 stage rows for 12 stages, final stock prefix 137,717, last stock transfer 524,384 bytes, 0 motions replayed (the target is a transported checkpoint) |
| Small reference unchanged | 22,883 motions in the same session |

The browser run is software evidence for the packaged artifact, not a
performance measurement: the request/paint timings in this milestone were
measured natively, and browser frame timing is not claimed here.

### Native end-to-end smoke

The native half of the same workflow is scriptable because it is the
application's own worker path: `scripts/native-smoke.mjs` starts the review
executable with `--worker <mailbox>` and sends the same framed requests the
native UI sends. Report `artifacts/gui/gui9-native-smoke.json`, log
`artifacts/gui/gui9-native-smoke.txt`; 17 checks, all passing on the release
review build.

| Step | Observed (release native) |
| --- | --- |
| Apply machine profile | Applied; the document keeps its own machining values |
| Generate the batch at Standard | 137,717 motions in 19.9 s, 33.4 MB payload, 17 motion pages, 28 checkpoints, cell 1.2109 mm, key `51c8836d…` |
| Seek to 68,858 | 26 ms, 524,384 bytes, 0 motions replayed |
| Rebuild the display at Fine | 123 ms, cell 0.6055 mm, 56 MiB ladder, new key `99d098dd…`, playhead 68,858 preserved |
| Seek after the rebuild | 48 ms and still on the new key |
| Prepare the checked program | 2.6 s, `plansRun: 1` (no replanning), `sequence.ngc`, sha256 `cb5b672d…` — byte-identical to the same run on the debug build |

This is the implementing agent's end-to-end smoke run for the native target:
real executable, real worker process, real mailbox, real retained service. It
does not involve a window, a GPU or input devices, so the interactive tour in
[the review recipe](gui9-review.md) is still the only way to judge feel.

### Evidence and acceptance audit

| GUI9a requirement | Evidence |
| --- | --- |
| Open a larger real carving without truncation | `tests/large_job.rs` generates the batch fixture (137,717 motions) and asserts the count, the 17 motion pages and the 28 checkpoints; `a_plan_above_the_display_motion_limit_is_refused_by_name` pins the refusal at the bound |
| Scrub backwards and forwards without freezing | The harness runs 32 seeks (12 forward, 12 backward, 8 jumps) and asserts the p95 stays inside the plan's 250 ms M-scrub budget; the seek targets are asserted against the returned frame |
| Nearby checkpoints already bound the work | `checkpointIntervals` are recorded per fixture; the batch's longest is 8,608 motions |
| Identical stock after a checkpoint restore | The harness replays one transported seek prefix from the pristine field at the display cell and requires identical packed cells; `sim::tests::a_forward_jump_restores_the_nearest_checkpoint_before_replaying` covers the restore rule |
| Bounded paging and checkpoints | Preview frames stay inside `MAX_PREVIEW_BYTES`; motion pages, page count and checkpoint count are reported in `TransportMeta`; `paging::tests` cover eviction, fingerprints and budget omission |
| Transfer accounting | `TransportMeta` on every scene, the new per-seek size and replay count on the playback bar, `stock_render::Stats` for GPU-side tile copies, and the harness report |
| Latest-request scheduling | `app::tests::display_requests_coalesce_and_late_answers_are_ignored`: three scrub targets submit only the latest, two resolution choices rebuild only the latest, and a completion for another request id is ignored instead of shown |
| Virtualization | `Viewport::timeline_window` bounds the playback bar to 8 stage rows and publishes `timelineRows` in the probe snapshot; the unit test pins the window |
| A partial scene says so | When the resident budget refuses requested path pages, the playback bar states how many are not drawn (and the probe publishes `pagesOmittedByBudget`); the stock field and timeline are unaffected |
| Exceeding a limit stays explicit | `Session::admit_motions` refuses by name above `MOTION_LIMIT`; `stock_preview` reports dropped stage boundaries; the worker refuses a response above 128 MB |
| The small workflow still works | The same harness measures the 22,883-motion reference through the same path; the full `cam-gui`, `cam-core` and `cam-service` suites and the GUI2–GUI8 tours are unchanged |
| The shipped package behaves the same way | The real-browser run above opens the batch fixture, admits 137,717 motions, shows the bounded stage row and reaches the final stock prefix in the packaged bundle |
| Changing a value and regenerating | The recipes now prove the loop rather than a pre-rendered result: Fixture A step 7 changes one copy's maximum depth and regenerates (137,717 → 136,277 motions and back), and the browser run edits the cutting feed during a generation and regenerates from the edited document |

### Checks

- `cargo test -p cam-gui -p cam-core -p cam-service --locked`: every library and
  integration suite passes, including the new `tests/large_job.rs` admission
  test and the `sim`/`viewport` unit tests. Log:
  `artifacts/gui/gui9a-tests.txt`.
- `cargo test --release -p cam-gui --test large_job --locked -- --ignored
  --nocapture`: the GUI9a evidence run. Log:
  `artifacts/gui/gui9a-large-job.txt`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` and
  `cargo fmt --all -- --check` pass. Logs: `artifacts/gui/gui9a-clippy.txt`,
  `artifacts/gui/gui9a-fmt.txt`.
- `node crates/cam-gui/web/smoke.mjs --gui9 --browser=edge` against the review
  package: the browser workflow above passes. Log:
  `artifacts/gui/gui9a-browser-smoke.txt`.
- Native launch smoke: `artifacts/gui9/review/large-job-native/cam-gui.exe`
  starts, stays alive for ten seconds and closes with exit code 0, leaving no
  `cam-gui` process behind. This is a launch check, not the interactive tour —
  the manual recipe is the first full native run.

`artifacts/gui9/review/manifest.json` records which sources and packages the
review build was made from (`0cae4485…` native, `255cfe08…` WASM, offline bundle
`d289669f…`, and the 296 hashed source files (now including
`scripts/native-smoke.mjs`), covering the GUI9b and GUI9c
changes).

### Limits

- **One payload per execution.** The whole generated execution still travels
  to the display process as one message. `MOTION_LIMIT` is 250,000 motions and
  the plan's L workload (1,000,000) is not admitted; it is refused by name,
  not truncated. Making L fit needs the execution itself to be paged, which
  this increment does not do.
- **Per-seek transfer is the whole grid.** Every seek still ships the packed
  tile grid (512 KiB for the batch, 1 MiB for the reference) because the wire
  format has no sparse form yet, even though the GPU copies only changed tiles.
  The measured sizes are in the table above so the cost is visible.
- **The operation list is not windowed.** The timeline stage row is bounded,
  but the navigator still lays out one row per operation. The batch fixture has
  six operations, so this increment does not measure a large operation list;
  GUI11's whole-product usability pass owns it; this milestone does not claim
  it.
- **Checkpoint count is a budget, not a policy.** The ladder is the historical
  evenly spaced set plus stage boundaries, capped by `MAX_PREVIEW_BYTES`. A job
  whose stage boundaries crowd the budget drops the oldest boundaries and says
  so, as GUI8c recorded.
- **No physical claim.** Nothing here measured a cut surface. The fixture, the
  simulation and the transport are software evidence.

## GUI9b — finer stock inspection

The display raster is now a choice, not a constant, and the inspector can show
what that choice resolved. Changing the resolution re-derives the raster from
the *same* retained execution at the *same* playhead; it never replans and no
machining value moves. The inspector gained a cross-section through the
inspected point, and the panel states when the display is coarser than the
resolution the plan asked for.

### What the user can now do

- Pick **Coarse / Standard / Fine** from the playback bar. The raster is
  rebuilt from the retained execution, the cell size and retained checkpoint
  bytes are stated, and the playhead does not move — so the same cut can be
  compared at two resolutions without regenerating anything.
- Read the display's own cross-section through the inspected XY: **Section X**
  or **Section Y**, drawn from the displayed cell centres, with a one-cell
  scale bar, the depth quantum and the number of cells cut.
- See the resolution limit as a fact: for the batch fixture the plan requests
  0.0757 mm and the display shows 1.2109 mm (Standard) or 0.6055 mm (Fine), so
  the panel says the raster is coarser, that features narrower than one cell
  may not appear, and that the generated profile/tab boundaries drawn in the
  viewport are exact and are not rasterized.

### Contracts and implementation

**One simulation key per raster** (`crates/cam-gui/src/stock_preview.rs`).
`simulation_key` folds the execution identity, `DISPLAY_ALGORITHM_VERSION`, the
preset, the resulting cell size and grid, the stock rectangle and the tool
geometry into a 16-hex-digit key, and `PreviewMeta` carries it next to
`preset`, the plan's `reference_cell_mm` and the retained `ladder_frames`. The
stock renderer now keys its resident tiles on *that* key
(`PreviewMeta::identity`) rather than the scene payload hash, so a resolution
change re-uploads every tile while the motion pages, which belong to the
execution, stay resident. A checkpoint or tile from another key is never
reused just because the stock rectangle matches — the case the plan's
`SimulationKey` paragraph names.

**The rebuild is a typed command, not a regeneration**
(`crates/cam-gui/src/session.rs`). `Command::Generate` and
`Command::ValidatePlan` carry the preset the session asked for, and
`Command::DisplayPreset { handle, preset }` re-derives the raster inside the
worker from the retained plan: the same `Input`, the same stage-boundary
marks, the same prefix. The display keeps the stock, tools, requested
resolution and marks it needs for that rebuild, seeds a fresh bounded
`Playback` at the preset's budget, seeks back to the playhead it was at, and
returns one frame plus the new key. The application refuses a response whose
handle is not the plan on screen, and the viewport keeps the desired preset
separate from the shown one, so a restored working context requests its saved
resolution instead of silently showing another.

**A section is display evidence, not a measurement**
(`crates/cam-gui/src/viewport_inspection.rs`). `Viewport::section` samples one
cell centre per displayed cell along X or Y through the inspected point,
returns nothing when the point is off the stock, and the panel draws the stock
top, the removed material, the inspected cell and a one-cell scale bar. It
states the sample count, the cell size and the depth quantum, and the code
comment says plainly that it never feeds anything back into the engine.

### Measurements

The same release harness as GUI9a, now also rebuilding the batch fixture's
raster at every preset (`artifacts/gui/gui9a-large-job.json`,
`resolutions`). 137,717 motions, one retained execution, no replanning:

| Preset | Cell | Requested | Checkpoints | Ladder retained | Response | Rebuild |
| --- | --- | --- | --- | --- | --- | --- |
| Coarse | 2.4219 mm | 0.0757 mm | 28 | 7 MiB | 0.25 MiB | 18 ms |
| Standard | 1.2109 mm | 0.0757 mm | 28 | 14 MiB | 0.50 MiB | 37 ms |
| Fine | 0.6055 mm | 0.0757 mm | 28 | 56 MiB | 2.00 MiB | 109 ms |

Every ladder stays inside its preset budget (8 / 20 / 64 MiB), the three keys
are distinct, and the playhead is unchanged by the rebuild. The rebuild is fast
because it re-integrates the retained motion stream once, at a coarse display
grid, instead of replanning: 109 ms for the finest preset on a 137,717-motion
job.

The real-browser run repeated it on the packaged bundle (log
`artifacts/gui/gui9b-browser-smoke.txt`, evidence under
`artifacts/gui/browser-smoke/`): Standard → Fine moved the cell from
1.2109 mm to 0.6055 mm at the same playhead (137,717), reported the 28-frame /
56 MiB ladder and the new key `3a35e70f…`, and the section read 1024 cell
centres along X and 347 along Y through the inspected point.

The GUI9c run on the same package added the load and recovery records:

| Check | Observed |
| --- | --- |
| Bounded display upload | 17 motion pages copied over 6 frames with 0 deferred at rest; stock tiles over 1 frame; 7.7 MB resident |
| Edited while generating | The generation was running when the edit landed (`busyDuringEdit: true`), the revision moved 2 → 3, the camera change was kept, and the running generation did not become the current result |
| Renderer failure and recovery | 2 resource recreations, playhead 137,717 preserved, preset `fine`, simulation key unchanged, revision unchanged, 17 pages re-copied |
| Cancelled a running generation | Revision unchanged, document kept, worker stop reported in 0.10 ms, and the next generation in the same session produced the 22,883-motion reference |
| Sustained scrub burst | 20 rounds of alternating stage jump and Start (40 coalesced seek requests): 120 sampled frames, p50 19.7 ms, p95 96.6 ms, max 249.8 ms — these frames carry the seek responses and stock uploads, so they are reported separately from camera frames |
| Camera-only frames (plan §2.5 M target: p95 ≤ 33 ms) | 120 sampled frames while the camera moved over the settled scene: p50 15.7 ms, p95 16.3 ms, max 17.3 ms, with **0** page uploads and **0** uploaded bytes |
| Idle behavior (plan §2.5: at least 30 s with no animation, task or input) | 30 s observed at a fixed playhead: **0** additional page uploads, **0** additional uploaded bytes, playhead unchanged |
| Build identity | The run records `cam-gui 0.7.7` on protocol `cam-gui-retained-5`, so the numbers name the build they came from |

Native outcome, recorded separately: the same tour through the shipped
executable's worker mailbox (generate → seek → resolution rebuild → seek →
prepare) passes with the numbers in the native-smoke table above. The browser
and native runs agree on the plan, the simulation keys and the emitted
program's SHA-256.

### Evidence and acceptance audit

| GUI9b requirement | Evidence |
| --- | --- |
| Choose a display resolution and inspect at the same playhead | `tests/inspection.rs::another_display_resolution_rebuilds_the_same_execution_at_a_new_key` (cell shrinks, playhead kept, the retained plan keeps its fingerprint); the browser run above |
| Simulation-key changes invalidate checkpoints and tiles | `stock_preview::tests::the_simulation_key_separates_executions_and_display_resolutions` (same rectangle, different key per preset and per execution); `PreviewMeta::identity` is what the stock renderer keys tiles on |
| Checkpoint invalidation stays inside a budget | The rebuild asserts `retained_bytes <= DisplayPreset::…budget()` and reports the ladder; `stock_preview` still drops surplus stage boundaries and says how many |
| Labelled approximation | The inspection panel prints the display cell against the plan's requested cell and the multiple between them, and says features narrower than a cell may not appear |
| Section views | `viewport::inspection::tests::the_section_samples_the_displayed_raster_in_both_axes` (both axes, the inspected cell marked, intact stock reported as intact, off-stock has no section); the browser run reads both axes |
| Generated overlays stay exact | `tests/inspection.rs::a_profile_with_tabs_inspects_at_a_finer_resolution_without_changing_the_plan` builds the lettering profile with automatic tabs, changes only the display resolution, and requires the same motion count, the same plan handle and identical generated tab placements; GUI8's anchors, bridges and knife traces are unchanged and drawn from plan data |
| No raster-to-CAM feedback | The section samples display cells only; the preset command returns stock cells, never a plan, and never touches export eligibility |
| The small workflow still works | The full `cam-gui`, `cam-core` and `cam-service` suites pass, and the browser run regenerates the 22,883-motion reference in the same session |

### Checks

- `cargo test -p cam-gui --locked`: 79 library tests plus every integration
  suite pass, including the new `tests/inspection.rs`. Log:
  `artifacts/gui/gui9b-tests.txt`.
- `cargo test -p cam-core -p cam-service --locked`: every suite passes. Log:
  `artifacts/gui/gui9b-core-tests.txt`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` and
  `cargo fmt --all -- --check` pass. Logs: `artifacts/gui/gui9b-clippy.txt`,
  `artifacts/gui/gui9b-fmt.txt`.
- `cargo test --release -p cam-gui --test large_job --locked -- --ignored
  --nocapture` measures the three resolutions. Log:
  `artifacts/gui/gui9a-large-job.txt`.
- `node crates/cam-gui/web/smoke.mjs --gui9 --browser=edge` against the review
  package: the GUI9b steps pass. Log:
  `artifacts/gui/gui9b-browser-smoke.txt`.

### Limits

- **The raster is still capped.** Fine is the finest preset (1024 cells across
  the longest side); for the batch fixture that is 0.6055 mm against the plan's
  requested 0.0757 mm. Unbounded raster resolution is not offered because the
  retained ladder grows with the square of the cell count; the section view and
  the exact overlays are the answer to sub-cell detail, not a bigger grid.
- **A section is a display sample.** It reads cell centres of the raster, so it
  inherits the cell size and depth quantum it states. It is not a tolerance
  check and cannot be exported or fed back into the plan.
- **Changing the resolution costs the retained ladder.** Fine retains 56 MiB
  for the batch fixture (bounded by the preset), and a job whose stage
  boundaries crowd that budget drops the oldest boundaries and says so.
- **The rebuild is explicit.** A restored working context requests its saved
  resolution once a plan exists; until then the panel states the preset it will
  build at rather than pretending the raster already has it.

## GUI9c — robust display under load

The display no longer does unbounded work in a single frame, a busy worker no
longer swallows the user's next action, and a renderer failure has a reachable
drill that recovers from retained CPU data without disturbing the document, the
retained result or the view.

### What the user can now do

- Keep working while a heavy display task runs. A cold page or stock load is
  copied in bounded batches over several frames; the camera, the inspector and
  the document keep responding, and the display asks for the next frame itself
  instead of dropping the rest of the load.
- Edit while a rebuild runs. The edit is a document change and is kept; the
  rebuild is a display change and finishes without reverting it, and the
  following validation runs when the worker is free.
- Click something while the worker is busy and have it happen: an action that
  can be rebuilt from the current document (operation, artwork, resource edit
  or generation) is queued — one slot, newest wins — and runs when the worker
  is free, with the status line saying what it is waiting for. An action that
  cannot be rebuilt is refused out loud rather than dropped in silence.
- Cancel a running calculation (**Cancel** next to the spinner) and keep
  working. The worker stops, the status reports it, and the document, the last
  displayed result and any checked bundle already read back are still there;
  the display is marked stale because its retained execution died with the
  worker, and a queued action still runs. Generate again and the same session
  continues.
- Inject a renderer failure and recover from it. **Inspect result → Renderer
  diagnostics → Inject renderer failure** stops the viewport's GPU drawing and
  says so; **Rebuild renderer resources** recreates the pipelines and buffers
  from retained CPU data and re-copies the resident pages and stock tiles. The
  document, the retained result and the plan handle survive, and the playhead,
  display resolution, simulation key and camera come back unchanged.

### Contracts and implementation

**Bounded uploads** (`crates/cam-gui/src/paging.rs`,
`stock_preview.rs`, `render.rs`, `stock_render.rs`). `Pager::plan` gained a
per-frame copy budget (`render::PAGE_UPLOAD_BUDGET_PER_FRAME`, four pages);
pages it cannot copy this frame are counted as *deferred*, not omitted, so the
next frame asks for them again and the load still finishes with every page
copied exactly once. The stock renderer copies at most
`stock_preview::TILE_UPLOADS_PER_FRAME` changed tiles per frame through the
headless `next_tile_batch` policy, and both sides publish `pages_deferred` /
`pending_tiles` plus a `frames_loading` counter. The viewport requests another
frame while anything is deferred.

**Bounded events** (`crates/cam-gui/src/app.rs`). One frame consumes at most 64
worker events. A burst finishes on the following frames instead of starving the
frame, and the rest stay in the channel — no terminal result is dropped, and
the repaint is requested immediately.

**Frame, backend and memory instrumentation** (`crates/cam-gui/src/viewport.rs`,
`viewport_inspection.rs`). The diagnostics sample the interval between frames
that did real work (gaps above 250 ms are the legitimate silence of an
event-driven application, not a stall, and are not sampled) and publish
`frameMs {samples, last, p50, p95, max}` together with the adapter identity,
the application's build identity (package version and the retained protocol),
the required page range, resident pages and bytes, cumulative page uploads,
upload bytes, cache hits (pages whose fingerprint matched and were skipped) and
evictions, plus the scene/checkpoint/GPU/stock-tile memory categories. The panel
states all of it, so a review does not have to guess what the display is doing.

**No silently dropped actions** (`crates/cam-gui/src/app.rs`). A command that
arrives while the worker is busy is no longer ignored. `Pending` holds one
*intent* — operation, artwork or resource edit, or a generation — rebuilt from
the current document when it runs, so a queued action never replays the stale
snapshot that was refused. A second action replaces the first (a queue of stale
edits would be worse than doing the latest thing asked for), and the status
line states what is queued and why. Commands that cannot be rebuilt are refused
with a status message.

**A real recovery path** (`crates/cam-gui/src/render.rs`,
**The saved view keeps the chosen resolution** (`crates/cam-gui/src/viewport.rs`,
`resume.rs`). The working-context snapshot used to persist whatever raster was
*displayed*, so a context saved before its first generation lost the resolution
the user had chosen. `ViewSettings.preset` now comes from the desired preset,
and the resume test asserts the whole view — camera, zoom, yaw, stage, stock,
inspection point and resolution — round-trips through the recovery snapshot
rather than only the camera flag.

**A real recovery path** (`crates/cam-gui/src/render.rs`,
`stock_render.rs`, `viewport.rs`, `viewport_inspection.rs`). The drill that GUI1
built was unreachable from the shipped UI; **Renderer diagnostics** now exposes
it. Injecting a failure sets the same `gpu_unavailable` state the viewport
already refused to draw through, and recovery runs the existing `Drill::Recover`
path: pipelines, bind groups and buffers are recreated and `Pager::forget`
forces every page to be re-copied from the retained payload. Nothing on the CPU
side is rebuilt, so the document and the view are provably untouched.

### Evidence and acceptance audit

| GUI9c requirement | Evidence |
| --- | --- |
| Bounded uploads, nothing dropped | `paging::tests::the_frame_budget_spreads_a_large_load_over_several_frames` (two pages per frame, the rest deferred, the load finishes, a reload copies nothing) and `stock_preview::tests::stock_uploads_are_batched_per_frame_without_losing_tiles`; the browser run reports 17 motion pages over 6 frames with 0 still pending |
| Bounded events | `App::poll` consumes at most 64 events per frame and asks for the next frame; the worker channel keeps the rest |
| Editing during a heavy display task | The browser run edits the document while the display rebuild is in flight and asserts the revision advanced and the rebuild did not revert it |
| An action during a busy worker is not lost | `app::tests::a_command_that_arrives_while_busy_is_queued_once_and_replaced_by_the_newest` (queued, bounded to one, newest wins, a non-rebuildable command is refused out loud) and `app::tests::a_queued_generation_uses_the_current_document_and_resolution` (rebuilt from the current document, and not run at all while raw text is uncommitted) |
| Latest display request wins | `app::tests::display_requests_coalesce_and_late_answers_are_ignored`: scrub and resolution requests coalesce to the newest, and a completion for another request id is ignored |
| Renderer failure and recovery preserve document and identity | `app::tests::the_renderer_drill_preserves_the_document_result_and_view`; the browser run injects a failure and recovers with the same playhead, revision, simulation key and view, and the resident pages re-copied (recoveries 2, 17 pages) |
| Re-opening a saved view keeps the whole view | `app::resume::tests::recovery_keeps_raw_text_navigation_history_and_no_artifact_authority` now compares the full `ViewSettings` after a snapshot/restore round-trip, including the chosen display resolution |
| Cancel keeps what the user owns | `app::tests::cancelling_a_computation_keeps_the_draft_and_the_checked_bundle` (document, checked bundle and queued action survive; the dead plan handle is dropped); the browser run cancels a running generation and generates again in the same session (revision unchanged, document kept, worker stop 0.10 ms) |
| Resource lifecycle | Recovery goes through the existing recreate paths (`render::Resources::recreate`, `stock_render::Resources::recreate`), which reset resident versions so the next frames re-upload instead of trusting lost buffers |
| Instrumentation exists | `renderer_probe` publishes frame percentiles, backend identity, resident pages/bytes, page uploads/bytes, evictions and the memory categories; the diagnostics panel shows them |
| Actual platform evidence | The real-browser run above (Edge 153, NVIDIA Ampere) with camera-frame p95 inside the plan's 33 ms target and zero camera-time copies, plus the release measurement run and the native launch smoke recorded in the GUI9a section |

### Checks

- `cargo test -p cam-gui --locked`: 86 library tests plus every integration
  suite pass, including the new `app::tests` queue, cancel and drill tests. Log:
  `artifacts/gui/gui9c-tests.txt`.
- `cargo test -p cam-core -p cam-service --locked`: every suite passes. Log:
  `artifacts/gui/gui9c-core-tests.txt`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` and
  `cargo fmt --all -- --check` pass. Logs: `artifacts/gui/gui9c-clippy.txt`,
  `artifacts/gui/gui9c-fmt.txt`.
- `node crates/cam-gui/web/smoke.mjs --gui9 --browser=edge` against the review
  package: the load, drill, recovery and edit-under-load steps pass. Log:
  `artifacts/gui/gui9c-browser-smoke.txt`.
- `node scripts/native-smoke.mjs` against the review executable: 17 checks
  pass through the application's own worker mailbox (generate, seek, resolution
  rebuild, seek, prepare). Report: `artifacts/gui/gui9-native-smoke.json`, log
  `artifacts/gui/gui9-native-smoke.txt`.

### Limits

- **The queue holds one action.** A second action replaces the first; a user
  who wants two edits queued behind a long generation must make them in order.
  The status line always says which one is waiting.
- **Cancel ends the retained execution.** The worker that holds the plan is
  stopped, so the display is marked stale and the next Generate builds a new
  execution. The document, the last displayed result and any checked bundle
  already read back survive; a cancelled *preparation* still discards the
  prepared bundle, exactly as it did before.
- **The drill is a diagnostic, not a device-loss simulation.** It stops the
  viewport's custom callbacks and rebuilds their resources; it does not
  destroy the wgpu device or the egui renderer, and it makes no claim about a
  real driver reset. GUI11 owns whether such a control ships.
- **Bounded uploads trade latency for smoothness.** A cold load now takes
  several frames (6 frames / 17 pages for the batch fixture) instead of one
  long frame; the counters are visible in the diagnostics.
- **GUI11 owns the full qualification sweep.** This milestone records
  single-run windows with their sample counts (120 frames for camera and scrub,
  a 30-second idle observation). The plan's five-repetition rule and the
  sustained two-minute M playback/pan exercise belong to the GUI11 release
  qualification, on release artifacts from a clean checkout.
- **No physical claim.** Nothing here measured a cut surface.
