# GUI8 manual review recipe

This review covers the closed milling Profile and its tabs, radial finishing
and start/entry controls. Findings here are software observations; no machine
is controlled and no surface-quality claim is made.

## Launch

Native (recommended first):

```powershell
flat-v-carve\artifacts\gui8\review\profile-native\cam-gui.exe
```

Browser (offline package):

```powershell
cd flat-v-carve
node artifacts/gui8/review/serve-package.mjs
# open http://127.0.0.1:5182/web/index.html in desktop Chromium
```

`artifacts/gui8/review/manifest.json` records which source files and packages
these executables were built from (native SHA-256, WASM SHA-256 and the offline
bundle version).

## Fixtures

| Fixture | Use |
| --- | --- |
| `flat-v-carve/fixtures/gui3/lettering.svg` | The profile source: an L and a holed O. Three closed contours, two outers and one hole. |
| `flat-v-carve/fixtures/gui4/lettering.job.json` | The established Flat V-carve job with its applied machine configuration; add a Profile after it for the ordered case. |
| `flat-v-carve/fixtures/gui2/machine.json` | Applied through **Machine → Example machine → Apply flower machine profile** so the endmill has a controller mapping. |
| `real_data/flower_box-svg.job-real.json` | Regression tour (the known carving). |

## Fixture A — a profile alone, from an SVG (GUI8a)

1. **File → New profile job from SVG**, choose `fixtures/gui3/lettering.svg`.
   One Profile operation is created with no contours, no feeds and no geometry:
   every machining value shows `Unset`.
2. **Setup**: stock thickness `6`.
3. **Cutting**:
   - **Select all profile contours** → three rows appear with the importer's
     advisory side (`outside` for the two outer boundaries, `inside` for the
     O's hole). Change one side explicitly to see the row's own choice.
   - Stepdown `1`, Tool stepdown limit `1`
   - Roughing feed `300`, Plunge feed `100`, Spindle speed `12000`
   - Endmill diameter `3`, Cutting length `8`
   - **Cut Climb** and **Profile CW**
   - **Bottom: stock bottom** (a profile must state where the cut ends; without
     it the planner reports `HEIGHT_RANGE_INVALID`)
4. **Machine → Example machine → Apply flower machine profile**.
5. **Generate**: the result reports its motion count and the resolved heights.
6. **Simulate**: press **After profile rough**. The heightfield shows the cut
   band along each contour; **Start** restores the intact stock, and scrubbing
   backwards restores the earlier surface exactly.
7. **Export…**: the prepared program is read back and checked. **Save as…** and
   compare the displayed SHA-256 with the saved file.
8. **Save job**, then drop the saved file back into the window: the profile
   reopens with the same contours, sides, heights and feeds.

## Fixture B — tabs in that same workflow (GUI8b)

Continuing from Fixture A:

1. Open the **Tabs** group and press **Add tabs**. Height `0.5`, width `3`,
   count `1`.
2. **Generate**: the planner refuses the small hole with a located reason —
   *“requested 1 tabs on contour … hole-0 but only 0 fit its straight spans”* —
   and export stays unavailable. Nothing is silently dropped.
3. Uncheck `letter-o-0-hole-0` in **Geometry to cut** and **Generate** again:
   the two outer contours now cut with a bridge each. The viewport draws the
   requested anchor in yellow and the plan's generated bridge in green.
4. Switch **Tab placement** to **Manual anchors**, **Add tab on…** one outer
   contour at 25%, then:
   - type a new value in **Tab anchor fraction** (for example `0.4`), and
   - drag the yellow marker along the contour and release: the numeric row
     states the released position, and one **Undo** returns to the previous
     position.
5. **Simulate** and scrub: the bridge is visible after roughing and after the
   finishing pass. `Display simulation` reports the cell size; a tab narrower
   than one cell may not appear in the heightfield, while the drawn boundary is
   exact.
6. Export and reopen: the manual anchor, the placement mode and the tabs
   survive the portable round trip.

## Fixture C — radial finishing (GUI8c)

Continuing from Fixture B (tabs still configured):

1. Open **Radial finishing** and press **Add radial finishing**. Allowance
   `0.4`, Finish feed `150`.
2. **Generate**: the timeline now lists a rough and a finishing pass per
   contour, in execution order (`Profile rough paths (1 of 2)`,
   `Profile finish paths (1 of 2)`, …). The operation's feeds, spindle speed,
   stepdown limit and tool are unchanged: only the finishing feed is added.
3. Compare the passes: press **After profile rough (1 of 2)** and then the last
   **After profile finish** entry; **Inspect result** can pin one and compare
   with the playhead.
4. Change the allowance to `0` and regenerate: the finishing pass follows the
   same wall as the roughing with the finishing feed; the tabs stay in place.
5. Export and reopen: the finishing settings and the tabs are in the saved job.

## Fixture D — starts, entries and repair (GUI8d)

Continuing from Fixture C:

1. **Start & entry → Anchor the start**, choose the widest outer contour at
   `50%`. The cut now enters at that point instead of the automatic seam;
   dragging the yellow marker moves it and the numeric row states the same
   value.
2. Choose **Ramp entry** with angle `45` and feed `80`, then **Generate**:
   because the tool was never marked ramp capable, the result reports one
   setting that needs attention and routes it to **Ramp yes** in
   **Tool & cutting**.
3. Press **Ramp yes** and **Generate** again: the entry now descends while
   travelling along the contour instead of plunging straight down. Inspect the
   entry motions at the start of the pass.
4. Provoke an invalid entry: select the small hole as **On contour** and choose
   a **Tangent arc** lead-in. The planner refuses it with a located reason
   (a tangent arc needs a retained side). Press **Lead-in: No lead** to repair
   it, or return the hole's side to `Inside`.
5. A ramp or lead that cannot fit the small hole's loop is refused with the
   contour named; cut the outer contours instead and generate again.
6. Export, save and reopen: the anchored start, the ramp entry and every lead
   setting are in the portable job.

## Regression tour (short)

1. **File → Flower fixture**, apply the example machine, **Generate**,
   **Simulate**, **After endmill** / **After V-bit**, then **Export…** — the
   established carving is unchanged.
2. Drop `fixtures/gui4/lettering.job.json`, **+ Add operation → Add Flat
   V-carve** or **Add Profile**, reorder with **Move earlier**, and check that
   each operation's stock context and the prefix export still behave as in
   [the GUI7 recipe](gui7-review.md).
3. Type a partial number such as `-` in any profile field; the raw text
   survives a restart through recovery, and Generate stays blocked until it is
   completed.

## Known limits (visible in this review)

- **Rectangular tabs only.** Ramped shoulders are shown read-only and reported
  by the planner; nothing is silently reshaped.
- **One manual tab anchor per contour.** Use the automatic count for several
  tabs around one contour.
- **A small hole has no room.** For the `lettering.svg` fixture with a 3 mm
  cutter, the O's hole cannot host a 3 mm tab, a 45° ramp or a lead; each is
  refused with a located reason naming the contour.
- **One finishing pass per contour**, at the finished wall with its own feed.
- **The offset readout is nominal.** The yellow/green overlay distinguishes
  the requested anchors from the plan's generated bridges; neither measures a
  real edge.
- **No physical claim.** The tour is software and simulation review.
