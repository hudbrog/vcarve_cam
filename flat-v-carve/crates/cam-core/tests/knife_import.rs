//! F1 open-chain import (plan section 7.3): a separate centerline import
//! mode keeps open paths open, never doubles a stroke into two parallel
//! cuts, preserves subpath order, and leaves the fill importer untouched.
use cam_core::{
    contours::{ContourCatalogue, ContourRole},
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    project::{CamJob, ContourAnchor, ContourSide, SetupSettings, StockSetup},
    svg::{ImportMode, ImportOptions, Placement, import_svg},
};

const OPEN_CUTS: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="cuts" fill="none" stroke="#000" stroke-width="0.5" d="M1 2 L11 2 M20 2 L20 7"/></svg>"##;
const ONE_STROKE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="edge" fill="none" stroke="#000" stroke-width="2" d="M5 5 L15 5"/></svg>"##;
const MIXED_ELEMENTS: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="mixed" fill="none" stroke="#000" d="M2 2 L8 2 L8 8 Z M12 2 L18 2"/><line id="seg" x1="22" y1="2" x2="22" y2="8" stroke="#000" stroke-width="0.4"/><polyline id="pl" points="26,2 30,2 30,6" fill="none" stroke="#000" stroke-width="0.4"/><polygon id="pg" points="33,2 38,2 38,6" fill="none" stroke="#000" stroke-width="0.4"/></svg>"##;
const FILL_AND_STROKE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><rect id="plate" x="2" y="2" width="10" height="6" fill="#fff"/><path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M20 4 L30 4"/><path id="both" fill="#fff" stroke="#000" stroke-width="0.4" d="M1 20 L10 20"/></svg>"##;
/// The same mixed source without the ambiguous fill+stroke element.
const COEXIST: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><rect id="plate" x="2" y="2" width="10" height="6" fill="#fff"/><path id="cut" fill="none" stroke="#000" stroke-width="0.4" d="M20 4 L30 4"/></svg>"##;
const EDITED_CUTS: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><path id="cuts" fill="none" stroke="#000" stroke-width="0.5" d="M1 2 L12 2 M20 2 L20 7"/></svg>"##;

fn options(mode: ImportMode) -> ImportOptions {
    ImportOptions {
        placement: Placement::default(),
        mode,
        ..Default::default()
    }
}

fn job(svg: &str, placement: Placement, mode: ImportMode) -> CamJob {
    CamJob {
        schema_version: 4,
        name: "knife-import".into(),
        source: Some(SourceSnapshot {
            filename: "art.svg".into(),
            svg: svg.into(),
        }),
        import: ImportOptions {
            placement,
            mode,
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

/// The page Y axis flips on import (30 mm page): page (x, y) → setup (x, 30-y).
fn setup_point(x: f64, y: f64) -> Point {
    Point::new(x, 30. - y)
}

#[test]
fn open_paths_stay_open_and_subpath_order_is_preserved() {
    let catalogue = ContourCatalogue::build(&job(
        OPEN_CUTS,
        Placement::default(),
        ImportMode::Centerline,
    ))
    .unwrap();
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
    // Centerline mode: exactly one chain rides the stroke's centerline; the
    // 2 mm stroke width is ignored, never offset into two parallel cuts.
    let geometry = import_svg(ONE_STROKE, &options(ImportMode::Centerline), None).unwrap();
    assert_eq!(geometry.chains.len(), 1, "one stroke is one centerline");
    let chain = &geometry.chains[0];
    assert!(!chain.closed);
    assert_eq!(chain.points.len(), 2);
    assert!((chain.points[0].x - 5.).abs() < 1e-9 && (chain.points[1].x - 15.).abs() < 1e-9);
    assert!((chain.points[0].y - chain.points[1].y).abs() < 1e-9);
    // The interpretation is visible as a diagnostic naming the element.
    assert!(
        geometry
            .diagnostics
            .iter()
            .any(|d| d.code == "SVG_STROKE_CENTERLINE" && d.source_id.as_deref() == Some("edge"))
    );
    // The catalogue agrees: exactly one chain of length 10.
    let catalogue = ContourCatalogue::build(&job(
        ONE_STROKE,
        Placement::default(),
        ImportMode::Centerline,
    ))
    .unwrap();
    assert_eq!(catalogue.open_chains.len(), 1);
    assert!((catalogue.open_chains[0].perimeter_mm - 10.).abs() < 1e-9);

    // The fill importer is not weakened: the same SVG stays a stroke error.
    let error = import_svg(ONE_STROKE, &options(ImportMode::Fill), None).unwrap_err();
    assert_eq!(error.code, "SVG_STROKE");
}

#[test]
fn closed_subpaths_line_and_polyline_elements_import_as_chains() {
    let catalogue = ContourCatalogue::build(&job(
        MIXED_ELEMENTS,
        Placement::default(),
        ImportMode::Centerline,
    ))
    .unwrap();
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

    // In fill mode the mixed source still rejects its first visible stroke,
    // and a bare line element keeps its legacy open-path rejection.
    assert_eq!(
        import_svg(MIXED_ELEMENTS, &options(ImportMode::Fill), None)
            .unwrap_err()
            .code,
        "SVG_STROKE"
    );
    let lines_only = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><line id="seg" x1="1" y1="1" x2="9" y2="9"/></svg>"##;
    assert_eq!(
        import_svg(lines_only, &options(ImportMode::Fill), None)
            .unwrap_err()
            .code,
        "SVG_OPEN_PATH"
    );
}

#[test]
fn fill_and_chain_sources_coexist_but_never_on_one_element() {
    // Centerline mode still imports filled elements as regions (contours),
    // so one source serves profile carving and knife cutting together.
    let catalogue =
        ContourCatalogue::build(&job(COEXIST, Placement::default(), ImportMode::Centerline))
            .unwrap();
    assert_eq!(catalogue.contours.len(), 1, "the filled rect");
    assert_eq!(catalogue.open_chains.len(), 1, "only the stroke-only path");
    assert_eq!(catalogue.open_chains[0].id, "cut-chain-0");

    // A single element with both visible fill and stroke is an explicit
    // error asking for separation, never a guess.
    let both_error =
        import_svg(FILL_AND_STROKE, &options(ImportMode::Centerline), None).unwrap_err();
    assert_eq!(both_error.code, "SVG_STROKE");
    assert!(
        both_error.message.contains("fill-none"),
        "the error says how to separate them: {both_error}"
    );
    assert_eq!(both_error.source_id.as_deref(), Some("both"));
}

#[test]
fn placement_moves_chains_and_anchors_resolve_on_source_geometry() {
    let identity = ContourCatalogue::build(&job(
        OPEN_CUTS,
        Placement::default(),
        ImportMode::Centerline,
    ))
    .unwrap();
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
        ImportMode::Centerline,
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
    let edited = ContourCatalogue::build(&job(
        EDITED_CUTS,
        Placement::default(),
        ImportMode::Centerline,
    ))
    .unwrap();
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
    let catalogue =
        ContourCatalogue::build(&job(svg, Placement::default(), ImportMode::Centerline)).unwrap();
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
