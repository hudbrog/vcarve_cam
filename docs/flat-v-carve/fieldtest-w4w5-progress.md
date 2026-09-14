# Field-testing fixes: W4 and W5 progress

Date: 2026-09-14\
Workstreams: W4 (Inkscape/SVG import compatibility, report §3.1) and W5
(artwork and stock coordinates, report §3.2 + §3.3), from
[field-testing-fixes-plan.md](field-testing-fixes-plan.md).\
Engine 0.7.7.

Both workstreams are landed with tests, and the plan document now carries the
`Status:` paragraphs that record the decisions. Nothing here changes the
import mapping itself, so no saved job changes meaning and no project version
migration is needed.

## What changed

**W4 — `<style>` support and honest diagnostics.**

* `svg::style::Stylesheet` parses every `<style>` element once, then one
  cascade per element resolves presentation attributes, matching stylesheet
  rules and the inline `style` attribute in CSS order: `!important` first,
  then specificity (element / `.class` / `#id`, packed so compound selectors
  work), then source order. Inline declarations keep inline specificity.
* A CSS property outside the supported subset — Inkscape's own editor
  properties included — is a **warning** (`SVG_STYLE_IGNORED`) naming the
  element and the property, instead of being dropped in silence.
* A property that changes the drawn geometry stays a hard failure: `filter`,
  `mask`, `clip-path`, markers, `transform-origin`, and the CSS `transform`
  property (`SVG_RENDERING_FEATURE` / `SVG_STYLE_UNSUPPORTED`). The SVG
  `transform` *attribute* is untouched and still parsed as before.
* A selector this subset cannot match is reported once
  (`SVG_STYLE_SELECTOR`) rather than silently ignored.
* External stylesheets stay hard failures (`SVG_STYLESHEET`): both
  `<?xml-stylesheet?>` and `@import`, because the importer has no file or
  network access.
* Every element-scoped diagnostic now names the element's `id` **and** its
  `inkscape:label` (`element_scope`), and the fill-mode stroke refusal names
  both remedies: Stroke to Path, or import the artwork as centerlines. Step 4
  of the plan took its second branch — the refusal stays, and the one-click
  route is named in the message rather than a new import mode being invented.

**W5 — the coordinate contract, and stock resize that says what it did.**

* `svg::page_to_artwork` (`y_artwork = page_height − y_svg`) is now the one
  written expression of the flip, and `technical-design.md` §1 names the four
  XY spaces (SVG/document, artwork/model, stock/work, machine) with the single
  conversion each; §4 states it in user terms. The Artwork panel caption says
  the same thing where the user met the confusion.
* `commands::StockAnchor` gives a stock resize a named anchor: **Min corner**
  (default — exactly what the fields always did, so saved jobs keep their
  meaning), **Stock centre**, and **Artwork bounds**. Setup shows the three
  choices with help, and `commands::anchor_stock_rectangle` is a pure function
  over the requested rectangle, the previous rectangle and the placed artwork
  bounds.
* *Stock from artwork bounds* sits next to *Stock XY from SVG page*: every
  item, zero margins, through the existing fit-stock proposal.
* Geometry outside the stock is reported, never moved:
  `commands::artwork_outside_stock` / `stock_overlap_issues` name each item and
  the worst side it passes (`STOCK_ARTWORK_OUTSIDE`, path `setup.stock.xy`),
  and the Setup panel prints the same names beside the stock numbers. Tangent
  contact counts as inside: touching an edge is legal, as W1 already relies on.
* The anchor is workspace state, not document state. The rectangle it produces
  is what gets saved, so this needs no schema field and no migration.

## Evidence

Commands run in `flat-v-carve/` on 2026-09-14:

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | clean (exit 0) |
| `cargo clippy --workspace --all-targets` | clean, no warnings (exit 0) |
| `cargo test --workspace --no-fail-fast` | 79 binaries, 702 tests passed, 0 failed (exit 0); log in `artifacts/fieldtest-w4w5/workspace-tests.txt` |
| `cam import` over the six `fixtures/fieldtest/` files | recorded in `artifacts/fieldtest-w4w5/import-report.txt` |
| `pwsh ./scripts/build-gui.ps1` | `artifacts/gui/native/cam-gui.exe` rebuilt (release) |
| `node scripts/native-smoke.mjs --exe artifacts/gui/native/cam-gui.exe` | passed, 17 checks, 137717 motions — identical to the GUI9 baseline, so nothing in W4/W5 disturbed planning; report in `artifacts/fieldtest-w4w5/native-smoke.json` |

The CLI readback is the acceptance evidence for W4, verbatim in the artifact:
the report's Stroke-to-Path file imports with no XML editing; each ignored
property is listed as a warning naming the element; the stroke-only circle is
still refused, naming `circle1` / `Test circle` and both remedies; the external
stylesheet is still refused.

New and changed tests:

| Test | What it pins |
| --- | --- |
| `crates/cam-core/tests/svg_fieldtest.rs` | the report's circle (refusal and both remedies), the CSS ring import, `.plate.inset` specificity, `!important` vs inline vs presentation attribute, external stylesheet, unsupported selector, malformed inline declaration |
| `crates/cam-core/tests/svg_coordinates.rs` | the A4/mm golden bounds (page top-left → 277…297, page bottom-left → 0…20, the plan's 100 mm example → 80…100) and `page_to_artwork` |
| `crates/cam-core/tests/artwork_commands.rs` | anchors move the rectangle and never the artwork; tangent is inside; outside-stock items are named, reported and fixable by fitting |
| `crates/cam-gui/src/inspector.rs` | the Setup tab publishes the anchor choices, the stock-fit action and the outside-stock notice; a size edit keeps the chosen anchor |
| `crates/cam-core/tests/svg_jobs.rs` | the `<style>` case now expects an external stylesheet to fail, not `<style>` itself |

Fixtures added: `flat-v-carve/fixtures/fieldtest/` (six files plus a README
naming each one's purpose).

## One repair outside W4/W5, reported rather than silent

`cargo test` was already red on `main` when this batch started, and not because
of anything in W4/W5: commit `9059df4` replaced
`real_data/flower_box-svg.job-real.json` with the tester's schema-5 session
file, while five other places still used that path as their *legacy* job, so
`Job::from_json`/`LegacyJob::from_json` failed with `JOB_SCHEMA_VERSION`. The
legacy content is now stored verbatim as
`flat-v-carve/fixtures/m4/flower-combined-legacy.json` — the same blob the
previous revision of the real file held (`git hash-object` verified) — and the
legacy-input users read it:

| Consumer | Was | Now |
| --- | --- | --- |
| `crates/cam-core/tests/project_migration.rs` | failing | legacy fixture |
| `crates/cam-core/tests/sequence_plan.rs` | failing | legacy fixture |
| `crates/cam-gui/tests/workflow.rs` (two uses: the legacy planner comparison and "GUI2 is schema-5 only") | failing | legacy fixture |
| `crates/cam-gui/tests/collection.rs` | silently stopped exercising the migration path | legacy fixture |
| `crates/cam-gui/examples/reference_fixture.rs` | would fail if run | legacy fixture |
| `crates/cam-gui/web/gui4-scenario.mjs` ("Import older job") | would import a schema-5 file as if it were older | legacy fixture path |
| `experiments/gui1/src/compute.rs` | would fail on the first frame | legacy fixture |

The tester's `real_data/flower_box-svg.job-real.json` is untouched: its
consumers that *want* the schema-5 job (`planner_optimizations.rs`,
`vcarve/routing.rs`, `scripts/benchmark-flower.ps1`, the GUI9 regression tour)
keep reading it. The browser scenario change needs the tester's browser run to
confirm — it is a path swap with the same intent, not a behavioural change.

## Still open

* Open question 6 (which anchor should be the *default*) stays with the tester.
  The default here is deliberately today's min-corner behaviour; the choice is
  visible and changeable in the same panel, and it is workspace state, so
  changing the default later costs nothing in job compatibility.
* Open question 4 (thin-feature engraving depth) still blocks W6 and the
  "accept a stroke as a centerline in fill mode" variant of W4 step 4. The
  refusal branch landed instead.
* Warnings are listed on the importer's existing diagnostics channel, which is
  what `cam import` / `cam inspect` print (the artifact shows file C's six
  `SVG_STYLE_IGNORED` lines). The v5 artwork catalogue keeps a per-item
  `import_error` but not the importer's warnings, so the GUI still shows only
  fatal import failures. Threading the warnings into the artwork panel is a
  small follow-up — the catalogue would carry them and the panel would list
  them next to the rejected files — and it is deliberately not invented here:
  the plan's step 2 points at the existing channel.
* The GUI change is visible, so a review package is rebuilt in
  `artifacts/gui/native/`. The browser review scenarios
  (`crates/cam-gui/web/*.mjs`) need the tester's browser run; the new control
  labels are `Stock resize anchor`, `Stock anchor Min corner` /
  `Stock anchor Stock centre` / `Stock anchor Artwork bounds`,
  `Stock from artwork bounds`, and the notice probe
  `Stock artwork outside`.
