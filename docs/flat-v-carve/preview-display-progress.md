# Preview display — progress

Two user-facing preview defects found while unlocking the camera: the artwork
rescaled against the toolpath frame (fixed in `ca1799c`), and the simulated
stock drawing its side walls at the original stock height (this document).

## Free preview camera

Requested 2026-09-13: "unlock the rotation and zoom of the preview. Right now
'isometric' just allows rotations from the fixed viewpoint, we should have free
controls — pan/zoom/rotate. We should also support these from mouse — zoom with
mouse scroll, pan/rotate as in normal CADs."

The viewport had exactly two projections (top, and the isometric mix
`0.65*v + 0.76*z`), an azimuth that only moved with a primary drag, a zoom
locked to a 0.5–3 slider, and no pan at all. The elevation could not change and
nothing but the slider could zoom.

## Camera contract

`crates/cam-gui/src/camera.rs` is now the single orthographic orbit used by the
renderer, the picker and every overlay:

```text
u        = x*cos(yaw) - y*sin(yaw)
v        = x*sin(yaw) + y*cos(yaw)
screen_y = v*cos(tilt) + z*sin(tilt)
depth    = z*cos(tilt) - v*sin(tilt)
ndc      = ((u + pan.x)*zoom/aspect, (screen_y + pan.y)*zoom)
```

| Control | Range / meaning |
|---|---|
| `tilt` | Elevation. `0` is the historical top view, `ISO_TILT = atan2(0.76, 0.65)` reproduces the historical isometric mix, `±85°` is the limit (edge-on views have no ground-plane pointer solution) |
| `yaw` | Free azimuth, unwrapped |
| `zoom` | `0.05 … 64`, log-scaled on the toolbar slider; one is the framing the scene normalization was chosen for |
| `pan` | View-target offset in normalized scene units, applied before the zoom, so a panned point stays where it is when the zoom changes |

`Camera::uniform` packs the eight floats `scene.wgsl` and `stock.wgsl` read
(both camera buffers grew 16 → 32 bytes); `pick.rs` and `artwork_view.rs`
invert the same projection for cursor picking, artwork gestures and the
inspection point.

## Interaction

| Input | Effect |
|---|---|
| Primary drag | Orbit: horizontal azimuth, vertical elevation |
| Middle drag, or Shift + primary drag | Pan; the scene follows the pointer |
| Wheel, or pinch / ctrl-wheel | Zoom toward the pointer, so the point under the cursor stays put |
| Secondary drag | Orbit (an alternative for mice without a middle button) |
| Top / Isometric / Front | Set the elevation, keeping the azimuth |
| View slider | Numeric elevation, −85° … 85° |
| Zoom slider | Zoom, logarithmic |
| Fit | Default azimuth, zoom one and no pan; the chosen elevation is kept |

Interaction priority follows UI plan section 9.3: edit handles and selectable
geometry own their drags first. The artwork placement gesture keeps the primary
button, and the profile anchor drag keeps it too; the middle button pans in
every mode, and the wheel always zooms. The viewport carries the same map in its
`?` help.

## Compatibility

Saved views keep `isometric`, `zoom` and `yaw` and add `tiltDeg` and `pan`
(serde defaults, so older recovery files still load and restore the elevation
their flag recorded). The browser review scenarios that replicate the
projection in JavaScript (`gui3`, `gui4`, `knife-authoring`) stay correct
because they use the default top/isometric framing with no pan, and
`gui9`/`authoring` keep asserting `workspace.view.isometric`, which is now
derived from the elevation (`|tilt − ISO_TILT| < 1e-3`).

## Evidence

* `cargo test -p cam-gui` — 97 unit tests plus every integration suite.
  `camera.rs` pins the historical isometric mix (the historical pair is not
  exactly orthonormal, so the mix is reproduced to 4e-5), the top view, orbit
  deltas and limits, pan-follows-pointer, zoom-keeps-the-cursor-point at three
  elevations, and the ground-point inverse for twelve camera/rect/cursor
  combinations.
* `viewport::tests::the_pointer_orbits_pans_and_zooms_the_real_viewport` and
  `the_wheel_zooms_toward_the_pointer_and_the_presets_set_the_elevation` drive
  the real widget with synthetic pointer events: primary drag orbits and
  saturates at the limit, middle drag pans without turning, an artwork placement
  gesture keeps the primary drag while the middle button still pans,
  Shift + primary pans instead of orbiting, the wheel zooms about the cursor
  without moving the scene point under it, and the Top/Isometric presets set the
  elevation and keep the azimuth.
* `cargo clippy --workspace --all-targets` and `cargo fmt --check` are clean;
  `artifacts/gui/native/cam-gui.exe` is rebuilt from this change.

## Not covered

Animated transitions, keyboard-only orbit/zoom (the two sliders are the
keyboard-accessible path today), and a graphical view cube. Perspective
projection stays out of scope: the CAM preview is orthographic by design, and
`depth` remains a display ordering term rather than a physical camera.

## Stock walls follow the material

Reported 2026-09-13: "Why does the stock always have side walls at full height?
Generally, it feels like the whole visualization is broken because of that."

The stock pass drew one 30-vertex box — four sides from `z = 0` (the original
stock top) down to the stock bottom, plus the bottom face — next to a
per-cell surface that sits at the *machined* height. Facing the whole top by
4 mm therefore left a 4 mm brown rim standing around a recessed teal floor: the
part read as a tray holding the original stock envelope instead of a faced
plate. This contradicted the plan's own rule for the display (section 15.2,
"preserve a visible stock side/bottom where material exists").

`stock.wgsl` now walks the field's perimeter and emits one wall quad per
boundary cell — the low-Y, low-X, high-Y and high-X edges, with corner cells
carrying both of their faces — and each quad runs from **that cell's** machined
surface down to the stock bottom (`z = −removed · thickness` to `−thickness`).
An untouched boundary cell still shows the full stock thickness, a faced plate
shows its remaining thickness, a partially faced edge steps down cell by cell,
and a cell cut through leaves a real gap in the side rather than a wall
standing over air. The bottom face is unchanged. `stock_render::draw_vertices`
owns the matching vertex count (`cells·6 + perimeter·6 + 6`; the walls add 1.2%
of the surface's vertices on a 500 × 250 field), and the draw call uses it.

Evidence: `stock_render::tests::the_four_walls_cover_every_boundary_cell_once`
mirrors the shader's perimeter walk and asserts the four edges tile every
boundary cell exactly once, with corner cells on both of their edges;
`every_wall_quad_stands_on_its_own_boundary_and_follows_its_cell` additionally
checks the *placed* corners: each quad lies on the edge its walk step names,
stays inside the stock rectangle, and spans its own cell's remaining material
(the walk alone passed while two walls stood in the wrong place — see below);
`the_drawn_vertex_count_matches_the_shader_ranges` pins the vertex count against
the three ranges in `stock.wgsl`, so a truncated or over-long draw cannot pass
silently; `cargo test -p cam-gui --test shaders` parses and validates the shader
offline with naga. The geometry check is a mirror of the shader's walk, not a
GPU pixel test — the render itself still needs the tester's eye.

**Corrected after review.** The first build placed two of the four walls wrong:
the high-Y wall stood a full *width* past the stock (a long brown quad floating
beside the block) and the high-X wall stood at `x = height`, inside the stock.
Both came from one line: the edge coordinate used `along_extent` — the extent of
the axis the wall runs along — instead of the extent of the edge it stands on.
Only the two walls with `high = 1` were affected, which is why the two low edges
looked right. `stock.wgsl` now derives `edge_extent` separately, and the
positional test above fails with the old line (`wall (0, 2) along_x true edge 1
leaves the stock at (0, 200)` on a 200 × 100 stock), so the mistake cannot return
unnoticed.

Not covered: vertical walls **inside** a cut (a pocket or a partial face shows
its floor and its step edge only through the cell quads, not as a lit wall), and
a ghost outline of the original envelope, which is the cheap way to bring back
the "how much was removed" cue if it is missed.

## Stock display slices S1–S3 (plan: `stock-display-plan.md`)

The first three slices of the stock display plan have landed; S4 (a true
elevation/section view) has not started.

* **S1 (`96dfee9`) — identity, shading and style.** A cell now carries
  `depth | stage | tool` in the same 32 bits it always used, with the planner's
  own 256-stage bound covering the field, and the motion stream carries the
  stage the worker annotated it with. `stock_style.rs` holds the display-only
  appearance: plain, by operation, by tool and by depth modes over a ramp that
  spans the stock thickness, per-id colour overrides and automatic colours keyed
  by id, the wall threshold, the appearance and the artwork/path toggles.
  `stock.wgsl` shades with a per-fragment normal — the field gradient for floors,
  the wall plane for walls — and one fixed key light with an ambient term, so a
  step reads as an edge and a V-bit flank reads as a slope. Artwork and path
  toggles are draw-range decisions and shipped with it.
* **S2 (`b6af1be`) — interior walls.** `stock_walls.rs` derives the walls of the
  displayed field in the display process: one quad per step and per stock edge,
  runs merged, slopes below the threshold left as shaded surface, and a budget
  policy that raises the threshold, keeps the deepest steps and reports what it
  left out. The perimeter walk that lived in the shader is gone; the shader
  draws the instances, which is also what closes the one-cell slits that were
  visible at every cut edge.
* **S3 (`0b2915e`) — x-ray.** A second pipeline with no depth writes and alpha
  blending, drawn after the artwork and paths, so a path inside a cut stays
  readable through the material while material in front still tints it.

What the three slices do **not** yet do: a true elevation or section view (S4,
including the vertical-plane pointer mapping that would let the camera reach
90°), OIT for overlapping translucent surfaces, and per-tile incremental wall
rebuilds (the state-level rebuild is bounded and measured first).
