use cam_gui1::sim::{Field, Motion, Playback, Stock, ToolSpec, choose_resolution};

fn stock() -> Stock {
    Stock {
        x0: -4.5,
        y0: -4.5,
        x1: 4.5,
        y1: 4.5,
        thickness_mm: 4.,
    }
}
fn plunge(tool: usize, z: f64) -> Motion {
    Motion {
        kind: "plunge".into(),
        tool,
        x0: 0.,
        y0: 0.,
        z0: 1.,
        x1: 0.,
        y1: 0.,
        z1: z,
    }
}
fn depth(f: &Field, c: usize, r: usize) -> f64 {
    f.cell_at(c, r).0 as f64 * f.quantum
}

#[test]
fn analytic_disc_cone_and_finite_tip() {
    let mut disc = Field::new(stock(), &[ToolSpec::Endmill { diameter: 4. }], 1.).unwrap();
    disc.apply(&plunge(0, -2.), 0., 1.).unwrap();
    assert!((depth(&disc, 4, 4) - 2.).abs() <= disc.quantum);
    assert!((depth(&disc, 6, 4) - 2.).abs() <= disc.quantum);
    assert_eq!(depth(&disc, 6, 5), 0.); // outside the radius-two disc
    for tip in [0., 2.] {
        let mut cone = Field::new(
            stock(),
            &[ToolSpec::Vbit {
                angle: 90.,
                tip,
                diameter: 8.,
                height: 4.,
            }],
            1.,
        )
        .unwrap();
        cone.apply(&plunge(0, -3.), 0., 1.).unwrap();
        for col in 4..8 {
            let radius = (col - 4) as f64;
            let expected = (3. - (radius - tip / 2.).max(0.)).max(0.);
            assert!((depth(&cone, col, 4) - expected).abs() <= cone.quantum * 1.01);
        }
    }
}
#[test]
fn ramp_interval_and_tool_ownership_are_not_point_samples() {
    let tools = [
        ToolSpec::Endmill { diameter: 2. },
        ToolSpec::Vbit {
            angle: 90.,
            tip: 2.,
            diameter: 6.,
            height: 3.,
        },
    ];
    let mut f = Field::new(stock(), &tools, 1.).unwrap();
    let ramp = Motion {
        kind: "ramp".into(),
        tool: 0,
        x0: -3.,
        y0: 0.,
        z0: 0.,
        x1: 3.,
        y1: 0.,
        z1: -3.,
    };
    f.apply(&ramp, 0., 1.).unwrap();
    // Centre cell remains covered until t=2/3; the lowest tip is -2 mm.
    assert!((depth(&f, 4, 4) - 2.).abs() <= f.quantum);
    assert_eq!(f.cell_at(4, 4).1, 1);
    f.apply(&plunge(1, -3.), 0., 1.).unwrap();
    assert_eq!(f.cell_at(4, 4).1, 2);
    let before = f.cell_bytes();
    let mut rapid = plunge(0, -4.);
    rapid.kind = "rapid_x_y".into();
    f.apply(&rapid, 0., 1.).unwrap();
    assert_eq!(before, f.cell_bytes());
    let mut partial = Field::new(stock(), &tools, 1.).unwrap();
    partial.apply(&ramp, 0., 0.5).unwrap();
    assert!((depth(&partial, 4, 4) - 1.5).abs() <= partial.quantum);
}
#[test]
fn backwards_seek_restores_exact_cells_versions_and_stats_with_bounded_checkpoints() {
    let f = Field::new(stock(), &[ToolSpec::Endmill { diameter: 2. }], 0.025).unwrap();
    let motions: Vec<_> = (1..=8).map(|i| plunge(0, -(i as f64) / 2.)).collect();
    for budget in [0, 256 * 256 * 3, 4 * 256 * 256 * 3] {
        let mut player = Playback::new(f.clone(), budget);
        for prefix in [3, 8, 2, 7, 0, 8] {
            player.seek(&motions, prefix).unwrap();
            let mut direct = f.clone();
            for m in &motions[..prefix] {
                direct.apply(m, 0., 1.).unwrap();
            }
            assert_eq!(player.field.cell_bytes(), direct.cell_bytes());
            assert_eq!(player.field.checksum(), direct.checksum());
            assert_eq!(
                serde_json::to_value(&player.field.stats).unwrap(),
                serde_json::to_value(&direct.stats).unwrap()
            );
            assert!(player.checkpoint_bytes() <= budget);
        }
    }
}

#[test]
fn transported_checkpoints_survive_interactive_seeking() {
    let f = Field::new(stock(), &[ToolSpec::Endmill { diameter: 2. }], 0.25).unwrap();
    let motions: Vec<_> = (1..=64).map(|i| plunge(0, -(i as f64) / 8.)).collect();
    let points = vec![
        (0_usize, f.clone()),
        (16, {
            let mut field = f.clone();
            for motion in &motions[..16] {
                field.apply(motion, 0., 1.).unwrap();
            }
            field
        }),
        (64, {
            let mut field = f.clone();
            for motion in &motions {
                field.apply(motion, 0., 1.).unwrap();
            }
            field
        }),
    ];
    let seed_prefixes: Vec<usize> = points.iter().map(|(prefix, _)| *prefix).collect();
    let latest = points.last().unwrap().1.clone();
    let mut playback = Playback::seed(latest, f.clone(), points, 20 * 1024 * 1024);
    let mut worst = 0;
    for target in [7, 33, 61, 3, 48, 12, 64, 20, 55, 1] {
        playback.seek(&motions, target).unwrap();
        // Replay work is bounded by the pinned seed spacing even after many
        // seeks have added their own checkpoints.
        let nearest = playback
            .checkpoint_prefixes()
            .into_iter()
            .filter(|prefix| *prefix <= target)
            .max()
            .unwrap_or(0);
        worst = worst.max(target - nearest);
        for prefix in &seed_prefixes {
            assert!(
                playback.checkpoint_prefixes().contains(prefix),
                "seed checkpoint {prefix} was evicted"
            );
        }
        let mut direct = f.clone();
        for motion in &motions[..target] {
            direct.apply(motion, 0., 1.).unwrap();
        }
        assert_eq!(playback.field.cell_bytes(), direct.cell_bytes());
    }
    assert!(worst <= 32, "replay window grew to {worst} motions");
}
#[test]
fn admission_rejects_nonfinite_and_overbudget_without_large_allocations() {
    assert!(
        ToolSpec::Vbit {
            angle: 180.,
            tip: 0.,
            diameter: 2.,
            height: 3.
        }
        .normalize()
        .is_err()
    );
    assert!(Field::new(stock(), &[ToolSpec::Endmill { diameter: 1. }], 1e-100).is_err());
    let mut f = Field::new(stock(), &[ToolSpec::Endmill { diameter: 1. }], 1.).unwrap();
    let mut bad = plunge(0, -2.);
    bad.x1 = f64::NAN;
    assert!(f.apply(&bad, 0., 1.).is_err());
    assert_eq!(f.stats.applied_motions, 0);
    assert!(f.apply(&plunge(1, -2.), 0., 1.).is_err());
    let r = choose_resolution(100., 50., 0.1, 100., 1000.).unwrap();
    assert!(r.capped_by_texels && r.capped_by_budget);
}

#[test]
fn preview_checkpoints_include_both_stages_and_obey_the_retained_budget() {
    let input = cam_gui1::sim::Input {
        stock: stock(),
        tools: vec![ToolSpec::Endmill { diameter: 2. }],
        resolution: choose_resolution(9., 9., 0.01, 8192., 64_000_000.).unwrap(),
        motions: (1..=20).map(|i| plunge(0, -(i as f64) / 10.)).collect(),
        prefixes: vec![],
    };
    let preview = cam_gui1::stock_preview::build(&input, 7).unwrap();
    assert!(preview.meta.cell_mm > preview.meta.reference_cell_mm);
    assert!(preview.meta.retained_bytes <= cam_gui1::stock_preview::MAX_PREVIEW_BYTES);
    assert!(preview.meta.frames.iter().any(|f| f.prefix == 7));
    assert_eq!(preview.meta.frames.first().unwrap().prefix, 0);
    assert_eq!(preview.meta.frames.last().unwrap().prefix, 20);
    let mut field = Field::new(input.stock, &input.tools, preview.meta.cell_mm).unwrap();
    for motion in &input.motions {
        field.apply(motion, 0., 1.).unwrap();
    }
    // The transported checkpoint is the tile-major packed grid of the same field.
    assert_eq!(preview.cells.last().unwrap(), &field.packed_tile_bytes());
    assert_eq!(
        preview.meta.frames.last().unwrap().checksum,
        field.checksum()
    );
    assert_eq!(preview.meta.frames.last().unwrap().versions, field.versions);
}
