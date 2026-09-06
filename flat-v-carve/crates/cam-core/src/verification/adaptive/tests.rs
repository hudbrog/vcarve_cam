use super::*;
use crate::motion::{MotionKind, Position};

#[test]
fn parallel_cells_preserve_serial_results_and_global_resource_limits() {
    let job = job();
    let plan = crate::vcarve::plan_combined(&job).unwrap();
    let ctx = Context::new(&job).unwrap();
    for cells in [1, 63, 4096] {
        let options = VerificationOptions {
            max_cells: cells,
            max_findings: 1,
            ..Default::default()
        };
        let check = |workers| {
            run_with_workers(
                &ctx,
                &plan.endmill.motions,
                &plan.vbit_motions,
                &options,
                None,
                vec![],
                None,
                workers,
            )
            .unwrap()
        };
        let serial = check(0);
        assert!(serial.evaluated_cells <= cells);
        assert!(serial.findings.len() <= options.max_findings);
        if cells == 1 {
            assert_eq!(serial.status, VerificationStatus::Inconclusive);
        }
        assert_eq!(
            serde_json::to_vec(&serial).unwrap(),
            serde_json::to_vec(&check(4)).unwrap()
        );
    }
}

fn job() -> Job {
    Job::from_json(include_str!(
        "../../../../../fixtures/m4/curved-medial.json"
    ))
    .unwrap()
}

fn cut(id: usize, a: Point, b: Point, da: f64, db: f64) -> Motion {
    Motion {
        id,
        tool_id: "vbit".into(),
        operation_id: "flat-v-carve".into(),
        layer: 0,
        kind: MotionKind::Cut,
        start: Position::new(a, -da),
        end: Position::new(b, -db),
        feed_mm_min: Some(100.),
    }
}

fn cell_evaluation(ctx: &Context<'_>, bounds: Bounds, motions: &[Motion]) -> Evaluation {
    let sweeps: Vec<_> = motions
        .iter()
        .map(|motion| Sweep {
            endmill: false,
            motion,
            bounds,
        })
        .collect();
    evaluate(
        ctx,
        &Cell {
            bounds,
            depth: 0,
            sweeps: (0..motions.len()).collect(),
        },
        &sweeps,
        &VerificationOptions::default(),
        false,
    )
    .unwrap()
}

#[test]
fn convex_roof_bounds_a_rising_sweep_beyond_both_adjacent_boundary_faces() {
    // The whole rectangle projects beyond the ends of the two faces at the
    // reentrant (8,8) corner. The former affine/on-face bound cannot apply.
    for offset in [Point::new(0., 0.), Point::new(10000., -7000.)] {
        for tip in [0., 0.1] {
            let mut job = job();
            job.import.placement.origin_mm = offset;
            let Some(ToolGeometry::Vbit(spec)) = &mut job.tools[1].geometry else {
                panic!()
            };
            spec.tip_diameter_mm = 2. * tip;
            let ctx = Context::new(&job).unwrap();
            let shift = |p: Point| Point::new(p.x + offset.x, p.y + offset.y);
            let bounds = Bounds {
                min: shift(Point::new(7.3, 7.3)),
                max: shift(Point::new(7.5, 7.5)),
            };
            let slope = ctx.vbit.angle().slope();
            let motions = [cut(
                0,
                shift(Point::new(6.8, 6.8)),
                shift(Point::new(7., 7.)),
                (1.2 * 2f64.sqrt() - tip) / slope - 0.01,
                (2f64.sqrt() - tip) / slope - 0.01,
            )];
            let evaluation = cell_evaluation(&ctx, bounds, &motions);
            assert!(evaluation.errors[4].upper < 0.05, "{:?}", evaluation.errors);

            // Independent reference: uniformly sample the XYZ motion, without
            // the production stationary/closest-point optimizer. Its maximum
            // underestimation is bounded by the motion's Lipschitz constant.
            let m = &motions[0];
            let steps = 1024;
            let sample_error = ((m.end.depth() - m.start.depth()).abs()
                + m.start.xy().distance(m.end.xy()) / slope)
                / (2. * steps as f64);
            for y in 0..=16 {
                for x in 0..=16 {
                    let p = Point::new(
                        bounds.min.x + (bounds.max.x - bounds.min.x) * x as f64 / 16.,
                        bounds.min.y + (bounds.max.y - bounds.min.y) * y as f64 / 16.,
                    );
                    let target = (ctx.target.boundary().sample(p).unwrap().signed_distance_mm
                        / slope)
                        .clamp(0., ctx.target.depth_cap().mm());
                    let removal = (0..=steps)
                        .map(|i| {
                            let q = m.start.lerp(m.end, i as f64 / steps as f64);
                            (q.depth() - (q.xy().distance(p) - tip).max(0.) / slope).max(0.)
                        })
                        .fold(0., f64::max);
                    assert!(
                        target - removal <= evaluation.errors[4].upper + sample_error,
                        "residual at {p:?} exceeds whole-cell bound"
                    );
                }
            }
        }
    }
}

#[test]
fn different_sweeps_covering_all_corners_do_not_hide_an_interior_gap() {
    let job = job();
    let ctx = Context::new(&job).unwrap();
    let bounds = Bounds {
        min: Point::new(3., 3.),
        max: Point::new(5., 5.),
    };
    let corners = [
        bounds.min,
        Point::new(5., 3.),
        bounds.max,
        Point::new(3., 5.),
    ];
    let target_at = |p| {
        (ctx.target.boundary().sample(p).unwrap().signed_distance_mm / ctx.vbit.angle().slope())
            .clamp(0., ctx.target.depth_cap().mm())
    };
    let motions: Vec<_> = corners
        .iter()
        .enumerate()
        .map(|(i, &p)| cut(i, p, p, 0., target_at(p)))
        .collect();
    let evaluation = cell_evaluation(&ctx, bounds, &motions);
    assert!(
        corners
            .iter()
            .all(|&p| vbit_removed_depth_at(&motions, &ctx.vbit, p).unwrap() >= target_at(p))
    );
    assert!(evaluation.errors[4].lower > 1.);
    assert!(evaluation.errors[4].upper >= evaluation.errors[4].lower);
}
