use cam_core::{
    geometry::{BooleanOp, Grid, Point, Region, SiteKind, VoronoiDiagram},
    spike::{Fixture, Operation, run_fixture},
};

fn fixtures() -> Vec<Fixture> {
    serde_json::from_str(include_str!("../../../fixtures/m0.json")).unwrap()
}
fn fixture(id: &str) -> Fixture {
    fixtures().into_iter().find(|f| f.id == id).unwrap()
}
fn rectangle(x: f64, y: f64, w: f64, h: f64) -> Vec<Point> {
    vec![
        Point::new(x, y),
        Point::new(x + w, y),
        Point::new(x + w, y + h),
        Point::new(x, y + h),
    ]
}

#[test]
fn capability_fixtures_match_independent_references() {
    let results: Vec<_> = fixtures().into_iter().map(run_fixture).collect();
    let failures: Vec<_> = results.iter().filter(|r| !r.passed).collect();
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn snapping_bound_and_range_are_checked_before_casting() {
    let grid = Grid::new(0.001, 100_000.0).unwrap();
    assert_eq!(grid.scale(), 10000.0);
    assert!(grid.snap_bound_mm() <= grid.tolerance_mm() / 8.0);
    for x in [0.00004999, -0.00004999, 99999.999999, -99999.999999] {
        let p = Point::new(x, -x);
        assert!(p.distance(grid.point(grid.quantize(p).unwrap())) <= grid.snap_bound_mm());
    }
    assert_eq!(
        grid.quantize(Point::new(100_000.000_1, 0.0))
            .unwrap_err()
            .code,
        "COORDINATE_RANGE"
    );
    for n in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(grid.quantize(Point::new(n, 0.0)).is_err());
        assert!(Grid::new(n, 10.0).is_err());
    }
    assert!(Grid::new(0.0, 10.0).is_err());
    assert!(Grid::new(1e-10, 10.0).is_err());
    assert!(Grid::new(0.001, -1.0).is_err());
    assert!(Grid::with_scale(0.001, 10.0, 100.0).is_err());
}

#[test]
fn quantization_cannot_reverse_a_thin_ring() {
    let grid = Grid::new(0.001, 3.0).unwrap();
    let ring = vec![
        Point::new(0.0, 0.0),
        Point::new(1.0, 1.000049),
        Point::new(2.0, 2.000051),
    ];
    assert_eq!(
        Region::from_rings(grid, &[ring]).unwrap_err().code,
        "QUANTIZATION_ORIENTATION"
    );
}

#[test]
fn polygon_hierarchy_and_winding_survive_boolean_operations() {
    let grid = Grid::new(0.001, 20.0).unwrap();
    let outer = Region::from_rings(grid, &[rectangle(0.0, 0.0, 20.0, 20.0)]).unwrap();
    let inner = Region::from_rings(grid, &[rectangle(4.0, 4.0, 12.0, 12.0)]).unwrap();
    let nested = Region::from_rings(grid, &[rectangle(8.0, 8.0, 4.0, 4.0)]).unwrap();
    let region = outer
        .boolean(BooleanOp::Difference, &inner)
        .unwrap()
        .boolean(BooleanOp::Union, &nested)
        .unwrap();
    assert_eq!(region.area_mm2(), 272.0);
    assert_eq!((region.component_count(), region.hole_count()), (2, 1));
    assert_eq!(region.rings()[1].parent(), Some(0));
    assert_eq!(region.rings()[2].parent(), Some(1));
    assert_eq!(
        region
            .rings()
            .iter()
            .map(|r| r.is_hole())
            .collect::<Vec<_>>(),
        vec![false, true, false]
    );
}

#[test]
fn empty_regions_are_valid_boolean_operands() {
    let grid = Grid::new(0.001, 10.0).unwrap();
    let empty = Region::from_rings(grid, &[]).unwrap();
    let square = Region::from_rings(grid, &[rectangle(0.0, 0.0, 10.0, 10.0)]).unwrap();
    assert_eq!(
        empty.boolean(BooleanOp::Union, &square).unwrap().area_mm2(),
        100.0
    );
    assert_eq!(
        square
            .boolean(BooleanOp::Intersection, &empty)
            .unwrap()
            .area_mm2(),
        0.0
    );
    assert!(VoronoiDiagram::build(&empty).unwrap().edges.is_empty());
}

#[test]
fn offset_and_boolean_parameters_cannot_bypass_precision_policy() {
    let grid = Grid::new(0.001, 20.0).unwrap();
    let region = Region::from_rings(grid, &[rectangle(0.0, 0.0, 10.0, 10.0)]).unwrap();
    for n in [-1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(region.erode(n).unwrap_err().code, "INVALID_OFFSET");
    }
    assert_eq!(region.erode(0.0).unwrap().area_mm2(), 100.0);
    assert_eq!(region.erode(100_001.0).unwrap_err().code, "OFFSET_RANGE");
    let other = Region::from_rings(Grid::new(0.01, 20.0).unwrap(), &[]).unwrap();
    assert_eq!(
        region.boolean(BooleanOp::Union, &other).unwrap_err().code,
        "GRID_MISMATCH"
    );
}

#[test]
fn output_resolution_can_change_without_silently_erasing_features() {
    let raw = fixture("square_island").rings;
    let low = Region::from_rings(Grid::with_scale(0.001, 20.0, 10000.0).unwrap(), &raw)
        .unwrap()
        .erode(1.0)
        .unwrap();
    let high = Region::from_rings(Grid::with_scale(0.001, 20.0, 100000.0).unwrap(), &raw)
        .unwrap()
        .erode(1.0)
        .unwrap();
    assert_eq!(
        (low.component_count(), low.hole_count()),
        (high.component_count(), high.hole_count())
    );
    assert!((low.area_mm2() - high.area_mm2()).abs() < 0.01);
    let tiny = fixture("tiny_segment_collapses").rings;
    assert!(Region::from_rings(low.grid(), &tiny).is_err());
    assert!(Region::from_rings(high.grid(), &tiny).is_ok());
}

#[test]
fn offsets_and_voronoi_are_translation_rotation_and_scale_invariant_within_budget() {
    for id in [
        "rectangle_inset",
        "square_island",
        "concave_inset",
        "neck_splits",
        "voronoi_concave",
        "voronoi_island",
    ] {
        for (scale, angle, dx, dy) in [
            (1.0, 0.0, 123.0, -79.0),
            (1.0, std::f64::consts::FRAC_PI_2, 0.0, 0.0),
            (1.0, 0.371, 0.0, 0.0),
            (0.1, 0.0, 0.0, 0.0),
            (10.0, 0.0, 0.0, 0.0),
        ] {
            let mut f = fixture(id);
            let (sin, cos) = angle.sin_cos();
            for p in f.rings.iter_mut().flatten() {
                *p = Point::new(
                    scale * (p.x * cos - p.y * sin) + dx,
                    scale * (p.x * sin + p.y * cos) + dy,
                );
            }
            f.tolerance_mm *= scale;
            if let Operation::Erode { radius_mm } = &mut f.operation {
                *radius_mm *= scale;
            }
            if let Some(area) = &mut f.expected.area_mm2 {
                *area *= scale * scale;
            }
            f.expected.area_tolerance_mm2 =
                Some(f.expected.area_tolerance_mm2.unwrap_or(0.01) * scale * scale);
            let result = run_fixture(f);
            assert!(
                result.passed,
                "{id} scale={scale} angle={angle}: {:#?} {:#?}",
                result.diagnostics, result.measurements
            );
        }
    }
}

#[test]
fn rectangle_medial_vertices_have_analytic_locations_and_clearance() {
    let f = fixture("voronoi_rectangle");
    let region = Region::from_rings(Grid::new(f.tolerance_mm, 12.0).unwrap(), &f.rings).unwrap();
    let diagram = VoronoiDiagram::build(&region).unwrap();
    for expected in [Point::new(4.0, 4.0), Point::new(8.0, 4.0)] {
        assert!(
            diagram
                .edges
                .iter()
                .flat_map(|e| [e.start, e.end])
                .flatten()
                .any(|v| v.distance(expected) < 1e-10)
        );
        assert!((region.boundary_distance_mm(expected) - 4.0).abs() < 1e-10);
    }
    for i in 0..diagram.source_segments.len() {
        assert!(
            diagram
                .cells
                .iter()
                .any(|s| s.segment == i && s.kind == SiteKind::Segment)
        );
    }
    assert_eq!(
        diagram.source_boundaries,
        vec![[0, 0], [0, 1], [0, 2], [0, 3]]
    );
}

#[test]
fn curve_bound_refines_and_resource_exhaustion_is_explicit() {
    let result = run_fixture(fixture("voronoi_concave"));
    let diagram = result.voronoi.unwrap();
    let curve = diagram
        .edges
        .iter()
        .filter_map(|e| e.curve.as_ref())
        .find(|c| c.is_curved())
        .unwrap();
    let coarse = curve.linearize(0.01, 65536).unwrap();
    let fine = curve.linearize(0.00001, 65536).unwrap();
    assert!(fine.points.len() > coarse.points.len());
    assert!(fine.total_bound_mm <= 0.00001);
    assert_eq!(curve.linearize(1e-8, 1).unwrap_err().code, "CURVE_LIMIT");
    for tolerance in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(curve.linearize(tolerance, 100).is_err());
    }
    for t in [-0.1, 1.1, f64::NAN] {
        assert!(curve.evaluate(t).is_err());
    }
}

#[test]
fn incorrect_reference_fails_while_retaining_inspectable_geometry() {
    let mut f = fixture("rectangle_inset");
    f.expected.area_mm2 = Some(99.0);
    let result = run_fixture(f);
    assert!(!result.passed);
    assert!(result.output_region.is_some());
    assert!(result.measurements.iter().any(|m| !m.passed));
}

#[test]
fn repeated_runs_produce_identical_application_artifacts() {
    let f = fixture("voronoi_island");
    assert_eq!(
        serde_json::to_string(&run_fixture(f.clone())).unwrap(),
        serde_json::to_string(&run_fixture(f)).unwrap()
    );
}
