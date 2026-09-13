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
material that is left.

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
`max(0.25 × display cell, 0.05 mm)`, a style value). The wall quad stands on
the boundary between the higher and lower cell, runs from the **lower** cell's
surface up to the **higher** cell's surface, and carries the lower cell's
stage/tool.

**Merging.** Consecutive steps along a row or column with the same top, bottom
and identity merge into one quad, so a facing pass yields a handful of long
instances instead of one per cell.

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
| Plain | stock colour + shading | same, slightly darker |
| By operation | the stage that last deepened the cell | the stage that removed the material beside it |
| By tool | that stage's tool | same |
| By depth | the ramp evaluated at the cell's depth | the ramp evaluated at the **fragment's** z, so a wall reads as a gradient from its top to its bottom |

Palettes: one RGBA per stage (≤ 32) and per tool (≤ 256) in a single storage
buffer. Defaults come from a stable hash of the stage/tool **id**, with explicit
per-id overrides in the style; the buffer is rebuilt when the plan or an
override changes, never per frame. A mode that needs history ("show only what
operation 3 removed") uses the timeline's checkpoint replay; the colour mode
only ever colours the state that is displayed.

## 7. Display style, toggles and transparency

```rust
pub struct StockStyle {
    surface: ColorMode,        // Plain | ByStage | ByTool | ByDepth
    walls: WallMode,           // MatchSurface | OwnStage | ByDepth
    ramp: [Rgb; 3],            // depth gradient: two stops now, mid stops later
    stage_colors: BTreeMap<StageId, Rgb>,   // explicit overrides only
    tool_colors: BTreeMap<ToolId, Rgb>,
    wall_threshold_mm: f32,
    opacity: f32,              // 1.0 = opaque
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
* Opacity adds alpha to the style and a transparent stock pass after the opaque
  one (depth write off). Self-overlapping transparent geometry will still sort
  imperfectly until an OIT/depth-peeling slice lands; that limitation is
  reported in the UI rather than hidden.

## 8. Options considered

| Option | What it gives | Cost | Verdict |
|---|---|---|---|
| **Step instances from the field (S2)** | Exact 2.5D walls at display resolution, one record per face carrying identity and z range | Halo read at tile borders, a threshold rule, a bounded instance budget | **Chosen** |
| Per-cell procedural quads on the GPU (every cell emits 4 candidate quads, degenerate ones collapse) | No CPU work, no extra buffer | 4–5× vertex cost (750k → ~3 M per frame at Standard) and still needs indirection for per-wall attributes | Rejected |
| Solid mesh (marching cubes / dual contouring over a depth volume) | Closed solids, true silhouettes and section views, detachable pieces | Volumetric memory (cells × layers), rebuild machinery, cell identity must be carried explicitly | Later, only if sections or detachable pieces are needed (§10 S5) |
| B-rep CSG of swept tool volumes | Exact walls and true tool-shaped fillets | Not incremental, far outside a display feature | Rejected |

## 9. Slices

### S1 — Identity, shading and style plumbing

Pack stage/tool into the cell, add the palette buffer and `StockStyle` with
`Plain` as the default, add normals + light to `stock.wgsl`, add the artwork and
path toggles.

*Buys:* the three colour modes, toggles, and the shading that every later slice
needs. *Evidence:* packing round-trip tests; a colour-resolution test per mode
over a synthetic palette; a normal-computation test per synthetic field; the
naga shader validation; the probe reporting the active style. *Visual:* the
flower job in each mode, and Plain unchanged except for shading.

### S2 — Interior walls

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

### S3 — Opacity and an x-ray mode

Style alpha, the transparent stock pass after the opaque one, and a "see the
paths through the stock" preset.

*Buys:* inspecting a path inside a cut without hiding the stock.
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
* **Transparency.** Without OIT, overlapping transparent surfaces sort wrong;
  the plan states that rather than discovering it later.
* **Expectation of smooth walls.** A heightfield wall is quantised at the
  display cell; a user expecting a V-bit's smooth flank should see a shaded
  slope (threshold rule + shading), and only S5 can give a true smooth face.
* **Palette stability.** Auto colours must be derived from ids, or regenerating
  a plan would recolour the part.

## 12. Open questions for the tester

1. Default colour mode: plain (current look) or by operation?
2. Wall gradient range: over the wall's own height (each wall shows the full
   ramp) or over the stock thickness (walls are comparable across the part)?
3. Wall threshold: the `max(0.25 × cell, 0.05 mm)` default, and should it be a
   user control or a fixed display constant?
4. Is a "depth cut by this operation" mode wanted, or is absolute depth enough?
5. For the semi-transparent stock: a uniformly translucent envelope, or an
   x-ray that keeps the paths readable through the material?
