//! D2 drill-point geometry: the importer captures analytic circle/ellipse
//! centers before flattening discards them, the contour catalogue publishes
//! marker points (exact for circle elements, area-centroid equivalents for
//! other closed contours, deduplicated per drawn circle), and the combined
//! catalogue exposes them as owner-qualified point entries.
use cam_core::{
    contours::ContourCatalogue,
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    project::{CamJob, RectXY, SetupSettings, StockSetup, WorkZero, v5},
    svg::{ImportOptions, Placement},
};

/// A filled circle, a stroked circle, an ellipse, and a square dot drawn as a
/// filled path — the four marker authoring styles drilling must read.
const MARKERS: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><circle id="a" cx="10" cy="15" r="2.5" fill="#000"/><circle id="b" cx="30" cy="10" r="4" fill="none" stroke="#000" stroke-width="0.4"/><ellipse id="e" cx="20" cy="22" rx="3" ry="1.5" fill="#000"/><path id="dot" fill="#000" d="M4 24 h4 v4 h-4 Z"/></svg>"##;

fn job(svg: &str, placement: Placement) -> CamJob {
    CamJob {
        name: "drill-points".into(),
        source: Some(SourceSnapshot {
            filename: "markers.svg".into(),
            svg: svg.into(),
        }),
        import: ImportOptions {
            placement,
            ..Default::default()
        },
        setup: SetupSettings::default(),
        tools: vec![],
        operations: vec![],
        tolerances: PlanningTolerances::default(),
    }
}

fn catalogue(svg: &str) -> ContourCatalogue {
    ContourCatalogue::build(&job(svg, Placement::default())).unwrap()
}

#[test]
fn analytic_circle_and_ellipse_centers_survive_as_exact_points() {
    let catalogue = catalogue(MARKERS);
    let a = catalogue.point("a-point").unwrap();
    // The page Y axis flips on import (30 mm page): SVG (10, 15) → (10, 15).
    assert_eq!(a.center, Point::new(10., 15.));
    assert_eq!(a.diameter_mm, 5.);
    assert!(a.exact);
    let b = catalogue.point("b-point").unwrap();
    assert_eq!(b.center, Point::new(30., 20.));
    assert_eq!(b.diameter_mm, 8.);
    // An ellipse marker reports its largest width; the center stays exact.
    let e = catalogue.point("e-point").unwrap();
    assert_eq!(e.center, Point::new(20., 8.));
    assert_eq!(e.diameter_mm, 6.);
    assert!(e.exact);
}

#[test]
fn every_drawn_circle_publishes_exactly_one_point() {
    let catalogue = catalogue(MARKERS);
    // `a` is filled (analytic point + filled component), `b` is
    // stroked-unfilled (analytic point + `-outline` contour): neither may
    // also derive a centroid point. The path dot derives one.
    let ids: Vec<&str> = catalogue.points.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["a-point", "b-point", "e-point", "dot-0-outer-center"]
    );
}

#[test]
fn drawn_dots_derive_area_centroid_points() {
    let catalogue = catalogue(MARKERS);
    let dot = catalogue.point("dot-0-outer-center").unwrap();
    // The 4×4 mm square maps to page (4..8, 2..6): centroid (6, 4).
    assert_eq!(dot.center, Point::new(6., 4.));
    let expected = 2. * (16. / std::f64::consts::PI).sqrt();
    assert!((dot.diameter_mm - expected).abs() < 1e-9);
    assert!(!dot.exact);
}

#[test]
fn placement_moves_points_like_every_other_reading() {
    let placed = ContourCatalogue::build(&job(
        MARKERS,
        Placement {
            origin_mm: Point::new(1., 2.),
            scale: 2.,
            rotation_deg: 90.,
        },
    ))
    .unwrap();
    let a = placed.point("a-point").unwrap();
    // setup = 2·R90°(page − origin): (10,15) − (1,2) = (9,13) → (−13,9) → (−26,18).
    assert_eq!(a.center, Point::new(-26., 18.));
    assert_eq!(a.diameter_mm, 10.);
    let dot = placed.point("dot-0-outer-center").unwrap();
    // (6,4) − (1,2) = (5,2) → (−2,5) → (−4,10); centroids follow the same map.
    assert_eq!(dot.center, Point::new(-4., 10.));
    let original = catalogue(MARKERS);
    assert!((dot.diameter_mm - 2. * original.point(&dot.id).unwrap().diameter_mm).abs() < 1e-9);
}

#[test]
fn a_transformed_circle_group_keeps_its_exact_center() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40mm" height="30mm" viewBox="0 0 40 30"><g transform="translate(10,5)"><circle id="c" cx="0" cy="0" r="1" fill="#000"/></g></svg>"##;
    let catalogue = catalogue(svg);
    let c = catalogue.point("c-point").unwrap();
    assert_eq!(c.center, Point::new(10., 25.));
    assert_eq!(c.diameter_mm, 2.);
}

#[test]
fn the_combined_catalogue_publishes_owner_qualified_point_entries() {
    let job = v5::CamJobV5 {
        schema_version: v5::CAM_JOB_V5_SCHEMA_VERSION,
        name: "drill".into(),
        setup: SetupSettings {
            stock: StockSetup {
                thickness_mm: Some(8.),
                xy: Some(RectXY {
                    min_x_mm: 0.,
                    min_y_mm: 0.,
                    width_mm: 40.,
                    length_mm: 30.,
                }),
            },
            work_zero: WorkZero::default(),
            clearance_above_stock_mm: Some(5.),
            start_xy_mm: Some(Point::new(0., 0.)),
        },
        artwork: vec![v5::ArtworkItem {
            id: v5::ArtworkItemId("plate".into()),
            name: "Plate".into(),
            content: v5::ArtworkContent::Svg(SourceSnapshot {
                filename: "markers.svg".into(),
                svg: MARKERS.into(),
            }),
            import_settings: v5::SvgInterpretation {
                geometry_tolerance_mm: 0.001,
                ticks_per_mm: None,
            },
            placement: Placement {
                origin_mm: Point::new(0., 0.),
                scale: 1.,
                rotation_deg: 0.,
            },
        }],
        tools: vec![],
        operations: vec![],
        tolerances: PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
            arc_fit_tolerance_mm: None,
        },
        machine_configuration: None,
    };
    let combined = v5::artwork::inspect_artwork(&job).unwrap();
    let item = combined.item(&v5::ArtworkItemId("plate".into())).unwrap();
    let wires: Vec<&str> = item
        .point_entries
        .iter()
        .map(|p| p.wire_id.as_str())
        .collect();
    assert_eq!(
        wires,
        vec![
            "plate:point:a-point",
            "plate:point:b-point",
            "plate:point:e-point",
            "plate:point:dot-0-outer-center",
        ]
    );
    let a = combined.point_entry("plate:point:a-point").unwrap();
    assert_eq!(a.center, Point::new(10., 15.));
    assert_eq!(a.diameter_mm, 5.);
    assert!(a.exact);
    assert_eq!(a.reference.kind, v5::GeometryRefKind::Point);
    assert_eq!(a.reference.local_geometry_id, "a-point");
    // Wire IDs stay reversible to their picks.
    let pick = v5::artwork::parse_wire_id("plate:point:a-point").unwrap();
    assert_eq!(pick.kind, v5::GeometryRefKind::Point);
    assert_eq!(pick.local_geometry_id, "a-point");
    // The assembled planning catalogue qualifies point IDs the same way.
    let assembled = v5::resolve::assembled_catalogue(&combined).unwrap();
    assert!(assembled.point("plate:point:a-point").is_some());
    assert!(assembled.point("a-point").is_none());
}
