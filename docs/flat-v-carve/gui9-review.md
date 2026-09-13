# GUI9a manual review recipe

This review covers larger jobs and responsive playback. Findings here are
software observations; no machine is controlled and no surface-quality claim is
made.

What is new: the display admits a job with 137,717 motions (six copies of the
accepted flower box carving) where the previous build refused anything above
100,000; a scrub costs the same backwards and forwards because the display
restores the nearest exact checkpoint in either direction; the playback bar
states the scene payload, its motion pages and checkpoints, and the size and
replay work of the last stock update; while a request is in flight it names the
position that was requested next to the position still displayed; and the
stage row lays out a bounded window instead of one button per stage.

GUI9b adds two things on top: **Display: Coarse / Standard / Fine** rebuilds
the raster from the *same* retained execution at the *same* playhead — no
regeneration — and the **Inspect result** tab now draws the display's own
cross-section through the inspected point (Section X or Section Y) with the
cell size as a scale bar, and says when the raster is coarser than the
resolution the plan asked for.

GUI9c adds the load behavior and the recovery drill: a cold page or stock load
is copied in bounded batches over several frames instead of one long frame, an
action taken while the worker is busy is queued (one slot, newest wins) instead
of being dropped, and **Inspect result → Renderer diagnostics** can inject a
renderer failure and rebuild the GPU resources from retained data without
touching the document, the retained result or the view.

What is limited: the whole execution still travels as one payload and 250,000
motions is the admitted bound (a million is refused by name, never truncated);
each seek still transfers the whole checkpoint grid even though only changed
tiles reach the GPU; the display raster is capped at 1024 cells across the
longest side (Fine), so it can still be coarser than the plan's requested
resolution, and a section reads the raster rather than measuring a real
surface; the navigator's operation list is still unbounded. The full limit list
is at the end of this recipe.

What still works: every earlier workflow is unchanged in this build — the Flat
V-carve loop with its combined endmill/V-bit stages, the artwork collection and
placement, the tool library and machine configuration, the passive drag knife,
facing with ordered preparation and the closed Profile with tabs, finishing and
entries (GUI2–GUI8).

## Launch

Native (recommended first):

```powershell
flat-v-carve\artifacts\gui9\review\large-job-native\cam-gui.exe
```

Browser (offline package):

```powershell
cd flat-v-carve
node artifacts/gui9/review/serve-package.mjs
# open http://127.0.0.1:5183/web/index.html in desktop Chromium
```

`artifacts/gui9/review/manifest.json` records which source files and packages
these executables were built from (native SHA-256, WASM SHA-256 and the offline
bundle version).

The automated native end-to-end run that backs this build is reproducible:

```powershell
cd flat-v-carve
node scripts/native-smoke.mjs            # writes artifacts/gui/gui9-native-smoke.json
```

It drives the shipped executable's own worker mailbox (apply machine profile →
generate the batch → seek → rebuild the display at Fine → seek → prepare the
checked program) and prints its checks. It does not open the window, so the
tour below is still what tells you how the application feels.

## Fixtures

| Fixture | Use |
| --- | --- |
| `flat-v-carve/fixtures/gui9/flower-box-batch.job.json` | The larger job: six copies of the flower box carving, 137,717 motions, 620 × 210 × 20 mm stock. |
| `flat-v-carve/fixtures/gui2/flower.job.json` | The small reference (also reachable as **File → Flower fixture**): 22,883 motions. |
| `flat-v-carve/fixtures/gui2/machine.json` | Applied through **Machine → Example machine → Apply flower machine profile** so the cutters have a controller mapping. |
| `flat-v-carve/fixtures/gui4/lettering.job.json` | Regression tour for the ordered/profile workflows. |
| `flat-v-carve/fixtures/gui3/lettering.svg` | Source for the profile and tab fixtures in the GUI8 recipe. |
| `flat-v-carve/fixtures/gui5/library.json` | Tool library used by the GUI8 cutter picker steps. |
| `real_data/flower_box-svg.job-real.json` | The user's real carving, for the regression tour. |

### Fixture identities

The `sha256` column is the first twelve hex digits of the file hash, so a review
can prove it used the same inputs as the recorded runs. The fixture identity and
the whole review package are hashed again in
`artifacts/gui9/review/manifest.json`.

| Fixture | sha256 (12) | Expected |
| --- | --- | --- |
| `fixtures/gui9/flower-box-batch.job.json` | `f794c815b0b5` | 6 operations, 620 × 210 × 20 mm stock, 137,717 motions on generate |
| `fixtures/gui9/create-batch-fixture.mjs` | `b63accf6417e` | Regenerates the fixture deterministically |
| `fixtures/gui2/flower.job.json` | `ed06e471bee6` | 22,883 motions |
| `fixtures/gui2/machine.json` | `f82861b03ae9` | Applies without editing the job's machining values |
| `fixtures/gui4/lettering.job.json` | `7ad1e60f37e9` | Opens with its applied machine configuration |
| `fixtures/gui3/lettering.svg` | `3b69c4f90e5b` | Three closed contours (two outers, one hole) |
| `fixtures/gui5/library.json` | `5e991c119623` | Library with the endmill and lettering rough profile |
| `fixtures/gui6/knife.job.json` | `e834a0585bd0` | Knife-only job for the GUI6 regression tour |
| `real_data/flower_box-svg.job-real.json` | `80dd0208cec4` | The known real carving |

Expect the full tour (Fixtures A–D plus the short regression tour) to take
roughly five to ten minutes of clicking, plus about 20 seconds for each batch
generation on the development machine.

## Fixture A — the larger carving (GUI9a)

1. **File → Open job…** and choose
   `flat-v-carve/fixtures/gui9/flower-box-batch.job.json`. The navigator lists
   six Flat V-carve operations, one per copy, and the status line reports the
   applied machine profile is still missing.
2. **Machine → Example machine → Apply flower machine profile**, then
   **Generate**. Generation is slower than the single copy — about 20 seconds
   on the development machine — because six carvings are planned for real. The
   status reports the plan's motion count; it must be **137,717** motions. (The
   previous build stopped here with *“GUI2 exceeds 100,000 motions; no
   truncated scene admitted”*.)
3. Read the two lines under the **Stock motion** slider before touching it:

   ```
   Display simulation · 1.2109 mm cells (reference 0.0757) · 137717 / 137717 motions · …
   Display transfer · scene 31.9 MiB · 17 motion pages · 28 checkpoints (14.0 MiB retained) · last stock update 0 KiB after replaying 0 motions
   ```

   The first states the resolution actually displayed next to the resolution the
   plan asked for, and the checkpoint/page accounting. The second is the
   transport this display is responsible for.
4. Press **Start**, then drag the **Stock motion** slider quickly from left to
   right and back. While a request is outstanding the bar adds

   ```
   Requested stock motion N of 137717 — still showing motion M.
   ```

   so the field on screen is never read as the position you asked for. When the
   response lands, the prefix and the "last stock update" size/replay count
   update together.
5. Scrub to the end (**After v-bit (6 of 6)**, reached through
   **4 later stages ▶** when it is outside the visible window, or use the
   slider), then jump back to 10%, then forward again to 60%.
   Every jump must land on the requested motion (the slider and the line above
   it agree), the display must not freeze, and the backward and forward passes
   must feel the same. Before this build a forward jump replayed every motion
   between the playhead and the target, which is what the measured 112 ms worst
   case in the reference job was.
6. **Change a machining value and regenerate** (the contract asks the recipe to
   prove the loop, not just open a rendered result). With the batch job
   generated, select the first Flat V-carve operation, set **Cutting → Maximum
   depth** to `2` and press **Generate**. The status marks the display stale
   while it works, then reports the new plan: that copy plans 21,443 motions
   instead of 22,883 (the same edit on the single-copy reference moves 7,048
   endmill and 15,835 V-bit motions to 4,414 and 17,029), so the batch reports
   136,277. The cut is deeper because the operation's maximum depth is larger,
   and the simulated stock follows it. Set the value back to `1`, **Generate**
   once more, and the counts return to 137,717. Nothing about the artwork, the
   machine profile or the other five copies is edited by this.
7. Repeat step 4 on the small reference (**File → Flower fixture**, apply the
   machine, **Generate**). The small workflow is unchanged: three motion pages,
   eighteen checkpoints, and the same controls.

## Fixture B — the bounded stage row (GUI9a)

1. Reopen the batch job (**File → Open job…**, the GUI9 fixture), apply the
   machine profile and **Generate** if it is not still loaded, then look at the
   playback bar. Twelve stages exist
   (six carvings × rough/finish), so the bar lays out eight stage buttons with
   **◀ N earlier stages** and **N later stages ▶** around them.
2. Press **◀ 4 earlier stages** / **4 later stages ▶** to move the window. The
   stage selector (the combo box to the right) still lists every stage and still
   jumps to any of them; selecting one outside the window re-centres the row.
3. Every stage button keeps its established label ("After endmill (3 of 6)",
   "After v-bit (3 of 6)", …) and every one seeks to the end of its stage.

## Fixture C — display resolution and sections (GUI9b)

Continuing with the batch job generated and simulating (Fixture A):

1. In the playback bar, open **Display resolution** and pick
   **Fine · 1024 cells across · 64 MiB checkpoints**. The status reports the
   raster was rebuilt for the same execution, and the display line changes:

   ```
   Display simulation · 0.6055 mm cells (reference 0.0757) · 137717 / 137717 motions · …
   Display transfer · scene 31.9 MiB · 17 motion pages · 28 checkpoints (56.0 MiB retained) · …
   ```

   The playhead does not move: this is the same cut, at another resolution. The
   stage row and the motion pages are unchanged — only the raster and its
   checkpoints were re-derived. Compare **Display: Coarse** too: the cells get
   larger and the retained ladder shrinks.
2. Scrub after the change. Seeks still answer at the new resolution, and the
   requested/still-showing line behaves exactly as in Fixture A.
3. Press **Simulate**, then open the **Inspect result** tab. Type
   `X 310`, `Y 105` (the middle of the batch stock). The panel now shows a
   **Section X** plot: the stock top, the removed material, the inspected cell
   marked in yellow, and a scale bar that is exactly one display cell. Below it:

   ```
   1024 cell-centre samples from 0.303 mm · scale bar is one 0.6055 mm cell · …
   ```

   Press **Section Y** for the other axis. Comparing the two at the same
   playhead is the point: the section is what the chosen resolution resolved.
4. Repeat step 3 after switching back to **Coarse**. The plot gets visibly
   coarser and the sample count drops with the grid; the material the raster
   missed is exactly what the coarse cells cannot show.
5. Open a Profile job with tabs (Fixture B of
   [the GUI8 recipe](gui8-review.md)) and scrub to a pass that holds a bridge.
   The drawn tab boundary is unchanged by the display preset — it comes from
   the plan, not from the raster — and the inspection panel says so. The
   automated check behind this is
   `tests/inspection.rs::a_profile_with_tabs_inspects_at_a_finer_resolution_without_changing_the_plan`.

## Fixture D — load, edit and recovery (GUI9c)

Continuing from Fixture A (the batch job is generated and simulated), or from a
fresh open of the batch fixture:

1. **Watch a cold load finish in bounded pieces.** As soon as a scene appears,
   **Inspect result → Renderer diagnostics** reports `N page(s) deferred` /
   `M stock tile(s) pending` while the load continues, and settles to zero. The
   display asks for the next frame itself; nothing stalls and nothing is
   dropped. Switch to **Display: Fine** and repeat: more tiles, more frames,
   same finish.
   The same panel shows the backend, the frame-time percentiles, the resident
   pages/bytes and the total page uploads, and a memory line for the scene,
   the retained checkpoints, the GPU pages and the stock tiles. Pan the camera
   over a settled scene: the frame time stays inside the 33 ms target and the
   upload counters do not move.
   Leave the window alone for half a minute at a fixed playhead: the upload
   counters still do not move (the automated run observes exactly that for 30
   seconds, and records `cam-gui 0.7.7` on protocol `cam-gui-retained-5` next
   to the numbers).
   The memory line adds the display's categories (scene payload, retained
   checkpoints, the worker's live field and checkpoint copies, its decoded
   motion stream, the GPU pages and the stock tiles) to a declared total against
   the plan's 256 MiB browser budget — 115.8 MiB for the batch job at Fine with
   the playhead at the start — and says that WASM linear memory, the JS heap and
   total GPU memory are an unknown rather than a number it cannot see.
2. **Edit while the worker is busy.** Press **Generate** and, before it
   finishes, change the camera (**Isometric**) and then click **Cutting** in
   the navigator and type a value into **Roughing feed**. Both respond. When the
   generation completes,
   the status reports that the result was discarded for an older edit instead
   of replacing what was just typed — the edited value is still in the field —
   and **Generate** again plans the edited job.
3. **Take an action while the worker is busy.** During a generation, disable an
   operation or press Generate again: the status says the action is queued and
   will run when the current task finishes, and a second action replaces the
   first. It runs by itself once the worker is free.
4. **Inject a renderer failure.** **Inspect result → Renderer diagnostics →
   Inject renderer failure**. The viewport stops drawing and says the document
   and the prior result are retained; the playback bar, the inspector and the
   document keep working, and the playhead and resolution readouts do not move.
5. **Recover and re-open the prior view.** Press **Rebuild renderer
   resources**. The pipelines and buffers are recreated, the resident pages and
   stock tiles are re-copied from retained data, and the same document,
   playhead, display resolution, simulation key and camera come back. The
   diagnostics report the recovery count and the returned resident pages.
6. Close and re-open the viewport window or restart the review build and
   **File → Open job…** the batch fixture: the saved job opens normally and the
   display rebuilds from scratch.
7. **Cancel a running task.** Press **Generate**, then **Cancel** next to the
   spinner before it finishes. The status reports the cancellation and the
   worker stop time, the document and the last displayed result are still
   there (the display is marked stale because the retained execution went with
   the worker), and **Generate** in the same session produces the job again.

## Regression tour (short)

1. **File → Flower fixture**, apply the example machine, **Generate**,
   **Simulate**, **After endmill** / **After v-bit**, then **Export…** — the
   established carving is unchanged.
2. Drop `fixtures/gui4/lettering.job.json`, **+ Add operation → Add Flat
   V-carve** or **Add Profile**, reorder with **Move earlier**, and check that
   each operation's stock context and the prefix export still behave as in
   [the GUI7 recipe](gui7-review.md) and [the GUI8 recipe](gui8-review.md).
3. Apply a profile with tabs, finishing and a ramp entry and confirm the
   timeline labels, the anchor drag and the located refusals are as GUI8
   recorded.
4. Type a partial number such as `-` in any field; the raw text survives a
   restart through recovery, and Generate stays blocked until it is completed.

## Known limits (visible in this review)

- **250,000 motions is the display bound.** A plan above it is refused with
  both numbers named; nothing is truncated and no machining limit changes. The
  plan's million-motion L workload is not admitted yet.
- **The whole execution still travels as one payload.** 31.9 MiB for the batch
  fixture. Paging the execution itself is not implemented.
- **Each seek still transfers the whole checkpoint grid** (512 KiB for the
  batch, 1 MiB for the reference). The renderer copies only changed tiles to
  the GPU, but the wire format has no sparse form yet. The playback bar states
  the real number instead of implying the GPU-side saving is the whole story.
- **The navigator's operation list is not windowed.** Six operations fit
  comfortably; a job with hundreds of operations is GUI11 work (the whole-product
  usability pass), not something this milestone claimed.
- **The display raster is capped at 1024 cells across** (the Fine preset), so
  it can still be coarser than what the plan asked for; the panel states the
  multiple and the section shows what the cells resolved.
- **A section reads the raster.** It samples cell centres and states the cell
  size and depth quantum; it is a display cross-section, not a measurement of
  the cut surface.
- **The action queue holds one entry.** A second action taken while the worker
  is busy replaces the first; the status line says which one is waiting.
- **Cancel ends the retained execution.** The worker holding the plan stops, so
  the display is marked stale and the next Generate builds a new execution;
  the draft and any checked bundle already read back survive.
- **The renderer drill is a diagnostic.** It stops the viewport's custom GPU
  callbacks and rebuilds their resources; it does not destroy the wgpu device
  or simulate a driver reset.
- **Checkpoints are a budget, not a policy.** A job whose stage boundaries
  exceed the display budget drops the oldest boundaries and says how many.
- **No physical claim.** This is software and simulation review.

## Feedback record (to fill in during review)

The plan asks for the reviewer's own observations, separate from the automated
checks above. Short notes are enough; add the next action for anything that
needs one.

| Area | Observation | Bug or enhancement | Next action |
| --- | --- | --- | --- |
| Layout and density (playback bar, inspector, diagnostics) | | | |
| Terminology (Display resolution, sections, retained/ladder wording, status text) | | | |
| Navigation (timeline window, earlier/later stages, stage selector) | | | |
| Input feel (slider scrub, dragging, typing during work, cancel) | | | |
| Camera and scrub behavior (pan during load, backward/forward seeks, stalls) | | | |
| Errors and refusals (limits, cancelled work, failed rebuild, located reasons) | | | |
| Missing conveniences | | | |
