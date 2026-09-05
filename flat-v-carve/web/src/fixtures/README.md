# Captured Inkscape fixture

Captured on 2026-09-05 from the existing native `cam.exe` reporting engine `0.5.0` and milestone `M4`. The input is `flat-v-carve/fixtures/m2/inkscape-export.svg`, a synthetic geometry coupon. It has seven source regions, including a ring and the letter B with two preserved holes. All imported machining values are unset.

From the repository root, with an existing executable (do not rebuild into another task's target directory):

```powershell
flat-v-carve/target/release/cam.exe import flat-v-carve/fixtures/m2/inkscape-export.svg --output flat-v-carve/web/src/fixtures/inkscape.job.json
flat-v-carve/target/release/cam.exe inspect flat-v-carve/web/src/fixtures/inkscape.job.json --output flat-v-carve/web/src/fixtures/inkscape.preview.svg --report flat-v-carve/web/src/fixtures/inkscape.inspection.json
node flat-v-carve/web/scripts/project-display.mjs
```

- `inkscape.job.json` is the unmodified portable schema 3 import.
- `inkscape.inspection.json` is the unmodified full CLI inspection report, kept as the parity oracle.
- `inkscape.preview.svg` is the original inert debug output for reference; the app does not insert or load it.
- `inkscape.display.json` is a compact, deterministic projection of source IDs, labels, ring/hole flags, and coordinates. Integer source-grid coordinates are divided by the reported ticks/mm. No paths are simplified and no target/planning/stock geometry is generated.

`pnpm check:contracts` compares the complete projection with the report. The app ships only the compact display data and job snapshot. The report's legacy `planning_available` flag is deliberately not used as an API capability or verification claim.
