//! Published motion shape: the export report measures what it emitted, and
//! names the stages whose moves are finer than the machine can execute.
use cam_core::{
    motion::Position,
    post::{
        PathControl,
        motion_profile::{MICRO_MOVE_MM, MotionLengthProfile, MotionProfileReport},
        sequence::{PreparedProcess, PreparedSpindle, PreparedStage},
    },
    sequence::{ExecutionStage, StageRole},
    toolpath::{Interpolation, MotionEffect, MotionPurpose, PlannedMotion},
};

fn feed(id: usize, from: (f64, f64), to: (f64, f64)) -> PlannedMotion {
    motion(id, Interpolation::LinearFeed, from, to)
}

fn rapid(id: usize, from: (f64, f64), to: (f64, f64)) -> PlannedMotion {
    motion(id, Interpolation::Rapid, from, to)
}

fn motion(
    id: usize,
    interpolation: Interpolation,
    from: (f64, f64),
    to: (f64, f64),
) -> PlannedMotion {
    PlannedMotion {
        id,
        operation_id: "op".into(),
        stage_id: "op-stage".into(),
        tool_id: "tool".into(),
        contour_id: None,
        pass_id: 0,
        layer: 0,
        interpolation,
        purpose: MotionPurpose::KnifeCut,
        effect: MotionEffect::KnifeTrace,
        start: Position::new(cam_core::geometry::Point::new(from.0, from.1), -1.),
        end: Position::new(cam_core::geometry::Point::new(to.0, to.1), -1.),
        feed_mm_min: Some(1000.),
        blade_heading_deg: None,
    }
}

fn stage(range: (usize, usize), role: StageRole, path_control: PathControl) -> PreparedStage {
    PreparedStage {
        stage: ExecutionStage {
            stage_id: "op-stage".into(),
            operation_id: "op".into(),
            tool_id: "tool".into(),
            role,
            motion_range: range,
            entry_position: Position::new(cam_core::geometry::Point::new(0., 0.), 0.),
            exit_position: Position::new(cam_core::geometry::Point::new(0., 0.), 0.),
        },
        process: PreparedProcess {
            spindle: PreparedSpindle::Off,
            coolant: cam_core::post::Coolant::Off,
            path_control,
        },
        tool_number: 3,
        length_offset_number: None,
    }
}

#[test]
fn lengths_quantiles_shares_and_density_come_from_the_xy_feed_moves() {
    // Four XY feed moves (2^-10, 2^-7, 2^-6 and 2^-4 mm, exactly
    // representable so the buckets are not decided by rounding), one vertical
    // feed with no XY travel and one long rapid: the rapid is travel, never a
    // feed, and the vertical is a motion without being a lateral move.
    let first = 0.0009765625;
    let second = 0.0078125;
    let third = 0.015625;
    let fourth = 0.0625;
    let a = first;
    let b = a + second;
    let c = b + third;
    let d = c + fourth;
    let motions = vec![
        feed(0, (0., 0.), (a, 0.)),
        feed(1, (a, 0.), (b, 0.)),
        feed(2, (b, 0.), (c, 0.)),
        feed(3, (c, 0.), (d, 0.)),
        motion(4, Interpolation::LinearFeed, (d, 0.), (d, 0.)),
        rapid(5, (d, 0.), (d + 10., 0.)),
    ];
    let profile = MotionLengthProfile::of(&motions);
    assert_eq!(profile.motions, 6);
    assert_eq!(profile.feed_motions, 5);
    assert_eq!(profile.rapid_motions, 1);
    assert_eq!(profile.xy_feed_moves, 4);
    let feed_length = first + second + third + fourth;
    assert_eq!(profile.feed_length_mm, feed_length);
    assert_eq!(profile.xy_travel_mm, feed_length + 10.);
    assert_eq!(profile.shortest_feed_mm, first);
    assert_eq!(profile.longest_feed_mm, fourth);
    assert_eq!(profile.median_feed_mm, third);
    assert_eq!(profile.share_feed_under_0_01_mm, 0.5);
    assert_eq!(profile.share_feed_under_0_05_mm, 0.75);
    assert!((profile.feed_moves_per_mm - 4. / feed_length).abs() < 1e-9);
    // The histogram always carries the whole ladder, including empty buckets.
    assert_eq!(profile.histogram.len(), 12);
    assert_eq!(profile.histogram[0].upper_mm, Some(0.002));
    assert_eq!(profile.histogram[0].motions, 1);
    assert_eq!(profile.histogram[2].upper_mm, Some(0.01));
    assert_eq!(profile.histogram[2].motions, 1);
    assert_eq!(profile.histogram[5].upper_mm, Some(0.1));
    assert_eq!(profile.histogram[5].motions, 1);
    assert_eq!(profile.histogram.last().unwrap().upper_mm, None);
    assert_eq!(profile.histogram.last().unwrap().motions, 0);
    assert!(profile.is_micro_move_bound());
}

#[test]
fn a_coarse_program_reports_nothing() {
    let motions = vec![
        feed(0, (0., 0.), (20., 0.)),
        feed(1, (20., 0.), (20., 20.)),
        feed(2, (20., 20.), (0., 20.)),
    ];
    let report = MotionProfileReport::of(
        &[stage((0, 3), StageRole::Knife, PathControl::ExactPath)],
        &motions,
        3,
        3,
    );
    assert_eq!(report.total.xy_feed_moves, 3);
    assert!(report.observations().is_empty());
    assert_eq!(report.stages.len(), 1);
    assert_eq!(report.stages[0].profile.xy_feed_moves, 3);
}

#[test]
fn exact_path_over_micro_moves_and_escalated_precision_are_named() {
    // Ten 0.01 mm moves over 0.1 mm of path: 10 moves/mm, median far below
    // the machine's ability to reach feed.
    let motions: Vec<_> = (0..10)
        .map(|i| feed(i, (i as f64 * 0.01, 0.), ((i + 1) as f64 * 0.01, 0.)))
        .collect();
    let report = MotionProfileReport::of(
        &[stage((0, 10), StageRole::Knife, PathControl::ExactPath)],
        &motions,
        3,
        5,
    );
    let observations = report.observations();
    let codes: Vec<&str> = observations.iter().map(|o| o.code.as_str()).collect();
    assert_eq!(
        codes,
        vec![
            "EXPORT_PRECISION_ESCALATED",
            "EXPORT_EXACT_PATH_MICRO_MOVES"
        ]
    );
    let micro = &observations[1];
    assert_eq!(micro.stage_id.as_deref(), Some("op-stage"));
    assert!(micro.message.contains("exact path mode stops"), "{micro:?}");
    assert!(micro.message.contains("median is 0.0100 mm"), "{micro:?}");
    // The same path under blending is reported without the stop warning.
    let blended = MotionProfileReport::of(
        &[stage(
            (0, 10),
            StageRole::VcarveFinish,
            PathControl::Blend {
                tolerance_mm: 0.05,
                naive_cam_tolerance_mm: Some(0.05),
            },
        )],
        &motions,
        5,
        5,
    );
    let observations = blended.observations();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].code, "EXPORT_MICRO_MOVE_DENSITY");
    assert!(!observations[0].message.contains("stops the machine"));
    assert!(
        observations[0]
            .message
            .contains(&format!("{MICRO_MOVE_MM} mm"))
    );
}

#[test]
fn an_empty_slice_measures_zero_without_dividing_by_it() {
    let profile = MotionLengthProfile::of(&[]);
    assert_eq!(profile, MotionLengthProfile::default());
    assert_eq!(profile.feed_moves_per_mm, 0.);
    assert!(!profile.is_micro_move_bound());
    assert_eq!(profile.histogram.len(), 12);
}
