use cam_core::{geometry::Point, sketch::*};
use cam_sketch_solver::{MAX_ITERATIONS, SolveStatus, solve, solve_with_cancel};
fn e(id: &str, geometry: SketchGeometry) -> Entity {
    Entity {
        id: id.into(),
        label: None,
        construction: false,
        geometry,
    }
}
fn line(id: &str, a: (f64, f64), b: (f64, f64)) -> Entity {
    e(
        id,
        SketchGeometry::Line {
            start: Point::new(a.0, a.1),
            end: Point::new(b.0, b.1),
        },
    )
}
fn point(id: &str, x: f64, y: f64) -> Entity {
    e(
        id,
        SketchGeometry::Point {
            position: Point::new(x, y),
        },
    )
}
fn p(id: &str, point: PointRole) -> PointRef {
    PointRef {
        entity_id: id.into(),
        point,
    }
}
fn pos(id: &str) -> PointRef {
    p(id, PointRole::Position)
}
fn add(d: &mut SketchDocument, definition: ConstraintKind) {
    d.constraints.push(Constraint {
        id: format!("c{}", d.constraints.len()),
        reference: false,
        definition,
    });
}
fn fixed(d: &mut SketchDocument, id: &str) {
    add(
        d,
        ConstraintKind::Fixed {
            entity: id.into(),
            target: d.entity(id).unwrap().geometry.clone(),
        },
    );
}
fn accepted(d: &SketchDocument) -> SketchDocument {
    let r = solve(d).unwrap();
    assert!(
        r.accepted(),
        "{}: {:?}",
        r.solver_status,
        r.validation.issues
    );
    assert!(r.iterations <= MAX_ITERATIONS);
    assert!(r.candidate.validate_solution().unwrap().valid());
    r.candidate
}

#[test]
fn wire_connectivity_fixed_point_and_driving_resize_are_solved_together() {
    let mut d = SketchDocument {
        entities: vec![
            line("a", (0., 0.), (20., 0.)),
            line("b", (20., 0.), (20., 10.)),
            line("c", (20., 10.), (0., 10.)),
            line("d", (0., 10.), (0., 0.)),
        ],
        ..Default::default()
    };
    d.wires.push(Wire {
        id: "outline".into(),
        label: None,
        closed: true,
        edges: ["a", "b", "c", "d"]
            .into_iter()
            .map(|id| WireEdge {
                entity_id: id.into(),
                reversed: false,
            })
            .collect(),
    });
    add(
        &mut d,
        ConstraintKind::FixedPoint {
            point: p("a", PointRole::Start),
            position: Point::new(0., 0.),
        },
    );
    for id in ["a", "c"] {
        add(&mut d, ConstraintKind::Horizontal { line: id.into() });
    }
    for id in ["b", "d"] {
        add(&mut d, ConstraintKind::Vertical { line: id.into() });
    }
    add(
        &mut d,
        ConstraintKind::Length {
            line: "a".into(),
            value_mm: 30.,
        },
    );
    add(
        &mut d,
        ConstraintKind::Length {
            line: "b".into(),
            value_mm: 15.,
        },
    );
    let report = solve(&d).unwrap();
    assert!(report.accepted(), "{:?}", report.validation);
    assert_eq!(report.degrees_of_freedom, Some(0));
    let d = report.candidate;
    for (a, b) in d.wire_joints().unwrap() {
        assert!(d.point(&a).unwrap().distance(d.point(&b).unwrap()) < 1e-6);
    }
    assert!(
        d.point(&p("b", PointRole::End))
            .unwrap()
            .distance(Point::new(30., 15.))
            < 1e-6
    );
    let drag = cam_sketch_solver::solve_drag_with_cancel(
        &d,
        &[cam_sketch_solver::DragPoint {
            point: p("b", PointRole::End),
            target: Point::new(40., 40.),
        }],
        || false,
    )
    .unwrap();
    assert!(drag.accepted(), "{:?}", drag.validation);
    assert_eq!(drag.degrees_of_freedom, Some(0));
    assert!(
        drag.candidate
            .point(&p("b", PointRole::End))
            .unwrap()
            .distance(Point::new(30., 15.))
            < 1e-6
    );
}

#[test]
fn dragging_obeys_hard_dimensions_and_reference_dimensions_follow() {
    let mut d = SketchDocument {
        entities: vec![line("a", (0., 0.), (10., 0.))],
        ..Default::default()
    };
    add(
        &mut d,
        ConstraintKind::FixedPoint {
            point: p("a", PointRole::Start),
            position: Point::new(0., 0.),
        },
    );
    add(
        &mut d,
        ConstraintKind::Length {
            line: "a".into(),
            value_mm: 10.,
        },
    );
    d.constraints.push(Constraint {
        id: "x".into(),
        reference: true,
        definition: ConstraintKind::HorizontalDistance {
            a: p("a", PointRole::End),
            b: p("a", PointRole::Start),
            value_mm: 10.,
        },
    });
    let r = cam_sketch_solver::solve_drag_with_cancel(
        &d,
        &[cam_sketch_solver::DragPoint {
            point: p("a", PointRole::End),
            target: Point::new(8., 6.),
        }],
        || false,
    )
    .unwrap();
    assert!(r.accepted(), "{:?}", r.validation);
    assert_eq!(r.degrees_of_freedom, Some(1));
    assert!(
        r.candidate
            .point(&p("a", PointRole::End))
            .unwrap()
            .distance(Point::new(8., 6.))
            < 1e-3
    );
    assert!(
        (r.validation
            .constraints
            .iter()
            .find(|c| c.constraint_id == "x")
            .unwrap()
            .measured
            .unwrap()
            - 8.)
            .abs()
            < 1e-3
    );
    let r = cam_sketch_solver::solve_drag_with_cancel(
        &d,
        &[cam_sketch_solver::DragPoint {
            point: p("a", PointRole::End),
            target: Point::new(20., 0.),
        }],
        || false,
    )
    .unwrap();
    assert!(r.accepted(), "{:?}", r.validation);
    assert!(r.candidate.require_valid_solution().is_ok());
}

#[test]
fn free_drag_moves_and_invalid_targets_do_not_mutate_input() {
    let d = SketchDocument {
        entities: vec![e(
            "circle",
            SketchGeometry::Circle {
                center: Point::new(30., 30.),
                radius_mm: 10.,
            },
        )],
        ..Default::default()
    };
    let target = cam_sketch_solver::DragPoint {
        point: p("circle", PointRole::Center),
        target: Point::new(35., 30.),
    };
    let r = cam_sketch_solver::solve_drag_with_cancel(&d, std::slice::from_ref(&target), || false)
        .unwrap();
    assert!(r.accepted());
    assert!((r.candidate.point(&target.point).unwrap().x - 35.).abs() < 1e-3);
    assert_eq!(r.degrees_of_freedom, Some(3));
    let mut invalid = target;
    invalid.target.x = f64::NAN;
    assert!(cam_sketch_solver::solve_drag_with_cancel(&d, &[invalid], || false).is_err());
    assert_eq!(d.point(&p("circle", PointRole::Center)).unwrap().x, 30.);
}

#[test]
fn diameter_dimensions_preserve_units_in_solver_measurement_and_json() {
    let mut d = SketchDocument {
        entities: vec![e(
            "hole",
            SketchGeometry::Circle {
                center: Point::new(0., 0.),
                radius_mm: 2.,
            },
        )],
        ..Default::default()
    };
    add(
        &mut d,
        ConstraintKind::Diameter {
            curve: "hole".into(),
            value_mm: 10.,
        },
    );
    d.constraints.push(Constraint {
        id: "reference".into(),
        reference: true,
        definition: ConstraintKind::Diameter {
            curve: "hole".into(),
            value_mm: 4.,
        },
    });
    let solved = solve(&d).unwrap();
    assert!(solved.accepted());
    assert!(
        (solved
            .candidate
            .entity("hole")
            .unwrap()
            .geometry
            .circle()
            .unwrap()
            .1
            - 5.)
            .abs()
            < 1e-6
    );
    assert!(
        (solved
            .validation
            .constraints
            .iter()
            .find(|v| v.constraint_id == "reference")
            .unwrap()
            .measured
            .unwrap()
            - 10.)
            .abs()
            < 1e-6
    );
    let restored: SketchDocument =
        serde_json::from_str(&serde_json::to_string(&solved.candidate).unwrap()).unwrap();
    assert_eq!(restored, solved.candidate);
    add(
        &mut d,
        ConstraintKind::Radius {
            curve: "hole".into(),
            value_mm: 2.,
        },
    );
    assert!(!solve(&d).unwrap().accepted());
}

#[test]
fn rectangle_solves_endpoint_roles_and_publishes_geometry() {
    let mut d = SketchDocument {
        entities: vec![
            line("a", (0., 0.), (20., 0.)),
            line("b", (20.1, 0.1), (20.2, 9.9)),
            line("c", (20.1, 10.1), (-0.1, 10.2)),
            line("d", (-0.2, 10.1), (0.1, 0.1)),
        ],
        ..Default::default()
    };
    fixed(&mut d, "a");
    for (a, b) in [("a", "b"), ("b", "c"), ("c", "d"), ("d", "a")] {
        add(
            &mut d,
            ConstraintKind::Coincident {
                a: p(a, PointRole::End),
                b: p(b, PointRole::Start),
            },
        );
    }
    for line in ["b", "d"] {
        add(&mut d, ConstraintKind::Vertical { line: line.into() });
    }
    add(&mut d, ConstraintKind::Horizontal { line: "c".into() });
    add(
        &mut d,
        ConstraintKind::Length {
            line: "b".into(),
            value_mm: 10.,
        },
    );
    d.wires.push(Wire {
        id: "plate".into(),
        label: None,
        closed: true,
        edges: ["a", "b", "c", "d"]
            .iter()
            .map(|id| WireEdge {
                entity_id: (*id).into(),
                reversed: false,
            })
            .collect(),
    });
    let before = d.clone();
    let r = solve(&d).unwrap();
    assert_eq!(before, d);
    assert!(r.accepted(), "{:?}", r.validation.issues);
    assert_eq!(r.degrees_of_freedom, Some(0));
    let m = materialize::materialize(&r.candidate, &Default::default(), None).unwrap();
    assert!((m.geometry.selected.area_mm2() - 200.).abs() < 0.001);
    let saved = serde_json::to_string(&r.candidate).unwrap();
    let reopened: SketchDocument = serde_json::from_str(&saved).unwrap();
    assert_eq!(r.candidate, reopened);
}
#[test]
fn dimensional_reference_fixed_and_conflicting_constraints() {
    let mut d = SketchDocument {
        entities: vec![line("line", (1., 1.), (9., 2.))],
        ..Default::default()
    };
    add(
        &mut d,
        ConstraintKind::Horizontal {
            line: "line".into(),
        },
    );
    add(
        &mut d,
        ConstraintKind::Length {
            line: "line".into(),
            value_mm: 12.,
        },
    );
    let r = solve(&d).unwrap();
    assert!(r.accepted(), "{:?}", r.validation.issues);
    assert_eq!(r.degrees_of_freedom, Some(2));
    add(
        &mut d,
        ConstraintKind::Length {
            line: "line".into(),
            value_mm: 50.,
        },
    );
    d.constraints.last_mut().unwrap().reference = true;
    let solved = accepted(&d);
    assert!(
        (solved
            .line("line")
            .unwrap()
            .0
            .distance(solved.line("line").unwrap().1)
            - 12.)
            .abs()
            < 1e-6
    );
    d.constraints.last_mut().unwrap().reference = false;
    let r = solve(&d).unwrap();
    assert!(!r.accepted());
    assert!(
        r.validation
            .issues
            .iter()
            .any(|e| e.code == "SKETCH_UNSATISFIED")
    );
    let mut d = SketchDocument {
        entities: vec![point("p", 2., 3.)],
        ..Default::default()
    };
    fixed(&mut d, "p");
    d.entities[0] = point("p", 20., 30.);
    assert_eq!(accepted(&d).point(&pos("p")).unwrap(), Point::new(2., 3.));
    add(
        &mut d,
        ConstraintKind::Fixed {
            entity: "p".into(),
            target: SketchGeometry::Point {
                position: Point::new(9., 9.),
            },
        },
    );
    assert!(!solve(&d).unwrap().accepted());
}
#[test]
fn circle_radius_equal_radius_and_signed_arc_endpoints() {
    let mut d = SketchDocument {
        entities: vec![
            e(
                "a",
                SketchGeometry::Circle {
                    center: Point::new(0., 0.),
                    radius_mm: 3.,
                },
            ),
            e(
                "b",
                SketchGeometry::Circle {
                    center: Point::new(10., 0.),
                    radius_mm: 4.,
                },
            ),
        ],
        ..Default::default()
    };
    add(
        &mut d,
        ConstraintKind::Radius {
            curve: "a".into(),
            value_mm: 5.,
        },
    );
    add(
        &mut d,
        ConstraintKind::EqualRadius {
            a: "a".into(),
            b: "b".into(),
        },
    );
    let s = accepted(&d);
    assert!((s.circle("b").unwrap().1 - 5.).abs() < 1e-6);
    add(
        &mut d,
        ConstraintKind::Radius {
            curve: "a".into(),
            value_mm: 7.,
        },
    );
    assert!(!solve(&d).unwrap().accepted());
    let mut d = SketchDocument {
        entities: vec![
            e(
                "arc",
                SketchGeometry::Arc {
                    center: Point::new(0., 0.),
                    radius_mm: 5.,
                    start_angle_rad: 0.,
                    sweep_angle_rad: -std::f64::consts::FRAC_PI_2,
                },
            ),
            point("start", 5.1, 0.1),
            point("end", 0.1, -5.1),
            point("center", 0.1, 0.1),
        ],
        ..Default::default()
    };
    fixed(&mut d, "arc");
    for (id, role) in [
        ("start", PointRole::Start),
        ("end", PointRole::End),
        ("center", PointRole::Center),
    ] {
        add(
            &mut d,
            ConstraintKind::Coincident {
                a: p("arc", role),
                b: pos(id),
            },
        );
    }
    accepted(&d);
}
#[test]
fn all_remaining_common_constraints_are_mapped() {
    let mut base = SketchDocument {
        entities: vec![
            line("axis", (-10., 0.), (10., 0.)),
            line("other", (-4., 5.), (4., 5.1)),
            point("p", 2., 3.),
            point("q", -2., -3.),
            e(
                "circle",
                SketchGeometry::Circle {
                    center: Point::new(0., 5.),
                    radius_mm: 5.,
                },
            ),
        ],
        ..Default::default()
    };
    fixed(&mut base, "axis");
    let cases = vec![
        ConstraintKind::Parallel {
            a: "axis".into(),
            b: "other".into(),
        },
        ConstraintKind::Perpendicular {
            a: "axis".into(),
            b: "other".into(),
        },
        ConstraintKind::EqualLength {
            a: "axis".into(),
            b: "other".into(),
        },
        ConstraintKind::Angle {
            a: "axis".into(),
            b: "other".into(),
            value_rad: 0.3,
        },
        ConstraintKind::PointOnLine {
            point: pos("p"),
            line: "axis".into(),
        },
        ConstraintKind::Midpoint {
            point: pos("p"),
            line: "axis".into(),
        },
        ConstraintKind::PointOnCircle {
            point: pos("p"),
            curve: "circle".into(),
        },
        ConstraintKind::Symmetry {
            a: pos("p"),
            b: pos("q"),
            line: "axis".into(),
        },
        ConstraintKind::Distance {
            a: pos("p"),
            b: pos("q"),
            value_mm: 5.,
        },
        ConstraintKind::HorizontalDistance {
            a: pos("p"),
            b: pos("q"),
            value_mm: 5.,
        },
        ConstraintKind::VerticalDistance {
            a: pos("p"),
            b: pos("q"),
            value_mm: -3.,
        },
        ConstraintKind::Tangent {
            line: "axis".into(),
            curve: "circle".into(),
        },
    ];
    for c in cases {
        let mut d = base.clone();
        add(&mut d, c.clone());
        let r = solve(&d).unwrap();
        assert!(
            r.accepted(),
            "{c:?}: {} {:?}",
            r.solver_status,
            r.validation.issues
        );
    }
}
#[test]
fn cancellation_empty_invalid_and_degenerate_inputs() {
    let d = SketchDocument::default();
    assert!(solve(&d).unwrap().accepted());
    let r = solve_with_cancel(&d, || true).unwrap();
    assert!(matches!(r.status, SolveStatus::Cancelled));
    assert_eq!(r.candidate, d);
    let mut d = SketchDocument {
        entities: vec![line("bad", (0., 0.), (0., 0.))],
        ..Default::default()
    };
    assert!(!solve(&d).unwrap().accepted());
    add(
        &mut d,
        ConstraintKind::Length {
            line: "missing".into(),
            value_mm: 10.,
        },
    );
    assert!(solve(&d).is_err());
    let mut d = SketchDocument {
        entities: vec![line("line", (0., 0.), (9., 1.))],
        ..Default::default()
    };
    add(
        &mut d,
        ConstraintKind::Horizontal {
            line: "line".into(),
        },
    );
    let mut calls = 0;
    let r = solve_with_cancel(&d, || {
        calls += 1;
        calls == 3
    })
    .unwrap();
    assert!(matches!(r.status, SolveStatus::Cancelled));
    assert!(r.iterations <= 64);
}

#[test]
fn portable_saved_fixture_and_json_transport_resize_a_fully_constrained_plate() {
    let input = include_str!("../../../fixtures/sketch/rectangle.json");
    let mut d: SketchDocument = serde_json::from_str(input).unwrap();
    let baseline = solve(&d).unwrap();
    assert!(baseline.accepted());
    assert_eq!(baseline.degrees_of_freedom, Some(0));
    let c = d
        .constraints
        .iter_mut()
        .find(|c| matches!(&c.definition, ConstraintKind::Length { line, .. } if line == "a"))
        .unwrap();
    if let ConstraintKind::Length { value_mm, .. } = &mut c.definition {
        *value_mm = 24.;
    }
    let json = cam_sketch_solver::solve_json(&serde_json::to_string(&d).unwrap()).unwrap();
    let result: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(result["status"], "accepted");
    assert_eq!(result["degrees_of_freedom"], 0);
    let candidate: SketchDocument = serde_json::from_value(result["candidate"].clone()).unwrap();
    assert!((candidate.line("a").unwrap().1.x - 26.).abs() < 1e-6);
    materialize::materialize(&candidate, &Default::default(), None).unwrap();
    assert!(cam_sketch_solver::solve_json(&" ".repeat(MAX_SKETCH_BYTES + 1)).is_err());
}
