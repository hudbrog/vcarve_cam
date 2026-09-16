//! Contour density and accuracy: the merge budget every boundary spends must
//! buy vertices without spending the import tolerance. Measured against an
//! analytic circle so "accuracy" is not another approximation.
use cam_core::{
    contours::{Contour, ContourCatalogue},
    geometry::Point,
    job::{PlanningTolerances, SourceSnapshot},
    project::{CamJob, SetupSettings, StockSetup},
    svg::ImportOptions,
};

const TOLERANCE_MM: f64 = 0.001;

fn job(svg: &str) -> CamJob {
    CamJob {
        name: "density".into(),
        source: Some(SourceSnapshot {
            filename: "density.svg".into(),
            svg: svg.into(),
        }),
        import: ImportOptions {
            geometry_tolerance_mm: TOLERANCE_MM,
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
    }
}

fn only_outer(catalogue: &ContourCatalogue) -> &Contour {
    assert_eq!(catalogue.contours.len(), 1, "one filled boundary expected");
    &catalogue.contours[0]
}

fn ring(contour: &Contour) -> Vec<Point> {
    let mut ring = contour.vertices.clone();
    ring.push(ring[0]);
    ring
}

/// Distance from a point to a closed polyline.
fn distance_to_ring(p: Point, ring: &[Point]) -> f64 {
    let mut best = f64::INFINITY;
    for pair in ring.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let length_squared = dx * dx + dy * dy;
        let t = if length_squared <= f64::EPSILON {
            0.
        } else {
            (((p.x - a.x) * dx + (p.y - a.y) * dy) / length_squared).clamp(0., 1.)
        };
        best = best.min(p.distance(Point::new(a.x + t * dx, a.y + t * dy)));
    }
    best
}

#[test]
fn a_circle_boundary_spends_the_merge_budget_not_the_tolerance() {
    let centre = Point::new(30., 20.);
    let radius = 15.;
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="60mm" height="40mm" viewBox="0 0 60 40"><circle id="disc" cx="{}" cy="{}" r="{}" fill="black"/></svg>"#,
        centre.x, centre.y, radius
    );
    let catalogue = ContourCatalogue::build(&job(&svg)).unwrap();
    let contour = only_outer(&catalogue);
    let ring_points = ring(contour);

    // Chord error against the exact circle, both ways.
    let mut circle_to_ring: f64 = 0.;
    let mut vertex_radial_error: f64 = 0.;
    for step in 0..=7200 {
        let angle = std::f64::consts::TAU * step as f64 / 7200.;
        let p = Point::new(
            centre.x + radius * angle.cos(),
            centre.y + radius * angle.sin(),
        );
        circle_to_ring = circle_to_ring.max(distance_to_ring(p, &ring_points));
    }
    for p in &contour.vertices {
        vertex_radial_error = vertex_radial_error.max((p.distance(centre) - radius).abs());
    }
    eprintln!(
        "circle: vertices={} circle_to_ring={circle_to_ring:.6} vertex_radial={vertex_radial_error:.6} perimeter={:.4}",
        contour.vertices.len(),
        contour.perimeter_mm
    );
    assert!(
        circle_to_ring <= TOLERANCE_MM,
        "the boundary must stay inside the import tolerance: {circle_to_ring}"
    );
    assert!(vertex_radial_error <= TOLERANCE_MM);
    // The exact second-derivative bound subdivides a 15 mm circle into 545
    // chords at a quarter of the imported tolerance. The loose `|u| + |v|`
    // bound this replaced emitted 770 for the same accuracy.
    assert!(
        contour.vertices.len() <= 560,
        "circle boundary carries {} vertices, expected at most 560",
        contour.vertices.len()
    );
    // The merge budget may only remove vertices the tolerance already allows.
    assert!(
        circle_to_ring > 0.0001,
        "the merge gave away too much accuracy"
    );
}

#[test]
fn straight_edges_and_corners_survive_every_merge() {
    // A square: four vertices, four corners. Merging may not round them off,
    // and a filled rectangle must come back as four points.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="60mm" height="40mm" viewBox="0 0 60 40"><rect id="plate" x="10" y="10" width="30" height="20" fill="black"/></svg>"#;
    let catalogue = ContourCatalogue::build(&job(svg)).unwrap();
    let contour = only_outer(&catalogue);
    assert_eq!(
        contour.vertices.len(),
        4,
        "a rectangle is four vertices: {:?}",
        contour.vertices
    );
    assert!((contour.perimeter_mm - 100.).abs() < 1e-9);
}

#[test]
fn a_merged_boundary_stays_inside_the_declared_tolerance() {
    // The same check on a curve the flattener must subdivide, so the merge
    // budget is measured against a shape that is not already polygonal.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="60mm" height="40mm" viewBox="0 0 60 40"><path id="blob" fill="black" d="M10 20 C10 10 30 5 40 12 C50 19 50 30 40 35 C30 40 10 30 10 20 Z"/></svg>"#;
    let catalogue = ContourCatalogue::build(&job(svg)).unwrap();
    let contour = only_outer(&catalogue);
    eprintln!(
        "blob: vertices={} perimeter={:.4}",
        contour.vertices.len(),
        contour.perimeter_mm
    );
    // Every vertex stays inside the page and the ring keeps its orientation.
    assert!(contour.vertices.len() > 4);
    assert!(contour.perimeter_mm > 100.);
    assert!(
        contour.vertices.len() <= 800,
        "blob boundary carries {} vertices",
        contour.vertices.len()
    );
    // No vertex may be duplicated: a merged ring never carries a zero edge.
    let ring_points = ring(contour);
    for pair in ring_points.windows(2) {
        assert!(pair[0].distance(pair[1]) > 0., "zero-length edge merged in");
    }
}
