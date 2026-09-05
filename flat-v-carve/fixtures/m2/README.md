# M2 Inkscape fixtures

These files were generated locally with **Inkscape 1.4.4 (dcaf3e7, 2026-05-05)** on Windows. Inkscape is needed to regenerate the exports, but is not a runtime or test dependency of the CAM application.

- `inkscape-source.svg` is the authored 100 × 60 mm coupon with a compound O, mixed path commands, a rounded rectangle, circle, overlapping rectangles, live Arial text B, and hidden text.
- `inkscape-export.svg` is Inkscape's plain SVG export after converting text to paths.
- `inkscape-native.svg` is the corresponding native export with editor metadata and layer labels.
- `inkscape-bounds.csv` contains Inkscape's `--query-all` output in CSS pixels: `id,x,y,width,height`. It includes hidden objects, which the importer intentionally excludes.

From this directory, with Inkscape on PATH:

```sh
inkscape inkscape-source.svg --export-plain-svg \
  --export-filename=inkscape-export.svg \
  --actions='select-by-element:text;object-to-path'
inkscape inkscape-source.svg --export-filename=inkscape-native.svg \
  --actions='select-by-element:text;object-to-path'
inkscape inkscape-export.svg --query-all > inkscape-bounds.csv
```

The test suite reads the committed exports. It checks the O's 300 mm² area and hole, the B's two holes, merged overlap provenance, matching native/plain selections, and all seven source bounds against Inkscape's measurements within 0.001 mm. Conversion to millimeters uses `25.4/96`; the SVG Y axis is then reversed about the 60 mm page height. Query output is decimal-rounded, so it is supporting evidence alongside analytic references, not an exact curve-error proof.

The unconverted source is an expected `SVG_TEXT` rejection. Core tests also generate compact SVG strings for units, transforms, fill rules, degeneracies, unsupported features, and precision limits.
