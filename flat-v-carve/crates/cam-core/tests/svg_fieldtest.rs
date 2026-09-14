//! W4 — Inkscape/SVG import compatibility, checked against the fixtures in
//! `fixtures/fieldtest/`. The report's circle either imports through its CSS
//! `<style>` block or is refused with a message that names the element and the
//! way out.
use cam_core::geometry::Severity;
use cam_core::svg::{ImportOptions, NormalizedGeometry, import_svg};

fn read(raw: &str) -> NormalizedGeometry {
    import_svg(raw, &ImportOptions::default(), None).unwrap()
}
fn near(a: f64, b: f64, e: f64) {
    assert!((a - b).abs() <= e, "{a} != {b} +/- {e}");
}
fn warning_bodies(geometry: &NormalizedGeometry) -> Vec<String> {
    geometry
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .map(|d| d.message.clone())
        .collect()
}

const STROKE: &str = include_str!("../../../fixtures/fieldtest/inkscape-circle-stroke.svg");
const STROKE_TO_PATH: &str =
    include_str!("../../../fixtures/fieldtest/inkscape-circle-stroke-to-path.svg");
const LAYER: &str =
    include_str!("../../../fixtures/fieldtest/inkscape-layer-enable-background.svg");
const CASCADE: &str = include_str!("../../../fixtures/fieldtest/inkscape-named-colour.svg");
const EXTERNAL: &str = include_str!("../../../fixtures/fieldtest/inkscape-external-stylesheet.svg");

#[test]
fn a_stroke_only_circle_imports_as_a_line_and_says_what_it_lost() {
    // The report's first file: a circle with a stroke and no fill. It is a
    // drawn line, so it imports as one — and the reading names the element,
    // the width it discarded and the way to get an outline instead.
    let geometry = read(STROKE);
    assert_eq!(geometry.chains.len(), 1);
    assert!(geometry.sources.is_empty(), "a stroke is not an area");
    let reading = geometry
        .diagnostics
        .iter()
        .find(|d| d.code == "SVG_STROKE_CENTERLINE")
        .expect("the stroke reading is reported");
    assert_eq!(reading.source_id.as_deref(), Some("circle1"));
    assert!(reading.message.contains("circle1"), "{reading}");
    assert!(reading.message.contains("Test circle"), "{reading}");
    assert!(reading.message.contains("1 mm"), "{reading}");
    assert!(reading.message.contains("Stroke to Path"), "{reading}");
    assert!(
        !geometry
            .diagnostics
            .iter()
            .any(|d| matches!(d.severity, Severity::Error)),
        "{:?}",
        geometry.diagnostics
    );
}

#[test]
fn the_stroke_to_path_circle_imports_through_its_stylesheet() {
    let geometry = read(STROKE_TO_PATH);
    near(geometry.page_width_mm, 210., 1e-9);
    near(geometry.page_height_mm, 297., 1e-9);
    assert_eq!(geometry.sources.len(), 1);
    assert_eq!(geometry.sources[0].source_id, "path1");
    assert_eq!(geometry.sources[0].label.as_deref(), Some("Test circle"));
    // A 1 mm stroke around a Ø40 circle becomes a ring: one hole, and an area
    // of pi(20.5² - 19.5²) = 40pi. Getting a disc instead would mean the
    // `fill-rule` in the class rule never reached the geometry.
    assert_eq!(geometry.sources[0].geometry.hole_count(), 1);
    let ring = std::f64::consts::PI * (20.5 * 20.5 - 19.5 * 19.5);
    near(geometry.selected.area_mm2(), ring, 0.05);
    // No XML editing, no unexplained refusal: the stylesheet is simply used.
    assert!(
        !geometry
            .diagnostics
            .iter()
            .any(|d| matches!(d.severity, Severity::Error)),
        "{:?}",
        geometry.diagnostics
    );
}

#[test]
fn editor_properties_are_warnings_and_selector_specificity_is_honoured() {
    let geometry = read(LAYER);
    // `.plate` at 40x40, plus the ring where the `.ring` class must beat the
    // `path` element rule: even-odd gives 20x20 - 10x10 = 300, nonzero 400.
    near(geometry.selected.area_mm2(), 1600. + 300., 1e-6);
    let warnings = warning_bodies(&geometry);
    for property in [
        "enable-background",
        "-inkscape-font-specification",
        "stroke-linecap",
    ] {
        assert!(
            warnings.iter().any(|w| w.contains(property)),
            "no warning names {property}: {warnings:?}"
        );
    }
    // The warnings name the element they came from.
    assert!(
        geometry
            .diagnostics
            .iter()
            .any(|d| d.code == "SVG_STYLE_IGNORED" && d.source_id.as_deref() == Some("layer1")),
        "{:?}",
        geometry.diagnostics
    );
    assert!(
        geometry
            .diagnostics
            .iter()
            .all(|d| d.severity == Severity::Warning),
        "{:?}",
        geometry.diagnostics
    );
}

#[test]
fn important_and_class_rules_win_over_inline_and_presentation_attributes() {
    let geometry = read(CASCADE);
    // named-1 is transparent by `#named-1 { fill: transparent !important }`,
    // which must beat its inline `fill:yellow`; named-2 is painted by
    // `.opaque { fill: navy }`, which must beat its `fill="transparent"`.
    near(geometry.selected.area_mm2(), 800., 1e-6);
    assert_eq!(geometry.sources.len(), 1);
    assert_eq!(geometry.sources[0].source_id, "named-2");
    assert!(
        geometry
            .diagnostics
            .iter()
            .any(|d| d.code == "SVG_NO_PAINT" && d.source_id.as_deref() == Some("named-1")),
        "{:?}",
        geometry.diagnostics
    );
}

#[test]
fn an_external_stylesheet_stays_a_hard_failure() {
    let error = import_svg(EXTERNAL, &ImportOptions::default(), None).unwrap_err();
    assert_eq!(error.code, "SVG_STYLESHEET");
}

#[test]
fn an_unsupported_selector_is_reported_rather_than_silently_dropped() {
    let raw = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="60mm" viewBox="0 0 100 60">
        <style>rect[width] { display: none } .keep { fill: black }</style>
        <rect id="box" class="keep" x="10" y="10" width="80" height="40" />
    </svg>"#;
    let geometry = read(raw);
    // The unsupported attribute selector is not applied, so the rectangle
    // stays; the skipped rule is named in a warning.
    near(geometry.selected.area_mm2(), 3200., 1e-6);
    assert!(
        geometry
            .diagnostics
            .iter()
            .any(|d| d.code == "SVG_STYLE_SELECTOR" && d.message.contains("rect[width]")),
        "{:?}",
        geometry.diagnostics
    );
}

#[test]
fn a_malformed_inline_declaration_still_stops_the_import() {
    let raw = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="60mm" viewBox="0 0 100 60">
        <rect id="box" style="fill" x="10" y="10" width="80" height="40" />
    </svg>"#;
    let error = import_svg(raw, &ImportOptions::default(), None).unwrap_err();
    assert_eq!(error.code, "SVG_STYLE");
    assert_eq!(error.source_id.as_deref(), Some("box"));
}

#[test]
fn a_source_keeps_its_name_layer_and_colour_as_identity() {
    // Colour and layer never decide geometry: two shapes that differ only in
    // colour cut identically. They are how the drawing is recognised, though,
    // so they travel with the geometry all the way to the pickers.
    let geometry = read(LAYER);
    let rect = geometry
        .sources
        .iter()
        .find(|source| source.source_id == "plate-1")
        .expect("the inset plate");
    assert_eq!(rect.label.as_deref(), Some("Inset plate"));
    assert_eq!(rect.group.as_deref(), Some("Layer 1"));
    // The colour is the one the cascade resolved, not the one an attribute
    // happens to spell: `.plate.inset` beats `.plate` on specificity.
    assert_eq!(rect.paint.expect("a solid fill").hex(), "#404040");
    let path = geometry
        .sources
        .iter()
        .find(|source| source.source_id == "ring-1")
        .expect("the ring");
    assert_eq!(path.label.as_deref(), Some("Ring"));
    assert_eq!(path.group.as_deref(), Some("Layer 1"));
    assert_eq!(path.paint.expect("a solid fill").hex(), "#808080");

    // A stroke with no fill takes its colour from the stroke, and the chain
    // carries the same identity as the shape it came from.
    let stroke = read(STROKE);
    let circle = stroke
        .chains
        .iter()
        .find(|chain| chain.source_id == "circle1")
        .expect("the stroked circle");
    assert_eq!(circle.label.as_deref(), Some("Test circle"));
    assert_eq!(circle.group.as_deref(), Some("Layer 1"));
    assert_eq!(circle.paint.expect("a solid stroke").hex(), "#000000");
}
