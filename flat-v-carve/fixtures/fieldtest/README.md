# Field-test fixtures (Inkscape SVG import)

Fixtures for workstream W4 of
[field-testing-fixes-plan.md](../../../docs/flat-v-carve/field-testing-fixes-plan.md),
taken from the "CAM Module — Field Testing Report" of 2026-09-13 (finding 3.1).

| File | What it pins |
| --- | --- |
| `inkscape-circle-stroke.svg` | The report's Inkscape 1.4 circle with `style="fill:none;stroke:#000000;stroke-width:1"`. Fill mode still refuses it, but the refusal names `circle1` / "Test circle" and states both remedies. |
| `inkscape-circle-stroke-to-path.svg` | The same circle after Stroke to Path, saved through a CSS `<style>` block with `class="cls-1"`. It must import with no XML editing, and the `fill-rule` in the rule must reach the geometry (a ring, not a disc). |
| `inkscape-layer-enable-background.svg` | A layer carrying editor properties (`enable-background`, `-inkscape-font-specification`) that the importer ignores, and selector forms beyond a single class: `.plate.inset`, `#plate-1`, `*`. Every ignored property is listed as a warning. |
| `inkscape-named-colour.svg` | A `!important` id-selector rule beating an inline declaration and a presentation attribute, plus a named colour and `transparent`. |
| `inkscape-external-stylesheet.svg` | `@import` from another file, which stays a hard `SVG_STYLESHEET` failure. |

Each fixture is hand-written to the shape Inkscape 1.4 emits (A4 page in
millimetres, `sodipodi:namedview`, `inkscape:groupmode="layer"`,
`inkscape:label`), so the importer's behaviour can be re-checked without the
original tester's files.
