use super::plan_svg::{escape, region, text};
use cam_core::{geometry::Point, motion::MotionKind, svg::Bounds, vcarve::CombinedPlan};
use std::fmt::Write;
pub fn render(plan: &CombinedPlan) -> String {
    let source = plan
        .endmill
        .job
        .inspect()
        .expect("validated plan")
        .geometry
        .selected;
    let mut bounds = Bounds::of(&source).unwrap();
    for m in &plan.vbit_motions {
        for p in [m.start.xy(), m.end.xy()] {
            bounds.min.x = bounds.min.x.min(p.x);
            bounds.min.y = bounds.min.y.min(p.y);
            bounds.max.x = bounds.max.x.max(p.x);
            bounds.max.y = bounds.max.y.max(p.y);
        }
    }
    let k = (400. / (bounds.max.x - bounds.min.x).max(1e-9))
        .min(270. / (bounds.max.y - bounds.min.y).max(1e-9));
    let height = 1030 + plan.analysis.diagnostics.len() * 24;
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1440\" height=\"{height}\" viewBox=\"0 0 1440 {height}\"><title>{}</title><rect width=\"1440\" height=\"{height}\" fill=\"#f8fafc\"/><g font-family=\"sans-serif\" fill=\"#0f172a\">",
        escape(&plan.endmill.job.name)
    );
    text(&mut out, 30., 40., 26, &plan.endmill.job.name);
    text(
        &mut out,
        30.,
        72.,
        16,
        &format!(
            "M4 combined stage: {:?} · {} endmill moves → {} V-bit moves",
            plan.analysis.status,
            plan.endmill.motions.len(),
            plan.vbit_motions.len()
        ),
    );
    text(
        &mut out,
        30.,
        99.,
        13,
        &format!(
            "Final finish: {}/{} paths · {} proved air paths omitted · {} cleanup executions · {} quality samples",
            plan.analysis.finish_paths_executed,
            plan.analysis.finish_paths_expected,
            plan.analysis.pruned_air_paths,
            plan.analysis.cleanup_paths,
            plan.analysis.samples.len()
        ),
    );
    text(
        &mut out,
        30.,
        124.,
        13,
        "Blue: final V-bit finish · Teal: clearing · Dashed gray: clearance links · Orange: cutter-limited detail · Pink: missed reachable stock",
    );
    let floor = plan
        .analysis
        .slices
        .iter()
        .find(|s| s.depth_mm == plan.analysis.floor_check_depth_mm)
        .unwrap();
    for col in 0..3 {
        let left = 30. + col as f64 * 470.;
        let xy = |p: Point| {
            Point::new(
                left + 15. + (p.x - bounds.min.x) * k,
                465. - (p.y - bounds.min.y) * k,
            )
        };
        let title = match col {
            0 => "Recorded V-bit centers".into(),
            1 => format!("Combined removal at {:.3} mm", floor.depth_mm),
            _ => "Sampled residual depth".into(),
        };
        text(&mut out, left, 160., 18, &title);
        region(&mut out, &source, &xy, "#e2e8f0", "#94a3b8");
        if col == 0 {
            let final_start = plan
                .executions
                .iter()
                .find(|e| e.final_finish)
                .map_or(usize::MAX, |e| e.first_motion_id);
            for m in &plan.vbit_motions {
                let a = xy(m.start.xy());
                let b = xy(m.end.xy());
                let color = if m.kind.rapid() {
                    "#94a3b8"
                } else if m.id >= final_start {
                    "#2563eb"
                } else {
                    "#0d9488"
                };
                if a.distance(b) < 1e-8 {
                    if m.kind == MotionKind::Plunge {
                        let _ = write!(
                            out,
                            "<circle cx=\"{}\" cy=\"{}\" r=\"1.5\" fill=\"{color}\"/>",
                            a.x, a.y
                        );
                    }
                } else {
                    let _ = write!(
                        out,
                        "<path d=\"M {} {} L {} {}\" stroke=\"{color}\" stroke-width=\"{}\" {}><title>Move {}: {:?}; Z {} to {}</title></path>",
                        a.x,
                        a.y,
                        b.x,
                        b.y,
                        if m.id >= final_start { 1.5 } else { 0.6 },
                        if m.kind.rapid() {
                            "stroke-dasharray=\"3 4\""
                        } else {
                            ""
                        },
                        m.id,
                        m.kind,
                        m.start.z,
                        m.end.z
                    );
                }
            }
        } else if col == 1 {
            region(&mut out, &floor.removal.lower, &xy, "#86efac", "#16a34a");
            region(
                &mut out,
                &plan.analysis.missing_floor_beyond_tolerance,
                &xy,
                "#f43f5e",
                "#be123c",
            );
        } else {
            for s in &plan.analysis.samples {
                let p = xy(s.point);
                let color = if s.missed_reachable_mm
                    > s.allowed_ridge_mm
                        + plan
                            .endmill
                            .job
                            .tolerances
                            .verification_tolerance_mm
                            .unwrap()
                {
                    "#f43f5e"
                } else if s.unavoidable_residual_lower_mm > 0.001 {
                    "#f59e0b"
                } else {
                    "#16a34a"
                };
                let _ = write!(
                    out,
                    "<circle cx=\"{}\" cy=\"{}\" r=\"2.2\" fill=\"{color}\"><title>Residual {:.5} mm; reachable shortfall {:.5} mm; unavoidable [{:.5}, {:.5}] mm</title></circle>",
                    p.x,
                    p.y,
                    s.residual_mm,
                    s.missed_reachable_mm,
                    s.unavoidable_residual_lower_mm,
                    s.unavoidable_residual_upper_mm
                );
            }
        }
        let label = match col {
            0 => format!(
                "{} medial branches ({} curved)",
                plan.analysis.medial_axis.branches.len(),
                plan.analysis.medial_axis.curved_branches
            ),
            1 => format!(
                "Missing accessible floor: {:.4} mm²",
                plan.analysis.missing_floor_beyond_tolerance.area_mm2()
            ),
            _ => format!(
                "Max sampled residual: {:.4} mm",
                plan.analysis.max_sampled_residual_mm
            ),
        };
        text(&mut out, left, 497., 13, &label);
    }
    let mut chosen = vec![
        0,
        plan.analysis.slices.len().saturating_sub(2),
        plan.analysis.slices.len() - 1,
    ];
    chosen.dedup();
    for (col, &i) in chosen.iter().enumerate() {
        let s = &plan.analysis.slices[i];
        let left = 30. + col as f64 * 470.;
        let xy = |p: Point| {
            Point::new(
                left + 15. + (p.x - bounds.min.x) * k,
                865. - (p.y - bounds.min.y) * k,
            )
        };
        text(
            &mut out,
            left,
            556.,
            18,
            &format!("Remaining at depth {:.3} mm", s.depth_mm),
        );
        region(&mut out, &source, &xy, "#e2e8f0", "#94a3b8");
        region(&mut out, &s.nominal_section, &xy, "#f8fafc", "#cbd5e1");
        region(&mut out, &s.remaining_target, &xy, "#fbbf24", "#d97706");
        region(&mut out, &s.possible_overcut, &xy, "#a855f7", "#7e22ce");
        text(
            &mut out,
            left,
            897.,
            13,
            &format!(
                "Remaining {:.3} mm² · Possible overcut {:.4} mm²",
                s.remaining_target.area_mm2(),
                s.possible_overcut.area_mm2()
            ),
        );
    }
    text(
        &mut out,
        30.,
        938.,
        13,
        &format!(
            "Allowed floor ridge: {} mm · Allowed unreachable detail: {} mm · Amber slice regions include permitted ridges and cutter-limited detail.",
            plan.endmill.job.operation.max_floor_ridge_mm.unwrap(),
            plan.endmill.job.operation.max_detail_residual_mm.unwrap()
        ),
    );
    text(
        &mut out,
        30.,
        965.,
        13,
        "Sampled quality and fixed slices are M4 evidence. Continuous segment clearance is checked; adaptive full-volume verification remains M5.",
    );
    for (i, d) in plan.analysis.diagnostics.iter().enumerate() {
        text(&mut out, 30., 997. + i as f64 * 24., 13, &d.code);
    }
    out.push_str("</g></svg>\n");
    out
}
