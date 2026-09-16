//! Arc geometry in the removal model: the prefix stock query follows a
//! programmed arc, not its chord, and the shared distance helper agrees with
//! the closed-form cases.
use cam_core::{
    geometry::Point,
    motion::Position,
    project::RectXY,
    stock::history::{StockHistory, SweepBatch, SweepCutter, SweepMotion},
    toolpath::{ArcMove, Interpolation, MotionEffect, MotionPurpose, PlannedMotion},
};

/// The upper half of a 5 mm circle about the origin, run from (-5,0) to (5,0).
fn upper_half() -> ArcMove {
    ArcMove {
        center: Point::new(0., 0.),
        clockwise: true,
    }
}

fn arc_motion(cut_z: f64, arc: Option<ArcMove>) -> PlannedMotion {
    PlannedMotion {
        id: 0,
        operation_id: "op".into(),
        stage_id: "op-rough".into(),
        tool_id: "tool".into(),
        contour_id: None,
        pass_id: 0,
        layer: 0,
        interpolation: match arc {
            Some(arc) => Interpolation::ArcFeed(arc),
            None => Interpolation::LinearFeed,
        },
        purpose: MotionPurpose::Rough,
        effect: MotionEffect::MillingSweep,
        start: Position::new(Point::new(-5., 0.), cut_z),
        end: Position::new(Point::new(5., 0.), cut_z),
        feed_mm_min: Some(500.),
        blade_heading_deg: None,
    }
}

#[test]
fn the_distance_helper_matches_the_closed_form_cases() {
    let arc = upper_half();
    let (from, to) = (Point::new(-5., 0.), Point::new(5., 0.));
    // On the arc: zero.
    assert!(arc.distance(from, to, Point::new(0., 5.)) < 1e-12);
    // Radially outside the sweep: 7 - 5 = 2.
    assert!((arc.distance(from, to, Point::new(0., 7.)) - 2.).abs() < 1e-12);
    // Inside the half disc but not on the arc: 5 - 3 = 2.
    assert!((arc.distance(from, to, Point::new(0., 3.)) - 2.).abs() < 1e-12);
    // Below the chord the sweep does not reach, so the nearest point is an
    // endpoint rather than the radial projection.
    let below = Point::new(0., -1.);
    assert!((arc.distance(from, to, below) - below.distance(from)).abs() < 1e-12);
    // A full circle closes on itself instead of collapsing to a point.
    let full = ArcMove {
        center: Point::new(0., 0.),
        clockwise: false,
    };
    let closed = (Point::new(1., 0.), Point::new(1., 0.));
    assert!((full.distance(closed.0, closed.1, Point::new(0., 1.)) - 0.).abs() < 1e-12);
}

#[test]
fn a_programmed_arc_sweep_follows_the_curve_not_the_chord() {
    let mut history = StockHistory::new(
        4.,
        Some(RectXY {
            min_x_mm: -10.,
            min_y_mm: -10.,
            width_mm: 20.,
            length_mm: 20.,
        }),
    )
    .unwrap();
    history.push(SweepBatch {
        stage_id: "op-rough".into(),
        operation_id: "op".into(),
        cutter: SweepCutter::FlatEndmill { radius_mm: 0.5 },
        motions: vec![SweepMotion {
            start: Position::new(Point::new(-5., 0.), -1.),
            end: Position::new(Point::new(5., 0.), -1.),
            arc: Some(upper_half()),
        }],
    });
    // On the arc the floor is reached.
    assert_eq!(history.material_top_at(Point::new(0., 5.)).unwrap(), -1.);
    // A millimetre off the arc is untouched.
    assert_eq!(history.material_top_at(Point::new(0., 4.)).unwrap(), 0.);
    // The chord's own midpoint is 5 mm from the arc, so the arc sweep leaves
    // it alone: this is what distinguishes following the curve from following
    // the chord.
    assert_eq!(history.material_top_at(Point::new(0., 0.)).unwrap(), 0.);

    // The same motion as a straight sweep does cut that midpoint, so the
    // assertion above is about the arc and not about the batch being ignored.
    let mut straight = StockHistory::new(
        4.,
        Some(RectXY {
            min_x_mm: -10.,
            min_y_mm: -10.,
            width_mm: 20.,
            length_mm: 20.,
        }),
    )
    .unwrap();
    straight.push(SweepBatch {
        stage_id: "op-rough".into(),
        operation_id: "op".into(),
        cutter: SweepCutter::FlatEndmill { radius_mm: 0.5 },
        motions: vec![SweepMotion {
            start: Position::new(Point::new(-5., 0.), -1.),
            end: Position::new(Point::new(5., 0.), -1.),
            arc: None,
        }],
    });
    assert_eq!(straight.material_top_at(Point::new(0., 0.)).unwrap(), -1.);
}

#[test]
fn a_planned_arc_motion_reports_the_sweep_the_stock_model_reads() {
    // The plan-to-sweep conversion is what the prefix model consumes, so a
    // planned arc must arrive with its centre rather than as a chord.
    let motion = arc_motion(-1., Some(upper_half()));
    let distance = cam_core::toolpath::motion_distance(&motion, Point::new(0., 5.));
    assert!(distance < 1e-12);
    let as_chord = arc_motion(-1., None);
    assert!(
        cam_core::toolpath::motion_distance(&as_chord, Point::new(0., 5.)) > 4.9,
        "the chord form must not be mistaken for the curve"
    );
}
