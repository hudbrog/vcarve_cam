use cam_core::{
    geometry::{BoundaryQuery, Grid, Point, PointLocation, Region, Segment},
    model::{Depth, Endmill, EndmillSpec, IncludedAngle, Length, VBit, VBitSpec},
    preview::{ModelInput, PreviewStatus, build_preview},
    target::{CenterSetStatus, FitStatus, ReachabilityOptions, ReachabilityStatus, Target},
};

fn depth(v: f64) -> Depth {
    Depth::new(v).unwrap()
}
fn length(v: f64) -> Length {
    Length::new(v).unwrap()
}
fn rectangle(x: f64, y: f64, w: f64, h: f64) -> Vec<Point> {
    vec![
        Point::new(x, y),
        Point::new(x + w, y),
        Point::new(x + w, y + h),
        Point::new(x, y + h),
    ]
}
fn target(rings: Vec<Vec<Point>>, cap: f64, angle: f64) -> Target {
    let extent = rings
        .iter()
        .flatten()
        .map(|p| p.x.abs().max(p.y.abs()))
        .fold(0.0, f64::max);
    Target::new(
        Region::from_rings(Grid::new(0.001, extent).unwrap(), &rings).unwrap(),
        depth(cap),
        IncludedAngle::new(angle).unwrap(),
    )
    .unwrap()
}
fn mill(diameter: f64) -> Endmill {
    Endmill::try_from(EndmillSpec {
        diameter_mm: diameter,
        cutting_length_mm: 10.0,
        plunge_capable: false,
    })
    .unwrap()
}
fn bit(tip: f64, angle: f64) -> VBit {
    VBit::try_from(VBitSpec {
        included_angle_deg: angle,
        tip_diameter_mm: tip,
        max_cutting_diameter_mm: 30.0,
        cutting_height_mm: 4.0,
    })
    .unwrap()
}
fn options(tolerance: f64) -> ReachabilityOptions {
    ReachabilityOptions {
        depth_tolerance_mm: tolerance,
        max_cells: 100_000,
    }
}
fn model(id: &str) -> ModelInput {
    let raw = match id {
        "wide_channel" => include_str!("../../../fixtures/m1/wide_channel.json"),
        "finite_tip_corner" => include_str!("../../../fixtures/m1/finite_tip_corner.json"),
        "endmill_exact_line" => include_str!("../../../fixtures/m1/endmill_exact_line.json"),
        _ => panic!("unknown test model"),
    };
    serde_json::from_str(raw).unwrap()
}
fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected} +/- {tolerance}"
    );
}

#[test]
fn units_reject_invalid_numbers_including_deserialization() {
    for value in [-1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(Length::new(value).is_err());
        assert!(Depth::new(value).is_err());
    }
    for value in [0.0, -1.0, 180.0, 181.0, f64::NAN, f64::INFINITY] {
        assert!(IncludedAngle::new(value).is_err());
    }
    assert!(serde_json::from_str::<Depth>("-0.5").is_err());
    assert!(serde_json::from_str::<IncludedAngle>("180").is_err());
    assert!(!depth(-0.0).machine_z_mm().is_sign_negative());
    assert_eq!(depth(2.5).machine_z_mm(), -2.5);
    near(
        IncludedAngle::new(60.0).unwrap().slope(),
        1.0 / 3f64.sqrt(),
        1e-15,
    );
}

#[test]
fn cutter_dimensions_and_usable_height_are_validated() {
    for diameter in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(
            Endmill::try_from(EndmillSpec {
                diameter_mm: diameter,
                cutting_length_mm: 10.0,
                plunge_capable: false
            })
            .is_err()
        );
    }
    for (tip, diameter, height) in [
        (-1.0, 6.0, 2.0),
        (6.0, 6.0, 2.0),
        (1.0, 6.0, 0.0),
        (1.0, 6.0, 3.0),
        (1.0, f64::NAN, 2.0),
    ] {
        assert!(
            VBit::try_from(VBitSpec {
                included_angle_deg: 90.0,
                tip_diameter_mm: tip,
                max_cutting_diameter_mm: diameter,
                cutting_height_mm: height
            })
            .is_err()
        );
    }
    let short = VBit::try_from(VBitSpec {
        included_angle_deg: 90.0,
        tip_diameter_mm: 1.0,
        max_cutting_diameter_mm: 6.0,
        cutting_height_mm: 2.5,
    })
    .unwrap();
    near(
        short.radius_at_height(length(2.5)).unwrap().mm(),
        3.0,
        1e-14,
    );
    assert_eq!(
        short.validate_depth(depth(2.5001)).unwrap_err().code,
        "VBIT_CUTTING_HEIGHT"
    );
    assert!(short.radius_at_height(length(2.6)).is_err());
    assert!(!mill(4.0).plunge_capable());
}

#[test]
fn cutter_removal_uses_the_physical_tip_plane_and_finite_footprint() {
    let tool = bit(1.0, 90.0);
    for (r, removed) in [
        (0.0, 2.0),
        (0.5, 2.0),
        (1.0, 1.5),
        (1.5, 1.0),
        (2.5, 0.0),
        (20.0, 0.0),
    ] {
        near(
            tool.removal_depth(depth(2.0), length(r)).unwrap().mm(),
            removed,
            1e-14,
        );
    }
    assert_eq!(
        mill(4.0)
            .removal_depth(depth(2.0), length(2.0))
            .unwrap()
            .mm(),
        2.0
    );
    assert_eq!(
        mill(4.0)
            .removal_depth(depth(2.0), length(2.0001))
            .unwrap()
            .mm(),
        0.0
    );
    assert!(tool.removal_depth(depth(5.0), length(0.0)).is_err());
    assert!(mill(4.0).removal_depth(depth(11.0), length(0.0)).is_err());
}

#[test]
fn independent_query_distinguishes_holes_boundaries_and_subgrid_points() {
    let t = target(
        vec![
            rectangle(0.0, 0.0, 20.0, 20.0),
            rectangle(8.0, 8.0, 4.0, 4.0),
        ],
        2.0,
        90.0,
    );
    for (p, location, distance) in [
        (Point::new(-0.00001, 5.0), PointLocation::Outside, -0.00001),
        (Point::new(0.0, 5.0), PointLocation::Boundary, 0.0),
        (Point::new(1.0, 5.0), PointLocation::Inside, 1.0),
        (Point::new(9.0, 10.0), PointLocation::Outside, -1.0),
        (Point::new(8.0, 10.0), PointLocation::Boundary, 0.0),
    ] {
        let sample = t.boundary().sample(p).unwrap();
        assert_eq!(sample.location, location);
        near(sample.signed_distance_mm, distance, 1e-12);
    }
    assert_eq!(t.nominal_depth(Point::new(10.0, 10.0)).unwrap().mm(), 0.0);
    assert!(t.boundary().sample(Point::new(f64::NAN, 0.0)).is_err());
    let nested = target(
        vec![
            rectangle(0.0, 0.0, 20.0, 20.0),
            rectangle(4.0, 4.0, 12.0, 12.0),
            rectangle(8.0, 8.0, 4.0, 4.0),
        ],
        1.0,
        90.0,
    );
    assert_eq!(
        nested
            .boundary()
            .sample(Point::new(10.0, 10.0))
            .unwrap()
            .location,
        PointLocation::Inside
    );
}

#[test]
fn whole_segment_query_detects_an_intervening_island() {
    let t = target(
        vec![
            rectangle(0.0, 0.0, 20.0, 20.0),
            rectangle(8.0, 8.0, 4.0, 4.0),
        ],
        2.0,
        90.0,
    );
    let crossing = Segment {
        start: Point::new(4.0, 10.0),
        end: Point::new(16.0, 10.0),
    };
    assert!(t.boundary().sample(crossing.start).unwrap().distance_mm >= 4.0);
    assert!(t.boundary().sample(crossing.end).unwrap().distance_mm >= 4.0);
    assert_eq!(t.boundary().segment_distance_mm(crossing).unwrap(), 0.0);
    let clear = Segment {
        start: Point::new(4.0, 4.0),
        end: Point::new(16.0, 4.0),
    };
    near(t.boundary().segment_distance_mm(clear).unwrap(), 4.0, 1e-12);
}

#[test]
fn straight_channel_depth_and_floor_dimensions_match_multiple_angles() {
    for degrees in [30.0, 60.0, 90.0, 120.0] {
        let t = target(vec![rectangle(0.0, 0.0, 100.0, 20.0)], 1.5, degrees);
        let slope = (degrees.to_radians() / 2.0).tan();
        near(
            t.nominal_depth(Point::new(50.0, 1.0)).unwrap().mm(),
            1.5f64.min(1.0 / slope),
            1e-12,
        );
        assert_eq!(t.nominal_depth(Point::new(50.0, 10.0)).unwrap().mm(), 1.5);
        let floor = t.section(depth(1.5)).unwrap();
        let r = 1.5 * slope;
        near(
            floor.area.area_mm2(),
            (100.0 - 2.0 * r) * (20.0 - 2.0 * r),
            0.02,
        );
        let narrow_width = 1.5 * slope;
        let narrow = target(vec![rectangle(0.0, 0.0, 100.0, narrow_width)], 1.5, degrees);
        near(
            narrow
                .nominal_depth(Point::new(50.0, narrow_width / 2.0))
                .unwrap()
                .mm(),
            0.75,
            0.0003,
        );
        assert_eq!(
            narrow.section(depth(1.5)).unwrap().status,
            CenterSetStatus::Empty
        );
    }
}

#[test]
fn tool_centers_include_one_radius_compensation_and_horizontal_allowance() {
    let t = target(vec![rectangle(0.0, 0.0, 30.0, 20.0)], 2.0, 90.0);
    let tool = mill(4.0);
    let centers = t.endmill_centers(&tool, depth(2.0), length(0.5)).unwrap();
    near(centers.required_clearance_mm, 4.5, 1e-14);
    near(centers.area.area_mm2(), 21.0 * 11.0, 1e-9);
    let v = t.vbit_centers(&bit(1.0, 90.0), depth(2.0)).unwrap();
    near(v.required_clearance_mm, 2.5, 1e-14);
    near(v.area.area_mm2(), 25.0 * 15.0, 1e-9);
    near(
        t.section(depth(2.0)).unwrap().area.area_mm2(),
        26.0 * 16.0,
        1e-9,
    );
    assert_eq!(
        t.endmill_pose_fit(&tool, Point::new(4.0, 10.0), depth(2.0), length(0.5))
            .unwrap()
            .status,
        FitStatus::Infeasible
    );
    assert_eq!(
        t.endmill_pose_fit(&tool, Point::new(4.5, 10.0), depth(2.0), length(0.5))
            .unwrap()
            .status,
        FitStatus::Contact
    );
    assert_eq!(
        t.endmill_pose_fit(&tool, Point::new(5.0, 10.0), depth(2.0), length(0.5))
            .unwrap()
            .status,
        FitStatus::Clearance
    );
}

#[test]
fn preserved_islands_expand_in_depth_and_center_sections() {
    let t = target(
        vec![
            rectangle(0.0, 0.0, 20.0, 20.0),
            rectangle(8.0, 8.0, 4.0, 4.0),
        ],
        2.0,
        90.0,
    );
    let section = t.section(depth(1.0)).unwrap();
    let query = BoundaryQuery::new(&section.area);
    assert_eq!(
        query.sample(Point::new(7.5, 10.0)).unwrap().location,
        PointLocation::Outside
    );
    assert_eq!(
        query.sample(Point::new(6.5, 10.0)).unwrap().location,
        PointLocation::Inside
    );
    let centers = t.vbit_centers(&bit(1.0, 90.0), depth(1.0)).unwrap();
    assert_eq!(
        BoundaryQuery::new(&centers.area)
            .sample(Point::new(6.75, 10.0))
            .unwrap()
            .location,
        PointLocation::Outside
    );
}

#[test]
fn exact_fit_center_lines_are_retained_with_zero_margin() {
    let t = target(vec![rectangle(0.0, 0.0, 20.0, 8.0)], 2.0, 90.0);
    let centers = t
        .endmill_centers(&mill(4.0), depth(2.0), length(0.0))
        .unwrap();
    assert_eq!(centers.status, CenterSetStatus::ContactOnly);
    assert!(centers.area.rings().is_empty());
    near(
        centers
            .contact_segments
            .iter()
            .map(|s| s.start.distance(s.end))
            .sum(),
        12.0,
        1e-10,
    );
    for p in [
        Point::new(4.0, 4.0),
        Point::new(10.0, 4.0),
        Point::new(16.0, 4.0),
    ] {
        assert!(
            centers
                .contact_segments
                .iter()
                .any(|s| s.distance(p) < 1e-10)
        );
        assert_eq!(
            t.endmill_pose_fit(&mill(4.0), p, depth(2.0), length(0.0))
                .unwrap()
                .status,
            FitStatus::Contact
        );
    }
    assert_eq!(
        t.endmill_centers(&mill(4.00001), depth(2.0), length(0.0))
            .unwrap()
            .status,
        CenterSetStatus::Empty
    );
}

#[test]
fn isolated_square_and_triangle_centers_survive_empty_polygon_offsets() {
    for (t, diameter, d, p) in [
        (
            target(vec![rectangle(0.0, 0.0, 8.0, 8.0)], 2.0, 90.0),
            4.0,
            2.0,
            Point::new(4.0, 4.0),
        ),
        (
            target(
                vec![vec![
                    Point::new(0.0, 0.0),
                    Point::new(6.0, 0.0),
                    Point::new(0.0, 8.0),
                ]],
                1.0,
                90.0,
            ),
            2.0,
            1.0,
            Point::new(2.0, 2.0),
        ),
    ] {
        let centers = t
            .endmill_centers(&mill(diameter), depth(d), length(0.0))
            .unwrap();
        assert_eq!(centers.status, CenterSetStatus::ContactOnly);
        assert!(centers.contact_segments.is_empty());
        assert_eq!(centers.contact_points.len(), 1);
        near(centers.contact_points[0].distance(p), 0.0, 1e-10);
    }
}

#[test]
fn disconnected_area_and_exact_fit_components_coexist() {
    let t = target(
        vec![
            rectangle(0.0, 0.0, 20.0, 8.0),
            rectangle(30.0, 0.0, 12.0, 12.0),
        ],
        2.0,
        90.0,
    );
    let centers = t
        .endmill_centers(&mill(4.0), depth(2.0), length(0.0))
        .unwrap();
    assert_eq!(centers.status, CenterSetStatus::AreaAndContacts);
    near(centers.area.area_mm2(), 16.0, 1e-9);
    assert!(!centers.contact_segments.is_empty());
}

#[test]
fn a_subgrid_center_region_is_unresolved_until_the_grid_can_represent_it() {
    let mut input = model("endmill_exact_line");
    input.endmill.diameter_mm = 3.99996;
    let coarse = input.validate().unwrap();
    let set = coarse
        .target
        .endmill_centers(&coarse.endmill, depth(2.0), length(0.0))
        .unwrap();
    assert_eq!(set.status, CenterSetStatus::Unresolved);
    assert!(
        set.diagnostics
            .iter()
            .any(|d| d.code == "CENTER_SET_UNRESOLVED")
    );
    input.ticks_per_mm = Some(1_000_000.0);
    let fine = input.validate().unwrap();
    let set = fine
        .target
        .endmill_centers(&fine.endmill, depth(2.0), length(0.0))
        .unwrap();
    assert_eq!(set.status, CenterSetStatus::Area);
    near(set.area.area_mm2(), 12.00004 * 0.00004, 1e-9);
}

#[test]
fn depth_cap_and_cutting_height_errors_do_not_clamp_the_nominal_target() {
    let t = target(vec![rectangle(0.0, 0.0, 20.0, 20.0)], 5.0, 90.0);
    assert_eq!(t.nominal_depth(Point::new(10.0, 10.0)).unwrap().mm(), 5.0);
    assert_eq!(
        t.vbit_centers(&bit(1.0, 90.0), depth(1.0))
            .unwrap_err()
            .code,
        "VBIT_CUTTING_HEIGHT"
    );
    assert!(t.section(depth(6.0)).is_err());
    let t = target(vec![rectangle(0.0, 0.0, 20.0, 20.0)], 2.0, 90.0);
    assert_eq!(
        t.vbit_centers(&bit(1.0, 60.0), depth(1.0))
            .unwrap_err()
            .code,
        "ANGLE_MISMATCH"
    );
    let mut input = model("wide_channel");
    input.vbit.cutting_height_mm = 1.0;
    assert_eq!(input.validate().err().unwrap().code, "VBIT_CUTTING_HEIGHT");
}

#[test]
fn reachable_removal_near_a_wall_uses_the_flank_not_the_center_depth() {
    let t = target(vec![rectangle(0.0, 0.0, 20.0, 20.0)], 2.0, 90.0);
    let tool = bit(1.0, 90.0);
    let p = Point::new(0.25, 10.0);
    assert_eq!(t.max_vbit_center_depth(&tool, p).unwrap().mm(), 0.0);
    let reachable = t.vbit_reachability(&tool, p, options(1e-6)).unwrap();
    assert_eq!(reachable.status, ReachabilityStatus::Resolved);
    near(reachable.reachable_depth_lower_mm, 0.25, 1e-6);
    near(reachable.reachable_depth_upper_mm, 0.25, 1e-12);
}

#[test]
fn finite_tip_corner_residual_bounds_enclose_the_analytic_reference() {
    let t = target(
        vec![vec![
            Point::new(0.0, 0.0),
            Point::new(16.0, 0.0),
            Point::new(0.0, 16.0),
        ]],
        2.0,
        90.0,
    );
    let tool = bit(1.0, 90.0);
    for s in [0.0, 0.05, 0.15, 0.5, 1.0] {
        let expected = (s - 0.5 * (1.0 - 1.0 / 2f64.sqrt())).max(0.0);
        let r = t
            .vbit_reachability(&tool, Point::new(s, s), options(1e-5))
            .unwrap();
        assert_eq!(r.status, ReachabilityStatus::Resolved);
        assert!(
            r.reachable_depth_lower_mm <= expected + 1e-12
                && r.reachable_depth_upper_mm >= expected - 1e-12,
            "{r:?} vs {expected}"
        );
        assert!(r.reachable_depth_upper_mm - r.reachable_depth_lower_mm <= 1e-5);
    }
}

#[test]
fn acute_wedge_limits_match_tip_radius_and_half_angle() {
    for phi in [15f64, 30.0, 45.0] {
        let b = 10.0 * phi.to_radians().tan();
        let t = target(
            vec![vec![
                Point::new(0.0, 0.0),
                Point::new(10.0, b),
                Point::new(10.0, -b),
            ]],
            2.0,
            90.0,
        );
        let snapped_b = t.region().rings_mm()[0]
            .iter()
            .map(|p| p.y.abs())
            .fold(0.0, f64::max);
        let sin_phi = snapped_b / 10f64.hypot(snapped_b);
        for tip in [0.0, 0.5, 1.0] {
            let expected = ((2.0 + tip / 2.0) * sin_phi - tip / 2.0).max(0.0);
            let r = t
                .vbit_reachability(&bit(tip, 90.0), Point::new(2.0, 0.0), options(1e-5))
                .unwrap();
            assert_eq!(r.status, ReachabilityStatus::Resolved);
            assert!(
                r.reachable_depth_lower_mm <= expected + 1e-12
                    && r.reachable_depth_upper_mm >= expected - 1e-12,
                "phi={phi} tip={tip}: {r:?} expected {expected}"
            );
        }
    }
}

#[test]
fn exhausting_reachability_budget_retains_bounds_and_cannot_claim_resolution() {
    let t = target(
        vec![vec![
            Point::new(0.0, 0.0),
            Point::new(16.0, 0.0),
            Point::new(0.0, 16.0),
        ]],
        2.0,
        90.0,
    );
    let tool = bit(1.0, 90.0);
    let p = Point::new(1.0, 1.0);
    let coarse = t
        .vbit_reachability(
            &tool,
            p,
            ReachabilityOptions {
                depth_tolerance_mm: 1e-6,
                max_cells: 1,
            },
        )
        .unwrap();
    assert_eq!(coarse.status, ReachabilityStatus::ResourceLimit);
    let fine = t.vbit_reachability(&tool, p, options(1e-6)).unwrap();
    assert!(fine.reachable_depth_lower_mm >= coarse.reachable_depth_lower_mm);
    assert!(fine.reachable_depth_upper_mm <= coarse.reachable_depth_upper_mm);
    assert_eq!(fine.status, ReachabilityStatus::Resolved);
}

#[test]
fn target_and_reachability_are_translation_rotation_and_scale_invariant() {
    let original = [
        Point::new(0.0, 0.0),
        Point::new(16.0, 0.0),
        Point::new(0.0, 16.0),
    ];
    for factor in [0.1, 1.0, 10.0] {
        let transform = |p: Point| Point::new(123.0 - factor * p.y, -79.0 + factor * p.x);
        let t = target(
            vec![original.iter().copied().map(transform).collect()],
            2.0 * factor,
            90.0,
        );
        let tool = VBit::try_from(VBitSpec {
            included_angle_deg: 90.0,
            tip_diameter_mm: factor,
            max_cutting_diameter_mm: 12.0 * factor,
            cutting_height_mm: 4.0 * factor,
        })
        .unwrap();
        let result = t
            .vbit_reachability(
                &tool,
                transform(Point::new(1.0, 1.0)),
                options(1e-5 * factor),
            )
            .unwrap();
        let expected = factor * (1.0 - 0.5 * (1.0 - 1.0 / 2f64.sqrt()));
        assert_eq!(result.status, ReachabilityStatus::Resolved);
        near(result.reachable_depth_lower_mm, expected, 1e-5 * factor);
    }
}

#[test]
fn parameter_edits_rebuild_the_target_and_keep_nominal_and_tool_limits_separate() {
    let input = model("finite_tip_corner");
    let finite = build_preview(input.clone()).unwrap();
    let mut changed = input.clone();
    changed.vbit.tip_diameter_mm = 0.0;
    let ideal = build_preview(changed).unwrap();
    assert_eq!(finite.status, PreviewStatus::Complete);
    assert_eq!(ideal.status, PreviewStatus::Complete);
    for (a, b) in finite.cross_sections[0]
        .samples
        .iter()
        .zip(&ideal.cross_sections[0].samples)
    {
        near(a.nominal_depth_mm, b.nominal_depth_mm, 1e-12);
    }
    assert!(
        finite.cross_sections[0]
            .samples
            .iter()
            .any(|s| s.vbit_removal.unavoidable_residual_lower_mm > 0.1)
    );
    assert!(
        ideal.cross_sections[0]
            .samples
            .iter()
            .all(|s| s.vbit_removal.unavoidable_residual_lower_mm == 0.0)
    );
    let mut limited = input;
    limited.max_reachability_cells = 1;
    let preview = build_preview(limited).unwrap();
    assert_eq!(preview.status, PreviewStatus::Inconclusive);
    assert!(
        preview
            .diagnostics
            .iter()
            .any(|d| d.code == "REACHABILITY_INCONCLUSIVE")
    );
}

#[test]
fn malformed_preview_settings_are_rejected_before_evaluation() {
    let base = model("wide_channel");
    let mut bad = base.clone();
    bad.schema_version = 99;
    assert!(bad.validate().is_err());
    let mut bad = base.clone();
    bad.wall_allowance_mm = -0.1;
    assert!(bad.validate().is_err());
    let mut bad = base.clone();
    bad.section_depths_mm.push(3.0);
    assert!(bad.validate().is_err());
    let mut bad = base.clone();
    bad.cross_sections[0].samples = 0;
    assert!(bad.validate().is_err());
    let mut bad = base.clone();
    bad.cross_sections[0].end = bad.cross_sections[0].start;
    assert!(bad.validate().is_err());
    let mut bad = base.clone();
    bad.preview_depth_tolerance_mm = 0.0;
    assert!(bad.validate().is_err());
    let mut bad = base.clone();
    bad.max_reachability_cells = 0;
    assert!(bad.validate().is_err());
    let mut bad = base;
    bad.id = "../bad".into();
    assert!(bad.validate().is_err());
}
