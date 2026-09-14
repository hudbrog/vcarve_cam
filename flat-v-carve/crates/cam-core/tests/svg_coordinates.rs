//! W5 — the page-to-workpiece coordinate contract, pinned on an A4 fixture in
//! millimetres. The importer flips Y exactly once, about the physical page
//! height: the page's top edge becomes the artwork's maximum Y, and the page's
//! bottom-left corner becomes the setup origin at the default placement.
use cam_core::{
    geometry::Point,
    svg::{Bounds, ImportOptions, Placement, import_svg, page_to_artwork},
};

const A4: &str = include_str!("../../../fixtures/fieldtest/a4-corner-square.svg");

fn near(a: f64, b: f64, e: f64) {
    assert!((a - b).abs() <= e, "{a} != {b} +/- {e}");
}

fn bounds_of(geometry: &cam_core::svg::NormalizedGeometry, source: &str) -> [f64; 4] {
    let component = geometry
        .sources
        .iter()
        .find(|s| s.source_id == source)
        .unwrap_or_else(|| panic!("no source '{source}'"));
    let b = Bounds::of(&component.geometry).unwrap();
    [b.min.x, b.min.y, b.max.x, b.max.y]
}

/// Curved shapes are flattened, so bounds carry the usual flattening budget.
fn assert_bounds(geometry: &cam_core::svg::NormalizedGeometry, source: &str, expected: [f64; 4]) {
    let actual = bounds_of(geometry, source);
    for (a, e) in actual.iter().zip(expected) {
        near(*a, e, 0.001);
    }
}

#[test]
fn the_page_top_edge_becomes_the_maximum_y_edge_of_the_artwork() {
    let geometry = import_svg(A4, &ImportOptions::default(), None).unwrap();
    near(geometry.page_width_mm, 210., 1e-9);
    near(geometry.page_height_mm, 297., 1e-9);
    // Drawn at the XML origin — the page's visual top-left corner — so it
    // stands against the stock's maximum-Y edge: 297 - 20 … 297.
    assert_bounds(&geometry, "top-left", [0., 277., 20., 297.]);
    // The page's bottom-left corner is the setup origin.
    assert_bounds(&geometry, "bottom-left", [0., 0., 20., 20.]);
    // A circle tangent to the page's top edge stays tangent to the far edge.
    assert_bounds(&geometry, "top-tangent", [95., 277., 115., 297.]);
    // The drawn geometry spans the two corners; the page itself is the full
    // 210 x 297 the stock capture uses.
    let b = geometry.bounds.unwrap();
    assert_eq!([b.min.x, b.min.y, b.max.x, b.max.y], [0., 0., 115., 297.]);
}

#[test]
fn the_documented_mapping_flips_y_once_and_the_placement_then_inverts() {
    // The one documented conversion. Nothing downstream flips Y again.
    assert_eq!(
        page_to_artwork(Point::new(0., 0.), 297.),
        Point::new(0., 297.)
    );
    assert_eq!(
        page_to_artwork(Point::new(0., 297.), 297.),
        Point::new(0., 0.)
    );
    assert_eq!(
        page_to_artwork(Point::new(21., 42.), 297.),
        Point::new(21., 255.)
    );
    // Placement is the second, independent step: it works in artwork
    // millimetres and is the identity at its default.
    let placement = Placement::default();
    for artwork in [
        Point::new(0., 0.),
        Point::new(210., 297.),
        Point::new(17.5, 42.25),
    ] {
        assert_eq!(placement.to_setup(artwork).unwrap(), artwork);
        assert_eq!(placement.to_page(artwork).unwrap(), artwork);
    }
    // Composed, an SVG point reaches setup through the flip exactly once.
    let geometry = import_svg(A4, &ImportOptions::default(), None).unwrap();
    let corner = placement
        .to_setup(page_to_artwork(Point::new(0., 0.), geometry.page_height_mm))
        .unwrap();
    assert_eq!(corner, Point::new(0., 297.));
    let b = Bounds::of(
        &geometry
            .sources
            .iter()
            .find(|s| s.source_id == "top-left")
            .unwrap()
            .geometry,
    )
    .unwrap();
    near(b.max.y, corner.y, 1e-9);
    near(b.min.x, corner.x, 1e-9);
}

#[test]
fn a_hundred_millimetre_page_pins_the_plans_own_example() {
    // The reproduction baseline in the field-test plan: a 100x100 page with a
    // circle tangent to the page's top edge reports y = 80 … 100.
    let raw = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100">
        <circle id="c" cx="50" cy="10" r="10" fill="#000" />
    </svg>"##;
    let geometry = import_svg(raw, &ImportOptions::default(), None).unwrap();
    assert_bounds(&geometry, "c", [40., 80., 60., 100.]);
    // Placement of the same page, one axis at a time: nothing re-scales.
    for (origin, scale) in [(Point::new(5., 5.), 1.), (Point::new(0., 0.), 2.)] {
        let options = ImportOptions {
            placement: Placement {
                origin_mm: origin,
                scale,
                rotation_deg: 0.,
            },
            ..Default::default()
        };
        let placed = import_svg(raw, &options, None).unwrap();
        let b = Bounds::of(
            &placed
                .sources
                .iter()
                .find(|s| s.source_id == "c")
                .unwrap()
                .geometry,
        )
        .unwrap();
        near(b.min.x, (40. - origin.x) * scale, 0.001);
        near(b.min.y, (80. - origin.y) * scale, 0.001);
        near(b.max.x, (60. - origin.x) * scale, 0.001);
        near(b.max.y, (100. - origin.y) * scale, 0.001);
    }
}
