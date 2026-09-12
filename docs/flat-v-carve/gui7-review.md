# GUI7 manual review recipe

This review covers facing, ordered preparation and the supported mixed
Face → drag-knife sequence. Findings here are software observations; no machine
is controlled and no physical surface or blade-tracking claim is made.

## Launch

Native (recommended first):

```powershell
flat-v-carve\artifacts\gui7\review\facing-native\cam-gui.exe
```

Browser (offline package, any static server over the packaged directory):

```powershell
cd flat-v-carve\artifacts\gui7\review\browser
python -m http.server 5182
# open http://127.0.0.1:5182/web/index.html in desktop Chromium
```

`artifacts/gui7/review/manifest.json` records which source files and packages
these executables were built from.

## Fixture A — face a small stock by itself

1. **File → New face job**. The document has one Face operation, no artwork and
   no cutting values: every machining field shows `Unset`.
2. **Setup**: stock thickness `6`, stock minimum X `0`, minimum Y `0`, width
   `40`, length `30` (the four stock dimensions commit together).
3. **Cutting** (Face):
   - Stepdown `1`, Stepover `1.5`, Tool stepdown limit `1`
   - Cutting feed (shown as *Cutting feed*, stored as field 2) `300`, Plunge
     feed `100`, Spindle speed `12000`
   - Face pass angle `0`
   - Endmill diameter `3`, Cutting length `8`
   - Face bottom offset `-1` (this is the facing depth below the stock top)
   - **Face CW** for spindle direction (required before checked export)
4. **Machine → Example machine → Apply flower machine profile** so the endmill
   has a controller mapping.
5. **Generate**: the result reports the motion count, the basic checks and the
   published face plane. Expect a real face plane at `Z -1` covering the whole
   requested rectangle.
6. **Simulate**: press **After face**. The heightfield shows the removal across
   the stock; **Start** returns to intact stock, and scrubbing backwards
   restores the earlier surface exactly.
7. **Export…**: the prepared program is read back and checked. Choose
   **Save as…** and compare the displayed SHA-256 with the saved file.
8. **Save job**, then drop the saved file back into the window: the Face job
   reopens with the same values and generates the same result.

Things to look at while reviewing:

- Changing **Face bottom offset** changes the facing depth; changing stock
  thickness changes the measured depth reference.
- **Rectangle** coverage with per-side **Face margin** values and **Face entry
  / exit overrun** shows what is actually removed (the requested rectangle
  plus margins) and where the tool travels (the coverage plus overrun). The
  result inspection panel reports the covered rectangle and the tool travel
  separately.
- A pass angle other than 0° or 90°, or a stepover larger than the cutter
  diameter, refuses the job with a located reason; the result is never
  exportable.

## Fixture B — face before the familiar carving

Drop `flat-v-carve/fixtures/gui4/lettering.job.json` into the window (the
established Flat V-carve job with its machine configuration).

1. **Add operation → Add Face**. The new Face operation is selected and its
   inputs start unset; the carving is untouched.
2. Configure the face as in Fixture A but shallower: stepdown `0.5`, tool
   stepdown limit `0.5`, feeds `300` / `100`, spindle `12000`, endmill
   diameter `6`, cutting length `8`, bottom offset `-0.5`, **Face CW**.
3. **Move earlier** until the Face row is first. The Operations list is the
   sole execution order.
4. **Generate**: the sequence Face → Flat V-carve is planned and checked.
5. **Simulate**: **After face** shows the faced stock before the carving cuts
   anything; **After endmill** shows the carving on top of it. The carved floor
   is measured below the faced plane, and the earlier removal is never
   discarded by hiding paths or scrubbing backwards.
6. Select the carving row (**Cutting**), then **Top: face-1 face result** so the
   carving starts from the plane the face published. Generate again: the
   carving's cutting motions begin at the faced plane.
7. **Move later** on the Face row so the carving precedes its dependency.
   Generate: the carve reports a located unresolved height reference
   (the operation panel shows the issue and routes to the field that owns it).
   Move the Face row back and generate again to repair it.

Known limit: the lettering carving is endmill-only, so its timeline entry is
**After endmill**; a combined endmill/V-bit operation adds its own **After
V-bit** entry.

## Fixture C — supported Face → drag knife

Drop `flat-v-carve/fixtures/gui6/knife.job.json` (the GUI6 knife review source).

1. **Add operation → Add Face** and configure it exactly as in Fixture B
   (`-0.5` depth). Move it first.
2. **Generate**, **Simulate**: **After face** shows the faced stock; **After
   knife** shows the same stock with the knife traces drawn over it. Knife
   traces remove no material.
3. **Export…**: the prepared bundle contains the knife replay evidence for the
   emitted bytes of the whole prefix.
4. Set the Face **bottom offset** to `-2` (deeper than the blade's `-1` cut).
   Generate: the knife reports `KNIFE_CONTACT_UNSUPPORTED` with the location and
   the operation that removed the material. Restore `-0.5` to repair it.

Known limits for this fixture:

- The face cutter has no controller mapping in this job; preparation refuses
  the sequence with `MACHINE_MAPPING_MISSING` naming the tool until it is mapped
  in **Job tools**.
- Only the supported Face → drag-knife ordering is exercised. Contact over
  material removed by an arbitrary preceding carving or profile is not assumed
  supported, and the review does not assert physical blade tracking.

## Regression tour (short)

After the fixtures above, confirm the earlier workflows still behave:

1. **File → Flower fixture**, apply the example machine, **Generate**,
   **Simulate**, **After endmill** / **After V-bit**, then **Export…**.
2. Open the GUI6 knife job and repeat its own tour if the knife editor is part
   of what is under review.
3. Type a partial number such as `-` in any field, restart and restore the
   recovery draft; the raw text survives, and generation stays blocked until it
   is completed.
