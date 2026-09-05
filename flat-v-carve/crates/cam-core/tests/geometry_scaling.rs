use cam_core::{
    geometry::{BoundaryQuery, Grid, Point, Region, WindingRule},
    svg::{ImportOptions, import_svg},
};

#[test]
fn local_grid_contraction_preserves_a_contour_and_reports_its_bound() {
    let grid = Grid::new(0.005, 30.).unwrap();
    let raw = vec![
        Point::new(0., 0.),
        Point::new(20., 0.),
        Point::new(20., 10.),
        Point::new(0., 10.),
        Point::new(0., 0.000001),
    ];
    let region = Region::from_filled_paths(grid, &[raw], WindingRule::Nonzero).unwrap();
    assert_eq!(region.component_count(), 1);
    assert_eq!(region.hole_count(), 0);
    assert_eq!(region.area_mm2(), 200.);
    assert!(
        region
            .diagnostics()
            .iter()
            .any(|d| d.code == "SNAPPED_VERTEX_COALESCED")
    );
}

#[test]
fn contraction_cannot_erase_a_hole_or_an_attached_micro_loop() {
    let grid = Grid::new(0.005, 30.).unwrap();
    let outer = vec![
        Point::new(0., 0.),
        Point::new(20., 0.),
        Point::new(20., 10.),
        Point::new(0., 10.),
    ];
    let hole = vec![
        Point::new(5., 5.),
        Point::new(5.000001, 5.),
        Point::new(5.000001, 6.),
        Point::new(5., 6.),
    ];
    assert_eq!(
        Region::from_filled_paths(grid, &[outer, hole], WindingRule::Evenodd)
            .unwrap_err()
            .code,
        "QUANTIZATION_COLLAPSE"
    );
    let attached = vec![
        Point::new(0., 0.),
        Point::new(20., 0.),
        Point::new(20., 10.),
        Point::new(0., 10.),
        Point::new(0., 0.000002),
        Point::new(-0.000002, 0.000004),
        Point::new(-0.000002, 0.000001),
        Point::new(0., 0.000002),
    ];
    assert_eq!(
        Region::from_filled_paths(grid, &[attached], WindingRule::Nonzero)
            .unwrap_err()
            .code,
        "QUANTIZATION_TOPOLOGY"
    );
}

#[test]
fn thousands_of_components_keep_identity_holes_and_independent_distances() {
    let mut svg = String::from("<svg width='200mm' height='200mm' viewBox='0 0 200 200'>");
    for y in 0..50 {
        for x in 0..50 {
            svg.push_str(&format!(
                "<rect id='r{x}-{y}' x='{}' y='{}' width='2' height='2'/>",
                x * 4,
                y * 4
            ));
        }
    }
    svg.push_str("</svg>");
    let geometry = import_svg(&svg, &ImportOptions::default(), None).unwrap();
    assert_eq!(geometry.sources.len(), 2500);
    assert_eq!(geometry.components.len(), 2500);
    assert_eq!(geometry.selected.area_mm2(), 10000.);
    assert_eq!(geometry.selected.hole_count(), 0);
    for c in &geometry.components {
        assert_eq!(c.selected_region_ids.len(), 1);
    }
    let query = BoundaryQuery::new(&geometry.selected);
    for i in 0..128 {
        let p = Point::new((i * 73 % 211) as f64 + 0.123, (i * 47 % 211) as f64 + 0.357);
        let brute = geometry
            .selected
            .segments()
            .iter()
            .map(|s| s.distance(p))
            .fold(f64::INFINITY, f64::min);
        assert!((query.sample(p).unwrap().distance_mm - brute).abs() < 1e-12);
    }
}

#[test]
fn a_single_large_boundary_is_not_limited_to_the_old_fixture_budget() {
    let points: Vec<_> = (0..12000)
        .map(|i| {
            let t = i as f64 * std::f64::consts::TAU / 12000.;
            Point::new(50. + 40. * t.cos(), 50. + 40. * t.sin())
        })
        .collect();
    let region = Region::from_filled_paths(
        Grid::new(0.001, 100.).unwrap(),
        &[points],
        WindingRule::Nonzero,
    )
    .unwrap();
    assert_eq!(region.component_count(), 1);
    assert!((region.area_mm2() - std::f64::consts::PI * 1600.).abs() < 0.05);
    assert!(
        (BoundaryQuery::new(&region)
            .sample(Point::new(50., 50.))
            .unwrap()
            .distance_mm
            - 40.)
            .abs()
            < 0.001
    );
}
