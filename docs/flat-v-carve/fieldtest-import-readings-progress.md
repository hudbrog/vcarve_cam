# Import readings: no interpretation mode, and identity that survives to the pickers

Date: 2026-09-14\
Context: the compatibility policy in
[field-testing-fixes-plan.md](field-testing-fixes-plan.md) §0, plus slices A and
B agreed with the tester after the W4/W5 batch
([fieldtest-w4w5-progress.md](fieldtest-w4w5-progress.md)).\
Engine 0.7.7.

## The problem this removes

The importer used to ask the user a question the file cannot answer: is this
drawing an area or a line? `ImportMode::Fill` refused anything with a visible
stroke, `ImportMode::Centerline` refused an element that had both a fill and a
stroke, and which one applied was decided by *which operation happened to be
selected when the artwork was imported* — with no control anywhere to change it
afterwards. A mixed drawing (a filled carving plus a stroked outline of the
part) could not be imported at all: one offending element failed the whole
file.

## Slice A — the importer publishes every reading; the operation picks

`ImportMode` is deleted. One flattening pass (`svg/path.rs`) yields every
subpath as drawn, and each element publishes what it can honestly be:

| Reading | Produced when | Consumed by |
| --- | --- | --- |
| Region | visible fill, subpath of 3+ vertices | Flat V-carve (filled component); Profile (its region boundary) |
| Centreline | visible fill or stroke, subpath of 2+ vertices | Drag knife; a Profile may cut the closed ones |
| Closed contour | closed subpath on an element that has no fill (so no region already describes it) | Profile |

Consequences the tester asked for, all asserted by tests:

* a filled shape offers its own drawn outline to a knife — no "create knife
  outlines" step, no Stroke to Path, and it *cuts* (the configured-knife test
  plans it to `exportReady` with motion);
* an outline drawn as a closed stroked path is selectable by a profile as well
  as by the knife — the "carving + part outline" workflow works from one import;
* an element with both a fill and a stroke is both an area and a line instead of
  an error telling the user to separate them;
* a file where nothing draws at all is still refused.

Per-element diagnostics replace file-level failures, each naming the element
(`id` and `inkscape:label`) and what it lost: `SVG_NO_PAINT`,
`SVG_STROKE_CENTERLINE` (naming the discarded width and pointing at Stroke to
Path), `SVG_OPEN_PATH` (a filled open subpath is closed where every renderer
closes it), `SVG_DEGENERATE_PATH`, `SVG_PAINT_OPACITY` (a gradient or pattern
fill is an opaque region; its opacity cannot be known without rendering it).
Text, references, masks, clip paths, filters, external stylesheets and the CSS
`transform` property stay hard failures, because they change what geometry
exists and this importer cannot describe it.

## Slice B — identity: name, layer and colour, never geometry

Two shapes that differ only in colour or layer cut identically, so neither is
used to decide anything. They are how a person recognises their own drawing, so
they now travel with the geometry:

* `svg::SourcePaint` (resolved solid colour), the element's `inkscape:label`,
  and the nearest enclosing named layer ride on every source component and
  chain, through `Contour` and `CatalogueEntry`, into the GUI payloads
  (`authoring::Component`, `knife::Chain`, `profile::Contour`);
* the pickers name rows from them (`Flower`, not `carve::0`) with a colour
  swatch, and offer one *Select all in <layer>* button per layer and one
  clickable swatch per colour;
* the viewport draws excluded artwork in the colour the drawing used, while
  selected geometry keeps the operation's cyan so selection stays legible.

Colour is resolved through the CSS cascade, so it is the colour the drawing
actually shows rather than the attribute that happens to spell it — asserted on
the `.plate.inset` fixture.

## Evidence

| Command | Result |
| --- | --- |
| `cargo test --workspace --no-fail-fast` | 79 binaries, 711 tests passed, 0 failed (exit 0) |
| `cargo clippy --workspace --all-targets` | clean (exit 0) |
| `cargo fmt --all -- --check` | clean |

New and changed tests worth naming:

| Test | What it pins |
| --- | --- |
| `knife_import.rs::a_closed_outline_is_a_contour_for_profile_and_a_knife` | one drawn outline, two usable readings, and no duplicate contour for a filled element |
| `knife_import.rs::one_element_can_be_an_area_and_a_line_at_once` | fill + stroke is a reading, not an error |
| `knife_import.rs::a_filled_open_subpath_is_closed_for_filling_and_says_so` | implicit closing, reported |
| `knife_import.rs::a_gradient_fill_imports_as_an_opaque_region_with_a_warning` | gradients no longer block a file |
| `cam-gui/tests/knife.rs` | a filled shape's outline selects *and* plans to completion |
| `svg_fieldtest.rs::a_source_keeps_its_name_layer_and_colour_as_identity` | label, layer and cascade-resolved colour on components and chains |
| `operation_ui.rs::carving_rows_are_named_and_layered_and_offer_bulk_selection` | named rows, the layer button and the per-colour swatches |
| `scene.rs::artwork_is_drawn_in_its_own_colour_until_it_is_selected` | viewport colour, and selection outranking it |

## Stored data brought forward (not compatibility)

Two kinds of stored file changed content, both by rewriting them to the current
schema rather than by keeping a shim:

* the tester's knife jobs and the knife/GUI6 fixtures carried
  `"import_settings": { "mode": "centerline" }`, a field that no longer exists.
  The field was removed from `real_data/knife`, `real_data/knife_flower`,
  `fixtures/gui6/knife.job.json`, `fixtures/gui6/flower-knife.job.json` and
  `fixtures/knife/real-outline-0.25.job.json`;
* a source revision is a digest of the item's content *and* its import settings,
  so removing the field invalidated every saved selection that was bound to the
  old digest. `cargo run -p cam-core --example rebind_fixture_revisions -- FILE`
  re-binds a job's selections to their items' current revisions without
  touching geometry; it was run over the same five files (17 and 15 references
  in the two real jobs).

## Known follow-ups

* *Create knife outlines* is now redundant (a filled shape already offers its
  outline); the panel says so, and removing the action is a separate decision.
* The GUI still shows importer warnings only for fatal failures: the v5 artwork
  catalogue carries an `import_error` but not the importer's diagnostics, so the
  new per-element warnings are visible in `cam import` / `cam inspect` output
  and not yet in the artwork panel.
* Open question 9 (should the `cam` CLI speak schema 5, leaving one document
  model) is untouched by this slice.
