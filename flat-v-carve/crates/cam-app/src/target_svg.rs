use cam_core::{geometry::Point, preview::TargetPreview, target::CenterSet};
use std::fmt::Write;

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
pub fn failure(message: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1440 220\"><rect width=\"1440\" height=\"220\" fill=\"#fff1f2\"/><g font-family=\"sans-serif\" fill=\"#9f1239\"><text x=\"30\" y=\"55\" font-size=\"28\">Invalid model — preview unavailable</text><text x=\"30\" y=\"100\" font-size=\"16\">{}</text><text x=\"30\" y=\"155\" font-size=\"14\">The previous geometry preview has been invalidated. See input.json and report.json.</text></g></svg>\n",
        escape(message)
    )
}
fn text(svg: &mut String, x: f64, y: f64, size: usize, value: &str) {
    let _ = write!(
        svg,
        "<text x=\"{x}\" y=\"{y}\" font-size=\"{size}\">{}</text>",
        escape(value)
    );
}
fn path(rings: &[Vec<Point>], xy: &impl Fn(Point) -> (f64, f64)) -> String {
    let mut d = String::new();
    for ring in rings {
        for (i, &p) in ring.iter().enumerate() {
            let (x, y) = xy(p);
            let _ = write!(d, "{} {x:.6} {y:.6} ", if i == 0 { 'M' } else { 'L' });
        }
        d.push_str("Z ");
    }
    d
}
fn center_overlay(
    svg: &mut String,
    centers: &CenterSet,
    color: &str,
    xy: &impl Fn(Point) -> (f64, f64),
) {
    let _ = write!(
        svg,
        "<path d=\"{}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"1.8\"/>",
        path(&centers.area.rings_mm(), xy)
    );
    for s in &centers.contact_segments {
        let (x1, y1) = xy(s.start);
        let (x2, y2) = xy(s.end);
        let _ = write!(
            svg,
            "<line x1=\"{x1}\" y1=\"{y1}\" x2=\"{x2}\" y2=\"{y2}\" stroke=\"{color}\" stroke-width=\"4\"/>"
        );
    }
    for &p in &centers.contact_points {
        let (x, y) = xy(p);
        let _ = write!(
            svg,
            "<circle cx=\"{x}\" cy=\"{y}\" r=\"4\" fill=\"{color}\"/>"
        );
    }
}

pub fn render(preview: &TargetPreview) -> String {
    let rows = preview.sections.len().div_ceil(3);
    let profiles_top = 160.0 + rows as f64 * 320.0;
    let height = profiles_top + preview.cross_sections.len() as f64 * 275.0 + 100.0;
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1440 {height}\" width=\"1440\" height=\"{height}\"><title>{}</title><rect width=\"1440\" height=\"{height}\" fill=\"#f8fafc\"/><g font-family=\"sans-serif\" fill=\"#0f172a\">",
        escape(&preview.input.description)
    );
    text(
        &mut svg,
        32.0,
        38.0,
        26,
        &preview.input.id.replace('_', " "),
    );
    text(
        &mut svg,
        32.0,
        65.0,
        14,
        &format!(
            "M1 target and cutter geometry · {:?} preview · dimensions in mm · stock top Z = 0",
            preview.status
        ),
    );
    text(
        &mut svg,
        32.0,
        90.0,
        14,
        &format!(
            "Depth cap {} · V-bit {}° / tip Ø{} · endmill Ø{} · wall allowance {}",
            preview.input.max_depth_mm,
            preview.input.vbit.included_angle_deg,
            preview.input.vbit.tip_diameter_mm,
            preview.input.endmill.diameter_mm,
            preview.input.wall_allowance_mm
        ),
    );
    text(
        &mut svg,
        32.0,
        119.0,
        13,
        "Blue: nominal depth section · Orange: V-bit centers · Teal: endmill centers · thick lines/dots: exact-fit contacts",
    );
    text(
        &mut svg,
        32.0,
        142.0,
        13,
        "Gray: surface opening and profile lines · Pink profile samples: finite-tip residual above the requested preview tolerance",
    );
    let rings = preview.normalized_region.rings_mm();
    let minx = rings
        .iter()
        .flatten()
        .map(|p| p.x)
        .fold(f64::INFINITY, f64::min);
    let miny = rings
        .iter()
        .flatten()
        .map(|p| p.y)
        .fold(f64::INFINITY, f64::min);
    let maxx = rings
        .iter()
        .flatten()
        .map(|p| p.x)
        .fold(f64::NEG_INFINITY, f64::max);
    let maxy = rings
        .iter()
        .flatten()
        .map(|p| p.y)
        .fold(f64::NEG_INFINITY, f64::max);
    let scale = (390.0 / (maxx - minx).max(1e-6)).min(220.0 / (maxy - miny).max(1e-6));
    for (i, section) in preview.sections.iter().enumerate() {
        let left = 28.0 + (i % 3) as f64 * 470.0;
        let top = 160.0 + (i / 3) as f64 * 320.0;
        let _ = write!(
            svg,
            "<rect x=\"{left}\" y=\"{top}\" width=\"450\" height=\"305\" rx=\"8\" fill=\"white\" stroke=\"#cbd5e1\"/>"
        );
        text(
            &mut svg,
            left + 15.0,
            top + 25.0,
            16,
            &format!("Depth {} · Z {}", section.depth_mm, section.machine_z_mm),
        );
        let xy = |p: Point| {
            (
                left + 30.0 + (p.x - minx) * scale,
                top + 265.0 - (p.y - miny) * scale,
            )
        };
        let _ = write!(
            svg,
            "<path d=\"{}\" fill=\"#f1f5f9\" fill-rule=\"evenodd\" stroke=\"#94a3b8\" stroke-width=\"1\"/>",
            path(&rings, &xy)
        );
        let _ = write!(
            svg,
            "<path d=\"{}\" fill=\"#dbeafe\" fill-rule=\"evenodd\" stroke=\"#2563eb\" stroke-width=\"1.8\"/>",
            path(&section.nominal.area.rings_mm(), &xy)
        );
        center_overlay(&mut svg, &section.nominal, "#2563eb", &xy);
        center_overlay(&mut svg, &section.vbit, "#d97706", &xy);
        center_overlay(&mut svg, &section.endmill, "#0d9488", &xy);
        for profile in &preview.cross_sections {
            let (x1, y1) = xy(profile.input.start);
            let (x2, y2) = xy(profile.input.end);
            let _ = write!(
                svg,
                "<line x1=\"{x1}\" y1=\"{y1}\" x2=\"{x2}\" y2=\"{y2}\" stroke=\"#64748b\" stroke-width=\"0.8\" stroke-dasharray=\"3 4\"/>"
            );
            for sample in &profile.samples {
                if sample.vbit_removal.unavoidable_residual_lower_mm
                    > preview.input.preview_depth_tolerance_mm
                {
                    let (x, y) = xy(sample.xy);
                    let _ = write!(
                        svg,
                        "<circle cx=\"{x}\" cy=\"{y}\" r=\"1.3\" fill=\"#e11d48\"><title>finite-tip residual {:.6}..{:.6} mm</title></circle>",
                        sample.vbit_removal.unavoidable_residual_lower_mm,
                        sample.vbit_removal.unavoidable_residual_upper_mm
                    );
                }
            }
        }
        text(
            &mut svg,
            left + 15.0,
            top + 291.0,
            11,
            &format!(
                "V-bit: {:?} · endmill: {:?}",
                section.vbit.status, section.endmill.status
            ),
        );
    }
    for (i, profile) in preview.cross_sections.iter().enumerate() {
        let top = profiles_top + i as f64 * 275.0;
        text(
            &mut svg,
            32.0,
            top + 22.0,
            18,
            &format!("Profile: {}", profile.input.id.replace('_', " ")),
        );
        let length = profile.input.start.distance(profile.input.end);
        let max_depth = profile
            .samples
            .iter()
            .map(|s| s.nominal_depth_mm)
            .fold(0.0, f64::max)
            .max(0.001)
            * 1.08;
        let xy = |distance: f64, depth: f64| {
            (
                90.0 + distance / length * 1260.0,
                top + 45.0 + depth / max_depth * 155.0,
            )
        };
        for tick in 0..=4 {
            let distance = length * tick as f64 / 4.0;
            let (x, _) = xy(distance, 0.0);
            let _ = write!(
                svg,
                "<line x1=\"{x}\" y1=\"{}\" x2=\"{x}\" y2=\"{}\" stroke=\"#e2e8f0\"/>",
                top + 45.0,
                top + 200.0
            );
            text(
                &mut svg,
                x - 12.0,
                top + 220.0,
                12,
                &format!("{distance:.2}"),
            );
            let depth = max_depth * tick as f64 / 4.0;
            let (_, y) = xy(0.0, depth);
            let _ = write!(
                svg,
                "<line x1=\"90\" y1=\"{y}\" x2=\"1350\" y2=\"{y}\" stroke=\"#e2e8f0\"/>"
            );
            text(
                &mut svg,
                35.0,
                y + 4.0,
                12,
                &format!("{:.3}", if depth == 0.0 { 0.0 } else { -depth }),
            );
        }
        let mut nominal = String::new();
        let mut lower = String::new();
        let mut upper = String::new();
        for sample in &profile.samples {
            let (x, y) = xy(sample.distance_mm, sample.nominal_depth_mm);
            let _ = write!(nominal, "{x:.5},{y:.5} ");
            let (x, y) = xy(
                sample.distance_mm,
                sample.vbit_removal.reachable_depth_lower_mm,
            );
            let _ = write!(lower, "{x:.5},{y:.5} ");
            let (x, y) = xy(
                sample.distance_mm,
                sample.vbit_removal.reachable_depth_upper_mm,
            );
            let _ = write!(upper, "{x:.5},{y:.5} ");
        }
        let reverse = |s: &str| s.split_whitespace().rev().collect::<Vec<_>>().join(" ");
        let _ = write!(
            svg,
            "<polygon points=\"{nominal} {}\" fill=\"#fecdd3\"/><polygon points=\"{lower} {}\" fill=\"#fde68a\"/><polyline points=\"{nominal}\" fill=\"none\" stroke=\"#2563eb\" stroke-width=\"2\"/><polyline points=\"{lower}\" fill=\"none\" stroke=\"#d97706\" stroke-width=\"2\"/>",
            reverse(&upper),
            reverse(&upper)
        );
        text(
            &mut svg,
            90.0,
            top + 247.0,
            12,
            "Distance along profile (mm) · Blue: nominal surface · Orange: V-bit reachable-depth interval · Pink: V-bit geometric limit",
        );
    }
    let footer = height - 70.0;
    text(
        &mut svg,
        32.0,
        footer,
        12,
        "Preview of normalized geometry and idealized cutters. Exact-fit contacts need entry planning and a numerical margin check before machining.",
    );
    text(
        &mut svg,
        32.0,
        footer + 22.0,
        12,
        "Reachability bounds apply at the sampled XY locations; connecting profile points is a visual interpolation, not stock verification.",
    );
    text(
        &mut svg,
        32.0,
        footer + 44.0,
        11,
        &format!(
            "Diagnostics: {}",
            preview
                .diagnostics
                .iter()
                .map(|d| d.code.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );
    svg.push_str("</g></svg>\n");
    svg
}
