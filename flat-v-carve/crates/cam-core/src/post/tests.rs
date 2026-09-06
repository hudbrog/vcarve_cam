use super::*;

#[test]
fn micro_segments_keep_their_direction_and_required_z_at_selected_precision() {
    let mut profile = LinuxCncProfile::from_json(include_str!(
        "../../../../fixtures/m6/macro-stock-bottom.json"
    ))
    .unwrap();
    let mut m = Motion {
        id: 122,
        tool_id: "endmill".into(),
        operation_id: "flat-v-carve".into(),
        layer: 0,
        kind: MotionKind::Cut,
        start: Position {
            x: 39.13473125,
            y: 30.979125,
            z: -1.,
        },
        end: Position {
            x: 39.1347375,
            y: 30.979125,
            z: -1.,
        },
        feed_mm_min: Some(2000.),
    };
    let check = |m: &Motion, p: &LinuxCncProfile| {
        preserves_motions(
            &[Stage {
                role: "endmill",
                id: "endmill",
                motions: std::slice::from_ref(m),
                spindle: 12000.,
            }],
            p,
            20.,
        )
    };
    profile.decimal_places = 3;
    assert!(!check(&m, &profile));
    profile.decimal_places = 6;
    assert!(check(&m, &profile));
    m.kind = MotionKind::Plunge;
    m.end = Position {
        z: m.start.z - 0.00001,
        ..m.start
    };
    profile.decimal_places = 3;
    assert!(!check(&m, &profile));
    profile.decimal_places = 6;
    assert!(check(&m, &profile));
    // No precision within the supported subset can rescue a sub-nanometer
    // entry. The independent reader must continue to refuse it.
    let job = Job::from_json(include_str!("../../../../fixtures/m4/narrow-channel.json")).unwrap();
    let mut motions = vec![
        Motion {
            kind: MotionKind::Approach,
            start: Position { z: 5., ..m.start },
            end: Position { z: 0., ..m.start },
            ..m.clone()
        },
        Motion {
            start: Position { z: 0., ..m.start },
            end: Position {
                z: -1e-12,
                ..m.start
            },
            ..m
        },
    ];
    for (i, m) in motions.iter_mut().enumerate() {
        m.id = i;
    }
    let source = SourcePlan {
        job: &job,
        input_fingerprint: "test",
        motion_fingerprint: "test",
        endmill: &motions,
        vbit: &[],
        incomplete: false,
    };
    profile.decimal_places = 9;
    let stage = Stage {
        role: "endmill",
        id: "endmill",
        motions: &motions,
        spindle: 12000.,
    };
    let program = emit(&source, &profile, &[&stage], "combined.ngc", 8.).unwrap();
    let diagnostic = reader::read(&program.gcode, &profile, &[&stage], 8.)
        .err()
        .unwrap();
    assert_eq!(diagnostic.code, "POST_ROUNDING");
    assert!(diagnostic.message.contains("motion 1"));
}
