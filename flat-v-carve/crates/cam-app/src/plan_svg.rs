use cam_core::{
    geometry::{Point, Region},
    motion::MotionKind,
    pocket::EndmillPlan,
    svg::Bounds,
};
use std::fmt::Write;
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn text(out: &mut String, x: f64, y: f64, size: u32, s: &str) {
    let _ = write!(
        out,
        "<text x=\"{x}\" y=\"{y}\" font-size=\"{size}\">{}</text>",
        escape(s)
    );
}
fn region(out: &mut String, r: &Region, xy: &impl Fn(Point) -> Point, fill: &str, stroke: &str) {
    let mut d = String::new();
    for ring in r.rings_mm() {
        for (i, p) in ring.into_iter().enumerate() {
            let p = xy(p);
            let _ = write!(
                d,
                "{} {:.4} {:.4} ",
                if i == 0 { 'M' } else { 'L' },
                p.x,
                p.y
            );
        }
        d.push_str("Z ");
    }
    let _ = write!(
        out,
        "<path d=\"{d}\" fill=\"{fill}\" fill-rule=\"evenodd\" stroke=\"{stroke}\" stroke-width=\"0.7\"/>"
    );
}
pub fn render(plan: &EndmillPlan) -> String {
    let count = plan.analysis.layers.len().min(16);
    let mut codes: Vec<_> = plan
        .analysis
        .diagnostics
        .iter()
        .map(|d| d.code.as_str())
        .collect();
    codes.sort();
    codes.dedup();
    let height = 220 + count * 390 + codes.len() * 25;
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1440\" height=\"{height}\" viewBox=\"0 0 1440 {height}\"><title>{}</title><rect width=\"1440\" height=\"{height}\" fill=\"#f8fafc\"/><g font-family=\"sans-serif\" fill=\"#0f172a\">",
        escape(&plan.job.name)
    );
    text(&mut out, 30., 40., 26, &plan.job.name);
    text(
        &mut out,
        30.,
        72.,
        16,
        &format!(
            "M3 endmill stage: {:?} · {} verified motions · stock-top Z = 0 mm",
            plan.analysis.status,
            plan.motions.len()
        ),
    );
    text(
        &mut out,
        30.,
        101.,
        13,
        "Blue: cuts · Orange: ramps / plunges · Dashed gray: clearance links · Green: removed stock · Amber: remaining target",
    );
    text(
        &mut out,
        30.,
        125.,
        13,
        "Reports are recomputed from recorded XYZ moves. Coverage applies to accessible floors at these slices; full-volume verification is M5.",
    );
    let source = plan
        .job
        .inspect()
        .expect("plan was validated before rendering")
        .geometry
        .selected;
    let mut bounds = Bounds::of(&source).expect("nonempty planning selection");
    for m in &plan.motions {
        for p in [m.start.xy(), m.end.xy()] {
            bounds.min.x = bounds.min.x.min(p.x);
            bounds.min.y = bounds.min.y.min(p.y);
            bounds.max.x = bounds.max.x.max(p.x);
            bounds.max.y = bounds.max.y.max(p.y);
        }
    }
    let k = (400. / (bounds.max.x - bounds.min.x).max(1e-9))
        .min(270. / (bounds.max.y - bounds.min.y).max(1e-9));
    for (i, l) in plan.analysis.layers.iter().take(count).enumerate() {
        let top = 155. + i as f64 * 390.;
        for (col, title) in [
            "Recorded motion centers",
            "Removed stock at this depth",
            "Remaining target / missing floor",
        ]
        .iter()
        .enumerate()
        {
            let left = 30. + col as f64 * 470.;
            let xy = |p: Point| {
                Point::new(
                    left + 15. + (p.x - bounds.min.x) * k,
                    top + 315. - (p.y - bounds.min.y) * k,
                )
            };
            text(&mut out, left, top, 16, title);
            region(&mut out, &source, &xy, "#e2e8f0", "#94a3b8");
            if col == 0 {
                region(&mut out, &l.requested_centers, &xy, "#dbeafe", "#93c5fd");
                for m in plan.motions.iter().filter(|m| m.layer == i) {
                    let a = xy(m.start.xy());
                    let b = xy(m.end.xy());
                    let color = if m.kind.rapid() {
                        "#64748b"
                    } else if m.kind == MotionKind::Cut {
                        "#2563eb"
                    } else {
                        "#ea580c"
                    };
                    if a.distance(b) < 1e-8 {
                        let _ = write!(
                            out,
                            "<circle cx=\"{}\" cy=\"{}\" r=\"2\" fill=\"{color}\"><title>Move {}: {:?}, Z {} to {}</title></circle>",
                            a.x, a.y, m.id, m.kind, m.start.z, m.end.z
                        );
                    } else {
                        let _ = write!(
                            out,
                            "<path d=\"M {} {} L {} {}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"1\" {}><title>Move {}: {:?}, Z {} to {}</title></path>",
                            a.x,
                            a.y,
                            b.x,
                            b.y,
                            if m.kind.rapid() {
                                "stroke-dasharray=\"4 4\""
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
                region(&mut out, &l.removal.lower, &xy, "#86efac", "#16a34a");
            } else {
                region(&mut out, &l.nominal_section, &xy, "#fef3c7", "#eab308");
                region(&mut out, &l.removal.lower, &xy, "#f8fafc", "none");
                region(&mut out, &l.remaining_target, &xy, "#fbbf24", "#d97706");
                region(
                    &mut out,
                    &l.missing_floor_beyond_tolerance,
                    &xy,
                    "#f43f5e",
                    "#be123c",
                );
                region(&mut out, &l.possible_overcut, &xy, "#a855f7", "#7e22ce");
            }
            let label = match col {
                0 => format!("Depth {:.3} mm · {:?}", l.depth_mm, l.status),
                1 => format!(
                    "Lower removal {:.3} mm² · {} contributing moves",
                    l.removal.lower.area_mm2(),
                    l.removal.contributing_motion_ids.len()
                ),
                _ => format!(
                    "Remaining {:.3} mm² · Missing floor {:.4} mm²",
                    l.remaining_target.area_mm2(),
                    l.missing_floor_beyond_tolerance.area_mm2()
                ),
            };
            text(&mut out, left, top + 347., 13, &label);
        }
    }
    let y = 165. + count as f64 * 390.;
    text(
        &mut out,
        30.,
        y,
        13,
        &format!(
            "Showing {count}/{} slices. Pink: missing floor beyond XY tolerance; purple: possible overcut. All geometry and diagnostics are in the JSON report.",
            plan.analysis.layers.len()
        ),
    );
    for (i, code) in codes.iter().enumerate() {
        text(&mut out, 30., y + 27. + i as f64 * 25., 14, code);
    }
    out.push_str("</g></svg>\n");
    out
}
