//! Import readings (plan section 7.3 and the W4 slice of the field-test plan):
//! the importer publishes every reading a drawing supports — each subpath as a
//! centreline, each filled subpath as a region — and the operation that
//! consumes the artwork picks the one it needs. Nothing is doubled: a stroke
//! is never offset into two parallel cuts, and an open subpath is never closed
//! unless a fill makes it an area.
use cam_core::{
    contours::{ContourCatalogue, ContourRole},
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    project::{CamJob, ContourAnchor, ContourSide, SetupSettings, StockSetup},
    svg::{ImportOptions, Placement, import_svg},
};

const OPEN_CUTS: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="cuts" fill="none" stroke="#000" stroke-width="0.5" d="M1 2 L11 2 M20 2 L20 7"/></svg>"##;
const ONE_STROKE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="edge" fill="none" stroke="#000" stroke-width="2" d="M5 5 L15 5"/></svg>"##;
const MIXED_ELEMENTS: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="mixed" fill="none" stroke="#000" d="M2 2 L8 2 L8 8 Z M12 2 L18 2"/><line id="seg" x1="22" y1="2" x2="22" y2="8" stroke="#000" stroke-width="0.4"/><polyline id="pl" points="26,2 30,2 30,6" fill="none" stroke="#000" stroke-width="0.4"/><polygon id="pg" points="33,2 38,2 38,6" fill="none" stroke="#000" stroke-width="0.4"/></svg>"##;
/// A filled plate, a stroked line, and one element that is both an area and a
/// line at once.
const FILL_AND_STROKE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><rect id="plate" x="2" y="2" width="10" height="6" fill="#fff"/><path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M20 4 L30 4"/><path id="both" fill="#fff" stroke="#000" stroke-width="0.4" d="M1 20 L10 20 L10 26 Z"/></svg>"##;
/// A filled plate plus one stroked centerline: the two readings side by side.
const COEXIST: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><rect id="plate" x="2" y="2" width="10" height="6" fill="#fff"/><path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M20 4 L30 4"/></svg>"##;
const EDITED_CUTS: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="cuts" fill="none" stroke="#000" stroke-width="0.5" d="M1 2 L12 2 M20 2 L20 7"/></svg>"##;

fn job(svg: &str, placement: Placement) -> CamJob {
    CamJob {
        name: "knife-import".into(),
        source: Some(SourceSnapshot {
            filename: "art.svg".into(),
            svg: svg.into(),
        }),
        import: ImportOptions {
            placement,
            ..Default::default()
        },
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(8.),
                xy: None,
            },
            ..Default::default()
        },
        tools: vec![],
        operations: vec![],
        tolerances: PlanningTolerances::default(),
        legacy_machine_profile: None,
    }
}

fn catalogue(svg: &str) -> ContourCatalogue {
    ContourCatalogue::build(&job(svg, Placement::default())).unwrap()
}

/// The page Y axis flips on import (30 mm page): page (x, y) → setup (x, 30-y).
fn setup_point(x: f64, y: f64) -> Point {
    Point::new(x, 30. - y)
}

fn has_diagnostic(geometry: &cam_core::svg::NormalizedGeometry, code: &str, source: &str) -> bool {
    geometry
        .diagnostics
        .iter()
        .any(|d| d.code == code && d.source_id.as_deref() == Some(source))
}

#[test]
fn open_paths_stay_open_and_subpath_order_is_preserved() {
    let catalogue = catalogue(OPEN_CUTS);
    assert!(catalogue.contours.is_empty(), "no filled regions here");
    // One chain per subpath, in document and subpath order.
    let ids: Vec<&str> = catalogue
        .open_chains
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(ids, vec!["cuts-chain-0", "cuts-chain-1"]);
    for chain in &catalogue.open_chains {
        assert!(
            !chain.closed,
            "{} stays open; nothing is closed implicitly",
            chain.id
        );
        assert_eq!(chain.role, ContourRole::Open);
        assert_eq!(chain.suggested_side(), ContourSide::On);
        // Source vertex order: the chain starts where the drawing starts.
        assert_eq!(
            chain.vertices.len(),
            2,
            "{} keeps its two endpoints",
            chain.id
        );
    }
    let first = catalogue.chain("cuts-chain-0").unwrap();
    assert_eq!(first.vertices[0], setup_point(1., 2.));
    assert_eq!(first.vertices[1], setup_point(11., 2.));
    assert!((first.perimeter_mm - 10.).abs() < 1e-9);
    let second = catalogue.chain("cuts-chain-1").unwrap();
    assert_eq!(second.vertices[0], setup_point(20., 2.));
    assert_eq!(second.vertices[1], setup_point(20., 7.));
    // Exact chain selection: unknown references are located errors.
    let selected = catalogue
        .select_chains(&["cuts-chain-1".to_string()])
        .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id, "cuts-chain-1");
    assert_eq!(
        catalogue
            .select_chains(&["cuts-chain-9".to_string()])
            .unwrap_err()
            .code,
        "CONTOUR_REFERENCE"
    );
}

#[test]
fn stroke_centerlines_are_not_doubled() {
    // Exactly one chain rides the stroke's centerline; the 2 mm stroke width
    // is ignored, never offset into two parallel cuts. There is no fill, so
    // there is no region either.
    let geometry = import_svg(ONE_STROKE, &ImportOptions::default(), None).unwrap();
    assert_eq!(geometry.chains.len(), 1, "one stroke is one centerline");
    let chain = &geometry.chains[0];
    assert!(!chain.closed);
    assert_eq!(chain.points.len(), 2);
    assert!((chain.points[0].x - 5.).abs() < 1e-9 && (chain.points[1].x - 15.).abs() < 1e-9);
    assert!((chain.points[0].y - chain.points[1].y).abs() < 1e-9);
    assert!(geometry.sources.is_empty(), "a stroke is not an area");
    // The reading is visible as a diagnostic naming the element and the width
    // it discarded.
    assert!(has_diagnostic(&geometry, "SVG_STROKE_CENTERLINE", "edge"));
    let reading = geometry
        .diagnostics
        .iter()
        .find(|d| d.code == "SVG_STROKE_CENTERLINE")
        .unwrap();
    assert!(reading.message.contains("2 mm"), "{}", reading.message);
    // The catalogue agrees: exactly one chain of length 10.
    let catalogue = catalogue(ONE_STROKE);
    assert_eq!(catalogue.open_chains.len(), 1);
    assert!((catalogue.open_chains[0].perimeter_mm - 10.).abs() < 1e-9);
}

#[test]
fn closed_subpaths_line_and_polyline_elements_import_as_chains() {
    let catalogue = catalogue(MIXED_ELEMENTS);
    let ids: Vec<&str> = catalogue
        .open_chains
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec![
            "mixed-chain-0",
            "mixed-chain-1",
            "seg-chain-0",
            "pl-chain-0",
            "pg-chain-0"
        ]
    );
    // An explicitly closed subpath is one closed chain without a duplicated
    // closing vertex; the following open subpath stays open.
    let closed = catalogue.chain("mixed-chain-0").unwrap();
    assert!(closed.closed);
    assert_eq!(closed.vertices.len(), 3);
    assert_eq!(closed.vertices[0], setup_point(2., 2.));
    let open = catalogue.chain("mixed-chain-1").unwrap();
    assert!(!open.closed);
    assert_eq!(open.vertices.len(), 2);
    // <line> imports as an open chain of exactly its endpoints.
    let line = catalogue.chain("seg-chain-0").unwrap();
    assert!(!line.closed);
    assert!((line.perimeter_mm - 6.).abs() < 1e-9);
    // <polyline> never gains a closing edge: its perimeter is the drawn
    // length (4 + 4), not a closed loop.
    let polyline = catalogue.chain("pl-chain-0").unwrap();
    assert!(!polyline.closed);
    assert!((polyline.perimeter_mm - 8.).abs() < 1e-9);
    // <polygon> is closed by definition.
    let polygon = catalogue.chain("pg-chain-0").unwrap();
    assert!(polygon.closed);
    assert_eq!(polygon.vertices.len(), 3);
}

#[test]
fn a_closed_outline_is_a_contour_for_profile_and_a_chain_for_the_knife() {
    // The workflow the tester described: the outline of the finished part is
    // drawn as a closed path with a stroke and no fill. It has to be usable by
    // a profile (as a boundary) and by a knife (as a line) without converting
    // anything in Inkscape first.
    let mixed = catalogue(MIXED_ELEMENTS);
    let contour = mixed.contour("mixed-chain-0-outline").unwrap();
    assert!(contour.closed);
    assert_eq!(contour.role, ContourRole::Outer);
    // The stored ring is canonicalized (direction and start point are the
    // catalogue's, not the file's), so compare it as the same set of corners.
    assert_eq!(sorted(&contour.vertices), sorted(&closed_vertices()));
    let leg = 6.0f64;
    assert!(
        (contour.perimeter_mm - (2. * leg + leg * 2.0f64.sqrt())).abs() < 1e-9,
        "two 6 mm legs and the hypotenuse: {}",
        contour.perimeter_mm
    );
    // The same drawn line is the knife's chain.
    assert_eq!(mixed.chain("mixed-chain-0").unwrap().vertices.len(), 3);
    // An open subpath is not a boundary, so it never becomes a contour.
    assert!(mixed.contour("mixed-chain-1-outline").is_none());
    // A polygon with a stroke is closed too, so it works for both.
    assert!(mixed.contour("pg-chain-0-outline").is_some());
    // A filled element already contributes its rings, so its chain is not
    // listed as a second contour with the same geometry.
    let coexisting = catalogue(COEXIST);
    assert_eq!(coexisting.contours.len(), 1, "the filled plate's outline");
    assert_eq!(coexisting.open_chains.len(), 2, "plate path + stroked cut");
    assert!(
        coexisting.contour("plate-chain-0-outline").is_none(),
        "a filled element's outline comes from its region, not twice"
    );
    let _ = ContourSide::Outside;
}

fn closed_vertices() -> Vec<Point> {
    vec![
        setup_point(2., 2.),
        setup_point(8., 2.),
        setup_point(8., 8.),
    ]
}

fn sorted(points: &[Point]) -> Vec<(i64, i64)> {
    let mut rounded: Vec<(i64, i64)> = points
        .iter()
        .map(|p| ((p.x * 1e6).round() as i64, (p.y * 1e6).round() as i64))
        .collect();
    rounded.sort_unstable();
    rounded
}

#[test]
fn one_element_can_be_an_area_and_a_line_at_once() {
    // A filled plate and a stroked line read side by side, exactly as before.
    let coexisting = catalogue(COEXIST);
    assert_eq!(coexisting.contours.len(), 1, "the filled plate");
    assert_eq!(coexisting.open_chains.len(), 2);
    assert_eq!(coexisting.open_chains[1].id, "cut-chain-0");

    // An element with both a visible fill and a visible stroke is now both: a
    // region to carve and a line to cut. It used to be a hard error telling
    // the user to separate them; there is nothing to separate any more.
    let geometry = import_svg(FILL_AND_STROKE, &ImportOptions::default(), None).unwrap();
    assert!(
        geometry.diagnostics.iter().all(|d| d.code != "SVG_STROKE"),
        "a fill and a stroke on one element is a reading, not an error: {:?}",
        geometry.diagnostics
    );
    assert_eq!(geometry.sources.len(), 2, "plate and the filled triangle");
    assert_eq!(
        geometry.chains.len(),
        3,
        "plate outline, the cut, and the triangle's own outline"
    );
    let both = catalogue(FILL_AND_STROKE);
    // The triangle is selectable as an area (v-carve) and as a line (knife).
    assert!(both.contour("both-0-outer").is_some());
    assert!(both.chain("both-chain-0").unwrap().closed);
}

#[test]
fn a_filled_open_subpath_is_closed_for_filling_and_says_so() {
    // Every renderer closes an open subpath when it fills it. CAM does the
    // same, because that is what the user sees, and reports the added edge.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="open-area" fill="#000" d="M4 4 L20 4 L20 16" /></svg>"##;
    let geometry = import_svg(svg, &ImportOptions::default(), None).unwrap();
    assert_eq!(geometry.sources.len(), 1, "the implicit closing edge");
    assert!(has_diagnostic(&geometry, "SVG_OPEN_PATH", "open-area"));
    let reading = geometry
        .diagnostics
        .iter()
        .find(|d| d.code == "SVG_OPEN_PATH")
        .unwrap();
    assert!(
        reading.message.contains("closing edge is added"),
        "{}",
        reading.message
    );
    // It is a line as well, and that reading stays open.
    assert_eq!(geometry.chains.len(), 1);
    assert!(!geometry.chains[0].closed);
}

#[test]
fn invisible_elements_contribute_nothing_and_say_so() {
    // A line with no stroke draws nothing: no chain may be invented for it,
    // even though the file has another element that does draw.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><rect id="plate" x="2" y="2" width="10" height="6" fill="#fff"/><line id="bare" x1="20" y1="1" x2="28" y2="9"/></svg>"##;
    let geometry = import_svg(svg, &ImportOptions::default(), None).unwrap();
    // The plate contributes its outline as a chain; the bare line contributes
    // nothing at all.
    assert!(geometry.chains.iter().all(|c| c.source_id == "plate"));
    assert!(has_diagnostic(&geometry, "SVG_NO_PAINT", "bare"));
    // A file where *nothing* draws at all is refused rather than imported
    // empty, and says why.
    let empty = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><line id="bare" x1="1" y1="1" x2="9" y2="9"/></svg>"##;
    assert_eq!(
        import_svg(empty, &ImportOptions::default(), None)
            .unwrap_err()
            .code,
        "SVG_NO_REGIONS"
    );
}

#[test]
fn a_gradient_fill_imports_as_an_opaque_region_with_a_warning() {
    // The area is unambiguous even when the paint is not; evaluating a
    // gradient's opacity would need a renderer, so it is assumed opaque and
    // said out loud.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><defs><linearGradient id="g"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#000"/></linearGradient></defs><rect id="shaded" x="4" y="4" width="16" height="10" fill="url(#g)" /></svg>"##;
    let geometry = import_svg(svg, &ImportOptions::default(), None).unwrap();
    assert_eq!(geometry.sources.len(), 1);
    assert!((geometry.selected.area_mm2() - 160.).abs() < 1e-6);
    assert!(has_diagnostic(&geometry, "SVG_PAINT_OPACITY", "shaded"));
}

#[test]
fn placement_moves_chains_and_anchors_resolve_on_source_geometry() {
    let identity = catalogue(OPEN_CUTS);
    let chain = identity.chain("cuts-chain-0").unwrap();
    // Halfway along the 10 mm first cut.
    let anchor = ContourAnchor {
        contour_id: chain.id.clone(),
        source_geometry_fingerprint: chain.source_fingerprint.clone(),
        fraction_along_source_contour: 0.5,
    };
    let resolved = identity.resolve_anchor(&anchor).unwrap();
    assert!(
        resolved.point.distance(setup_point(6., 2.)) < 1e-9,
        "anchor resolves halfway along the chain: {:?}",
        resolved.point
    );
    // The tangent points along the drawn direction (toward +X here).
    assert!((resolved.tangent.x - 1.).abs() < 1e-9 && resolved.tangent.y.abs() < 1e-9);

    // Same artwork, placement: scale 2, rotate 90 degrees about the origin.
    let moved = ContourCatalogue::build(&job(
        OPEN_CUTS,
        Placement {
            origin_mm: Point::new(0., 0.),
            scale: 2.,
            rotation_deg: 90.,
        },
    ))
    .unwrap();
    let moved_chain = moved.chain("cuts-chain-0").unwrap();
    assert_eq!(
        moved_chain.id, chain.id,
        "chain identity is placement-independent"
    );
    assert_eq!(
        moved_chain.source_fingerprint, chain.source_fingerprint,
        "source identity does not move with the artwork"
    );
    let moved_resolved = moved.resolve_anchor(&anchor).unwrap();
    let expected = Point::new(-2. * resolved.point.y, 2. * resolved.point.x);
    assert!(
        moved_resolved.point.distance(expected) < 1e-6,
        "anchor moves with the artwork: {:?} != {expected:?}",
        moved_resolved.point
    );
    // The chain vertices themselves moved the same way.
    assert!(
        moved_chain.vertices[0].distance(Point::new(
            -2. * chain.vertices[0].y,
            2. * chain.vertices[0].x
        )) < 1e-6
    );

    // A source edit leaves the anchor explicitly unresolved.
    let edited = ContourCatalogue::build(&job(EDITED_CUTS, Placement::default())).unwrap();
    let error = edited.resolve_anchor(&anchor).unwrap_err();
    assert_eq!(error.code, "CONTOUR_ANCHOR_UNRESOLVED");
    assert!(error.message.contains("reattach"));
}

#[test]
fn near_zero_jogs_collapse_but_orientation_corners_survive() {
    // A jog below the declared budget (tolerance/4 = 0.25 µm at the default
    // 1 µm import tolerance) merges away; a near-reversal with the same tiny
    // deviation is a corner the knife needs and always survives.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="jogs" fill="none" stroke="#000" stroke-width="0.4" d="M1 10 L20 10 L20.00005 10.0001 L20 10.5 M1 20 L10 20 L10 20.0002 L10 19.8"/></svg>"##;
    let catalogue = catalogue(svg);
    let merged = catalogue.chain("jogs-chain-0").unwrap();
    // The 0.05 µm jog at (20.00005, 10.0001) is within budget and under the
    // corner threshold: three vertices remain.
    assert_eq!(merged.vertices.len(), 3, "sub-tolerance jog merged");
    let corner = catalogue.chain("jogs-chain-1").unwrap();
    // The reversal at (10, 20.0002) deviates far less than a chain width but
    // turns ~180 degrees: the corner vertex survives for knife orientation.
    assert_eq!(corner.vertices.len(), 4, "near-reversal corner kept");
    assert!(corner.vertices.contains(&setup_point(10., 20.0002)));
}
