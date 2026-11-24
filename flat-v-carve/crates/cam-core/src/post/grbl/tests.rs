use super::*;
use crate::{
    geometry::Point,
    sequence::{CoolantIntent, ProcessSpindle},
    toolpath::{ArcMove, Interpolation, MotionEffect, MotionPurpose, PlannedMotion},
};
fn profile() -> GrblProfile {
    serde_json::from_str(include_str!(
        "../../../../../fixtures/gui2/grbl-machine.json"
    ))
    .unwrap()
}
fn p(x: f64, y: f64, z: f64) -> Position {
    Position { x, y, z }
}

#[test]
fn the_same_machine_actions_have_the_same_bytes_and_requirements_for_every_producer() {
    let profile = profile();
    let intent = ProcessIntent {
        spindle: ProcessSpindle::Off,
        coolant: CoolantIntent::Off,
        path_control: PathControlIntent::UseMachineProfile,
    };
    let motion = PlannedMotion {
        id: 0,
        operation_id: "pocket".into(),
        stage_id: "rough".into(),
        tool_id: "tool".into(),
        contour_id: None,
        pass_id: 0,
        layer: 0,
        interpolation: Interpolation::ArcFeed(ArcMove {
            center: Point::new(0., 0.),
            clockwise: false,
        }),
        purpose: MotionPurpose::Rough,
        effect: MotionEffect::MillingSweep,
        start: p(1., 0., 5.),
        end: p(0., 1., 4.),
        feed_mm_min: Some(100.),
        blade_heading_deg: None,
    };
    let generate = |m: &PlannedMotion| {
        let run = Run {
            process: process(&intent, &m.stage_id, &profile).unwrap(),
            motions: std::slice::from_ref(m).into(),
        };
        blocks::program(&[run], &profile, p(0., 0., 0.), 3, 0.0005).unwrap()
    };
    let gcode = generate(&motion);
    let mut other = motion.clone();
    other.operation_id = "profile".into();
    other.stage_id = "finish".into();
    other.purpose = MotionPurpose::Finish;
    assert_eq!(gcode, generate(&other));
    let constrained = ProcessIntent {
        path_control: PathControlIntent::ExactPath,
        ..intent.clone()
    };
    for m in [&motion, &other] {
        let e = process(&constrained, &m.stage_id, &profile).unwrap_err();
        assert_eq!(e.code, "PROCESS_PATH_CONTROL");
        assert_eq!(e.source_id.as_ref(), Some(&m.stage_id));
    }
    let runs = [Run {
        process: process(&intent, &motion.stage_id, &profile).unwrap(),
        motions: std::slice::from_ref(&motion).into(),
    }];
    for (a, b) in [
        ("G54", "G55"),
        ("G0 Z5\nG0 X", "G0 X"),
        ("G3 ", "G2 "),
        ("I-1", "I-0.5"),
        ("F100", "F99"),
        ("M5", "M3 S1234"),
        ("M2", "M6"),
        ("G17", "G18"),
    ] {
        let altered = gcode.replacen(a, b, 1);
        assert_ne!(altered, gcode);
        assert!(
            blocks::verify(&altered, &runs, &profile, p(0., 0., 0.), 0.0005).is_err(),
            "{altered}"
        );
    }
}

#[test]
fn controller_precision_cannot_turn_a_required_move_into_a_stationary_feed() {
    let profile = profile();
    let m = PlannedMotion {
        id: 0,
        operation_id: "op".into(),
        stage_id: "stage".into(),
        tool_id: "tool".into(),
        contour_id: None,
        pass_id: 0,
        layer: 0,
        interpolation: Interpolation::LinearFeed,
        purpose: MotionPurpose::Finish,
        effect: MotionEffect::MillingSweep,
        start: p(10000., 1., 5.),
        end: p(10000.00001, 1., 5.),
        feed_mm_min: Some(100.),
        blade_heading_deg: None,
    };
    let runs = [Run {
        process: Process {
            spindle: PreparedSpindle::Off,
            coolant: Coolant::Off,
        },
        motions: std::slice::from_ref(&m).into(),
    }];
    assert_eq!(
        blocks::program(&runs, &profile, p(0., 0., 0.), 9, 0.0005)
            .unwrap_err()
            .code,
        "POST_PRECISION"
    );
}
