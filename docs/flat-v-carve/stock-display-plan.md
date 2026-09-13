# Stock display model — plan

Source: review discussion of the free-camera preview, 2026-09-13, immediately
after the preview display batch (`817ecbc`). The tester asked for interior
walls "not cheaped out", colouring modes for the simulated stock (walls and
floor by the operation that created them, by depth — with a gradient on the
walls — and possibly by tool), on/off toggles for the artwork and the paths,
and eventually a semi-transparent stock, with the explicit requirement that the
approach must not block later work.

**Status: plan only.** Nothing below is implemented. What has landed so far is
recorded in [preview-display-progress.md](preview-display-progress.md): the one
display frame, the free camera, and the stock's boundary walls following the
material that is left. The five questions in §12 were answered by the tester on
2026-09-13; §3–§9 fold those answers in, and §12 keeps them with the
consequences and the smaller questions they raise.

---

## 1. What exists today

| Piece | What it holds | Where |
|---|---|---|
| Field cell | 4 bytes: removed depth (`u16`, quantum = thickness/65535) plus cutter **kind** (`u8`: 0 untouched, 1 endmill, 2 V-bit) | `sim.rs` — `Tile`, `Field::apply`, `packed_tile_bytes` |
| Tiling | 256 × 256 tiles, 256 KiB each, tile-major; per-tile version counters drive dirty-tile uploads | `sim.rs`, `stock_preview.rs`, `stock_render.rs` |
| Display raster | longest side 256/512/1024 cells (`DisplayPreset`), 18 retained checkpoints, 8/20/64 MiB budget | `stock_preview.rs` |
| Stock pass | one un-indexed draw: 6 vertices per cell (a flat quad at the cell depth), 6 per boundary cell (the walls from `817ecbc`), 6 for the bottom face | `stock.wgsl`, `stock_render.rs` |
| Shading | none — flat colour plus a depth-darkening term | `stock.wgsl` |
| Paths and artwork | one scene pass drawing a contour range and a visible motion range; per-artwork hiding already exists | `render.rs` |
| "As of operation N" | retained checkpoints plus the playhead | `stock_preview.rs`, `viewport.rs` |
| Bounds that matter | ≤ 32 display stages (`MAX_DISPLAY_GROUPS`), ≤ 256 tools (plan §15.2), `MOTION_LIMIT = 250_000` | `session.rs`, `2.5d-cam-plan.md` |

The field therefore knows *how deep* and *with what kind of cutter*, and
nothing else. **A heightfield has no interior geometry at all**: it is a 2.5D
surface, so every vertical face — the stock's own edge included — has to be
derived. There is also no per-cell operation identity, no normal or light, and
no display style. That is the whole gap this plan closes.

---

## 2. Decisions

**D1 — The heightfield stays the authoritative display model.** The machining
truth remains the plan and the emitted program; the field, the walls and the
style are display artifacts, derived, version-keyed and never saved into the
job (plan §15.2 and UI plan §9.1).

**D2 — Interior walls are explicit *step instances* derived from the field**,
not per-cell procedural quads (§5, §9). The boundary ring that exists today is
the same detector's special case and folds into it rather than staying a
second code path.

**D3 — Identity travels in the cell, without growing it.** Pack
`depth 16 | stage 8 | tool 8` — still exactly 32 bits, so neither memory nor
bandwidth changes. `stage` is the executed stage index that last deepened the
cell (≤ 32 by `MAX_DISPLAY_GROUPS`), `tool` is the job tool that cut it
(≤ 256 by the plan's cap). Bump `DISPLAY_ALGORITHM_VERSION` so retained caches
from the old packing are never reused.

Ownership semantics, to implement exactly once and keep:

* a **floor** belongs to the stage that last deepened that cell;
* a **wall** belongs to the stage that removed the material beside it — the
  lower cell's identity, i.e. the cutter that actually created the face;
* "only what operation N removed" is the existing checkpoint replay plus the
  timeline filter, never a per-cell history.

**D4 — Shading lands before walls.** A wall painted in the floor's colour is
invisible, and today's pass has no lighting at all. A per-fragment normal
(from the field gradient for surfaces, from the wall's own plane for walls)
plus one directional light is what makes walls *and* V-bit slopes readable, and
it is the shared substrate every colour mode needs.

**D5 — A step counts as a wall only above a threshold** (§5), so a V-bit slope
does not become a staircase of 0.1 mm quads.

**D6 — The display style is display-only and keyed by ids.** Modes, palettes,
ramp, opacity and toggles live with the workspace view beside the camera, are
keyed by stage/tool **id** so they survive regeneration, and never enter the
job schema.

**D7 — Transparency is designed in now, implemented later.** The pass order
becomes opaque stock → transparent stock → paths → overlays; correct sorting of
self-overlapping transparent geometry (OIT/depth peeling) is its own slice, not
a blocker for the rest.

---

## 3. The field: identity without growth

```
bits  0..15  removed depth      (u16, quantum = thickness/65535)
bits 16..23  stage index        (u8, 0 = untouched, ≤ 32 by MAX_DISPLAY_GROUPS)
bits 24..31  tool index         (u8, 0 = none, ≤ 256 by the plan's tool cap)
```

* `Field::apply` already only ever raises a cell's depth (`if level > previous`
  in `sim.rs`), so "the stage that last deepened the cell" is well defined and
  monotone: a later pass that removes more material owns the new floor.
* The cutter *kind* disappears from the cell; it is derived from the stage's
  tool in the plan, so no information is lost.
* `stage_removed_mm3` stays a legacy two-slot stat; per-stage volume already
  belongs to the plan's own accounting, not the display.
* A packing round-trip test (depth/stage/tool in, same out) plus a
  `DISPLAY_ALGORITHM_VERSION` bump is the whole migration.

## 4. Geometry: how interior walls are built

**Detection.** For each pair of adjacent cells along X and Y, a step exists
when the depth difference exceeds `wall_threshold` (default
`max(0.25 × display cell, 0.05 mm)`, a style value that ships as the default and
is not exposed as a control yet). The wall quad stands on the boundary between
the higher and lower cell, runs from the **lower** cell's surface up to the
**higher** cell's surface, and carries the lower cell's stage/tool.

**Merging.** Consecutive steps along a row or column with the same top, bottom
and identity merge into one quad, so a facing pass yields a handful of long
instances instead of one per cell.

**Status:** landed for S1–S3 (`96dfee9`, `b6af1be`, `0b2915e`); S4 is not
started. Two implementation notes differ from the text below and are recorded
here rather than left implicit: the boundary ring *is* folded into the detector
as planned, and the wall set is rebuilt for a changed displayed state and
otherwise cached (keyed on the stock identity, the playhead prefix and the
threshold) instead of being rebuilt per dirty tile. The state-level rebuild is
O(cells) with a small constant and happens only when the displayed state
changes, so the per-tile path is left as a measured follow-up rather than
guessed at; the interface in the shader is the same either way.

**Where they are built — in the display process, not the worker.** The display
side already holds the complete packed field for the displayed state: either a
transported checkpoint section in the payload, or the locally re-integrated
field it builds for seeks (`StockView::range` / `StockView::local`,
`viewport.rs`). Building walls there means:

* no payload, worker or job change — the walls never travel;
* the same per-tile version counters that drive dirty cell uploads drive
  incremental wall rebuilds, batched per frame like `TILE_UPLOADS_PER_FRAME`;
* a 1-cell halo is available, so a wall on a tile border is built with its
  neighbour's real height rather than a guess;
* a checkpoint swap rebuilds the whole wall set once, bounded by the same
  budget policy as the cells.

**Record and budget.** One instance is 16 bytes:
`cell u32 | direction u8 | stage u8 | tool u8 | pad | top f32 | bottom f32`.
Worst case is about two instances per cell: Standard (512 × 256) ≈ 262k
instances ≈ 4.2 MiB, Fine (1024 × 512) ≈ 17 MiB. Policy, in order: merge runs;
raise the threshold; fall back to the coarser display preset; and **report**
the reduced wall set the way display resolution is reported today. Silent
truncation is not allowed.

**Not walls.** Nothing is emitted for a knife stage (it removes no material)
and nothing for flat neighbourhoods.

## 5. Shading

* Surface normal per fragment from the field gradient across the cell's four
  neighbours (the same halo the wall builder reads).
* Wall normal from the wall's plane (axis-aligned, so it is exact).
* One directional light plus a small ambient term; optional depth-based
  darkening for deep pockets so a 6 mm pocket reads differently from a 1 mm
  one.
* Shading multiplies the *mode's* colour, so every colour mode and the plain
  look share one path.

## 6. Colouring modes

| Mode | Floor | Walls |
|---|---|---|
| **Plain (default)** | stock colour + shading | same, slightly darker |
| By operation | the stage that last deepened the cell | the stage that removed the material beside it |
| By tool | that stage's tool | same |
| By depth | the ramp evaluated at the cell's absolute depth | the ramp evaluated at the **fragment's** z, so a wall reads as a gradient |

The depth ramp spans the **stock thickness**, not each surface's own extent:
both floors and walls sample one ramp parameterised by absolute z from the
original stock top (`0 mm`) to the stock bottom (`thickness`). A 2 mm wall
therefore shows only the slice of the ramp between its two levels, which is what
keeps depth comparable across the part — a wall and the floor beside it read as
the same depth when they are the same depth.

The three identity/depth modes colour the state that is displayed; there is no
mode relative to a single operation's own depth ("depth cut by this operation")
for now. It remains a possible later variant of the same palette lookup, not a
separate code path.

Palettes: one RGBA per stage (≤ 32) and per tool (≤ 256) in a single storage
buffer. Defaults come from a stable hash of the stage/tool **id**, with explicit
per-id overrides in the style; the buffer is rebuilt when the plan or an
override changes, never per frame. A mode that needs history ("show only what
operation 3 removed") uses the timeline's checkpoint replay.

## 7. Display style, toggles and transparency

```rust
pub struct StockStyle {
    surface: ColorMode,        // Plain (default) | ByStage | ByTool | ByDepth
    walls: WallMode,           // MatchSurface | OwnStage | ByDepth
    ramp: [Rgb; 3],            // gradient over the stock thickness; 2 stops now
    stage_colors: BTreeMap<StageId, Rgb>,   // explicit overrides only
    tool_colors: BTreeMap<ToolId, Rgb>,
    wall_threshold_mm: f32,    // default max(0.25 * cell, 0.05); no control yet
    appearance: Appearance,    // Opaque (default) | XRay { opacity: f32 }
    show_stock: bool,
    show_artwork: bool,
    show_paths: bool,
}
```

* Lives in the workspace view beside the camera; serialized with the view
  (serde defaults for older saved views), never in the job.
* GPU binds it to the **stock pass only** (style uniform, palette storage
  buffer, wall storage buffer); the scene pass keeps its own bindings.
* Artwork and path toggles are draw-range decisions in the existing scene
  callback — an empty contour range and an empty visible motion range — so they
  need no shader work and can ship ahead of the walls.
* **X-ray** is the requested translucency: the stock is drawn so that artwork
  and paths read **across** it. That fixes the pass order — artwork and paths
  first (opaque, depth-writing), the stock last — with the stock writing depth
  in `Opaque` and **not** writing it in `XRay`. Material behind a path therefore
  no longer hides it, while material in front of it still tints it by the x-ray
  opacity, which is what makes an internal path readable inside a cut. Drawing
  the stock last is also correct in `Opaque` mode: geometry above the surface
  keeps the nearer depth it already wrote.
* The walls are part of the same translucent pass. Self-overlapping translucent
  geometry still sorts imperfectly until an OIT/depth-peeling slice lands (S3
  follow-up); the limitation is reported in the UI rather than hidden.

## 8. Options considered

| Option | What it gives | Cost | Verdict |
|---|---|---|---|
| **Step instances from the field (S2)** | Exact 2.5D walls at display resolution, one record per face carrying identity and z range | Halo read at tile borders, a threshold rule, a bounded instance budget | **Chosen** |
| Per-cell procedural quads on the GPU (every cell emits 4 candidate quads, degenerate ones collapse) | No CPU work, no extra buffer | 4–5× vertex cost (750k → ~3 M per frame at Standard) and still needs indirection for per-wall attributes | Rejected |
| Solid mesh (marching cubes / dual contouring over a depth volume) | Closed solids, true silhouettes and section views, detachable pieces | Volumetric memory (cells × layers), rebuild machinery, cell identity must be carried explicitly | Later, only if sections or detachable pieces are needed (§10 S5) |
| B-rep CSG of swept tool volumes | Exact walls and true tool-shaped fillets | Not incremental, far outside a display feature | Rejected |

## 9. Slices

### S1 — Identity, shading and style plumbing

**Status: landed (`96dfee9`).** Cells pack `depth | stage | tool`; the worker
publishes the stage identity table; `stock_style.rs` holds the modes, palettes,
ramp, threshold, appearance and toggles; the stock pass shades with a normal
from the field gradient and a fixed key light; artwork and path toggles are draw
ranges. Evidence: `sim::tests::packed_cells_carry_the_stage_and_tool_that_created_the_surface`,
the `stock_style` suite, `camera::tests::the_screen_basis_matches_the_projection`,
`viewport::tests::the_stock_style_drives_the_uniform_and_the_palette`, and naga
validation of the shader.

Pack stage/tool into the cell, add the palette buffer and `StockStyle` with
`Plain` as the default, add normals + light to `stock.wgsl`, add the artwork and
path toggles. The depth ramp is parameterised over the stock thickness from the
start, even while the default mode does not read it.

*Buys:* the three colour modes, toggles, and the shading that every later slice
needs. *Evidence:* packing round-trip tests; a colour-resolution test per mode
over a synthetic palette; a normal-computation test per synthetic field; the
naga shader validation; the probe reporting the active style. *Visual:* the
flower job in each mode, and Plain unchanged except for shading.

### S2 — Interior walls

**Status: landed (`b6af1be`).** `stock_walls.rs` derives one quad per step and
per stock edge from the packed field, merging runs, thresholding slopes and
bounding itself with a reported budget; the shader draws the instances instead
of walking the perimeter. Evidence: the `stock_walls` suite (faced plate,
pocket, through cut, slope below threshold, budget policy),
`stock_render::tests::the_drawn_vertex_count_matches_the_shader_ranges` and the
positional wall tests carried over from the boundary-ring fix.

Step detector with the threshold, run-length merging, incremental rebuild on the
dirty-tile set with a halo, the wall storage buffer and draw range, and the
boundary ring folded into the same detector.

*Buys:* pocket and partial-face walls, and the geometric closure of the slits
that are visible today at every cut edge. *Evidence:* step extraction on
synthetic fields — a facing pass (few long runs), a pocket (four walls), a
through cut (a gap, no wall), a V-bit slope (no walls above the threshold); the
budget policy test (pathological staircase merged/coarsened *and reported*);
the vertex-count/range contract test; and, learning from the misplaced-wall
bug, a positional test that asserts each instance's placed corners lie on its
own edge inside the stock and span the right heights. *Visual:* a facing job, a
pocket, and the flower job.

### S3 — X-ray stock

**Status: landed (`0b2915e`).** A second pipeline (no depth writes, alpha
blending) plus the scene-first/stock-last pass order; `output_alpha` is pinned
by test and the probe reports appearance, opacity and the wall budget.

The `Appearance::XRay { opacity }` style value, the artwork/paths-first pass
order with the stock drawn last and no depth writes, and a viewport control for
the opacity.

*Buys:* reading artwork and paths across the material — inspecting a path inside
a cut, or checking how a perimeter relates to the part, without hiding the
stock or turning it off.
*Evidence:* pass-order and style round-trip tests, and a documented statement of
what still sorts incorrectly. *Follow-up:* OIT/depth peeling as its own slice if
the artifacts matter.

### S4 — True elevation and section views

The silhouette/section rendering that makes a 90° front view readable, together
with the vertical-plane pointer mapping that a true elevation needs (today the
camera stops at 85° exactly because the ground-plane pointer conversion has no
solution beyond it). *Evidence:* a section-extraction test over a synthetic
field and a front-elevation probe.

### S5 — Solid mesh (only if needed)

Dual contouring over a depth volume, if detachable pieces, closed solids or
arbitrary sections become requirements. The wall instance stream is the
compatible predecessor: both are "surface patches with identity and a z range",
so the style, palette and shading layers do not change.

## 10. Interfaces that must not regress

1. No colour, style or wall geometry in the job, the plan identity or the
   emitted program; everything here is derived and version-keyed.
2. Identity by id, never by array index — palettes and overrides are keyed by
   stage/tool id and mapped to indices only when a buffer is built.
3. One shading model for floors and walls.
4. The stock's own boundary is the step detector's special case, not a parallel
   implementation.
5. A bounded, *reported* wall budget; never a silent truncation.
6. The style is one object with modes rather than a growing set of booleans, so
   a new mode is a new variant plus a palette lookup, not a new code path.
7. The stock pass keeps working when the field has no walls at all (an
   untouched stock, a knife-only job, a coarse preset).

## 11. Risks

* **Wall count.** A pathological staircase can approach two instances per cell;
  the merge/threshold/preset policy plus reporting is what keeps it bounded,
  and it needs a measurement on the flower job and on a synthetic worst case.
* **Incremental rebuild complexity.** Halos, per-tile wall versions and
  checkpoint swaps are the subtle part; the mitigation is to key wall versions
  on the same counters the cells use and to test a seek that changes one tile.
* **Transparency.** The x-ray appearance is the one mode where draw order is
  visible: overlapping translucent surfaces (the walls in particular) sort wrong
  without OIT/depth peeling. S3 states the limitation instead of discovering it
  later, and the pass order it fixes is the one OIT needs anyway.
* **Expectation of smooth walls.** A heightfield wall is quantised at the
  display cell; a user expecting a V-bit's smooth flank should see a shaded
  slope (threshold rule + shading), and only S5 can give a true smooth face.
* **Palette stability.** Auto colours must be derived from ids, or regenerating
  a plan would recolour the part.

## 12. Answers, consequences and what they raise

The tester answered the plan's five questions on 2026-09-13:

1. **Default colour mode: plain.** `Plain` is the shipped default, so the stock
   keeps today's look (plus the new shading); the identity and depth modes are
   opt-in.
2. **Wall gradient: across the stock.** One ramp parameterised by absolute z
   from the stock top to the stock bottom, sampled by floors and walls alike, so
   depths stay comparable across the part (§6).
3. **Wall threshold: keep the default.** `max(0.25 × display cell, 0.05 mm)`
   stays a style value with no control for now (§4).
4. **Absolute depth is enough for now.** No "depth cut by this operation" mode;
   it stays a possible later variant of the same palette lookup (§6).
5. **The translucent stock should be an x-ray** that keeps artwork and paths
   readable across the material, which is what fixes the pass order in §7: the
   scene's artwork and paths first with depth writes, the stock last without
   them in `XRay`, and the x-ray opacity controls how much the material in
   front tints what is behind it.

Consequences already folded in: the default mode in §6 and the `StockStyle`
sketch in §7, the ramp's parameterisation, the threshold note in §4, and S3's
scope. Nothing here changes S1 or S2, which is the point of answering them
before the plumbing is built.

Smaller questions these answers raise, none blocking S1:

* Is x-ray a toggle beside plain, or a third appearance with its own preset
  ("see-through", "ghost", ...) that also sets which layers stay visible?
* In x-ray, should the walls keep a higher opacity than the floors so the part's
  shape still reads while the interior stays visible?
* In x-ray, should artwork and paths be drawn fully undistorted (never tinted)
  so a path is unambiguous, with only the *stock* carrying the transparency?
