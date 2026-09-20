# Field-test fixtures (Inkscape SVG import)

Regression inputs for the [supported SVG import contract](../../../docs/flat-v-carve/technical-design.md#4-svg-normalization), derived from the 2026-09-13 Inkscape field report.

| File | What it pins |
| --- | --- |
| `inkscape-circle-stroke.svg` | The report's Inkscape 1.4 circle with `style="fill:none;stroke:#000000;stroke-width:1"`. It publishes centerline geometry and an analytic drill marker, with a named stroke-width diagnostic; it does not invent a filled carving region. |
| `inkscape-circle-stroke-to-path.svg` | The same circle after Stroke to Path, saved through a CSS `<style>` block with `class="cls-1"`. It must import with no XML editing, and the `fill-rule` in the rule must reach the geometry (a ring, not a disc). |
| `inkscape-layer-enable-background.svg` | A layer carrying editor properties (`enable-background`, `-inkscape-font-specification`) that the importer ignores, and selector forms beyond a single class: `.plate.inset`, `#plate-1`, `*`. Every ignored property is listed as a warning. |
| `inkscape-named-colour.svg` | A `!important` id-selector rule beating an inline declaration and a presentation attribute, plus a named colour and `transparent`. |
| `inkscape-external-stylesheet.svg` | `@import` from another file, which stays a hard `SVG_STYLESHEET` failure. |

Each fixture is hand-written to the shape Inkscape 1.4 emits (A4 page in
millimetres, `sodipodi:namedview`, `inkscape:groupmode="layer"`,
`inkscape:label`), so the importer's behaviour can be re-checked without the
original tester's files.
