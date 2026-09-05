use cam_core::{
    geometry::{Point, Region},
    job::JobInspection,
    svg::Bounds,
};
use std::fmt::Write;
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn text(svg: &mut String, x: f64, y: f64, size: u32, s: &str) {
    let _ = write!(
        svg,
        "<text x=\"{x}\" y=\"{y}\" font-size=\"{size}\">{}</text>",
        escape(s)
    );
}
pub fn failure(message: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1400\" height=\"220\" viewBox=\"0 0 1400 220\"><rect width=\"1400\" height=\"220\" fill=\"#fff1f2\"/><g font-family=\"sans-serif\" fill=\"#9f1239\"><text x=\"30\" y=\"55\" font-size=\"26\">Job preview unavailable</text><text x=\"30\" y=\"100\" font-size=\"15\">{}</text><text x=\"30\" y=\"155\">Previous geometry invalidated. Check the diagnostic report.</text></g></svg>\n",
        escape(message)
    )
}
fn path(region: &Region, xy: &impl Fn(Point) -> Point) -> String {
    let mut d = String::new();
    for ring in region.rings_mm() {
        for (i, p) in ring.into_iter().enumerate() {
            let p = xy(p);
            let _ = write!(
                d,
                "{} {:.6} {:.6} ",
                if i == 0 { 'M' } else { 'L' },
                p.x,
                p.y
            );
        }
        d.push_str("Z ");
    }
    d
}
pub fn render(inspection: &JobInspection) -> String {
    let g = &inspection.geometry;
    let mut bounds = Bounds {
        min: Point::new(f64::INFINITY, f64::INFINITY),
        max: Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
    };
    for s in &g.sources {
        if let Some(b) = Bounds::of(&s.geometry) {
            bounds.min.x = bounds.min.x.min(b.min.x);
            bounds.min.y = bounds.min.y.min(b.min.y);
            bounds.max.x = bounds.max.x.max(b.max.x);
            bounds.max.y = bounds.max.y.max(b.max.y);
        }
    }
    let width = (bounds.max.x - bounds.min.x).max(1e-9);
    let height = (bounds.max.y - bounds.min.y).max(1e-9);
    let k = (850. / width).min(490. / height);
    let xy = |p: Point| {
        Point::new(
            45. + (p.x - bounds.min.x) * k,
            665. - (p.y - bounds.min.y) * k,
        )
    };
    let bottom = 750. + g.sources.len() as f64 * 27.;
    let height = bottom + 100. + g.diagnostics.len().min(20) as f64 * 22.;
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1400\" height=\"{height}\" viewBox=\"0 0 1400 {height}\"><title>{}</title><rect width=\"1400\" height=\"{height}\" fill=\"#f8fafc\"/><g font-family=\"sans-serif\" fill=\"#0f172a\">",
        escape(&inspection.name)
    );
    text(&mut out, 32., 43., 27, &inspection.name);
    text(
        &mut out,
        32.,
        75.,
        15,
        "M2 SVG import · Workpiece XY in millimeters · Saved source and editable settings",
    );
    text(
        &mut out,
        32.,
        105.,
        14,
        &format!(
            "Page: {:.4} × {:.4} mm · Selected area: {:.6} mm² · {} components / {} holes",
            g.page_width_mm,
            g.page_height_mm,
            g.selected.area_mm2(),
            g.selected.component_count(),
            g.selected.hole_count()
        ),
    );
    text(
        &mut out,
        32.,
        132.,
        13,
        "Blue: selected union · Gray: other source regions · Y increases upward",
    );
    for s in &g.sources {
        let _ = write!(
            out,
            "<path d=\"{}\" fill=\"#e2e8f0\" fill-rule=\"evenodd\" stroke=\"#94a3b8\" stroke-width=\"1\"/>",
            path(&s.geometry, &xy)
        );
    }
    let _ = write!(
        out,
        "<path d=\"{}\" fill=\"#bfdbfe\" fill-rule=\"evenodd\" stroke=\"#2563eb\" stroke-width=\"1.6\"/>",
        path(&g.selected, &xy)
    );
    text(&mut out, 970., 185., 19, "Job settings");
    text(
        &mut out,
        970.,
        216.,
        14,
        &format!(
            "{} machining fields unset",
            inspection.missing_machining_fields.len()
        ),
    );
    text(
        &mut out,
        970.,
        244.,
        14,
        "Geometry is ready for inspection.",
    );
    text(
        &mut out,
        970.,
        272.,
        14,
        "M3 endmill planning is available.",
    );
    text(
        &mut out,
        970.,
        318.,
        14,
        &format!("Curve bound: {} mm", g.flattening_bound_mm),
    );
    text(
        &mut out,
        970.,
        346.,
        14,
        &format!("Grid: {} ticks/mm", g.grid.scale()),
    );
    text(
        &mut out,
        970.,
        374.,
        14,
        &format!("Snap bound: {:.7} mm", g.grid.snap_bound_mm()),
    );
    text(
        &mut out,
        970.,
        402.,
        14,
        &format!("Source snap: {:.7} mm", g.source_snap_bound_mm),
    );
    text(
        &mut out,
        32.,
        714.,
        18,
        "Source regions (stable within the embedded artwork)",
    );
    for (i, s) in g.sources.iter().enumerate() {
        text(
            &mut out,
            40.,
            747. + i as f64 * 27.,
            14,
            &format!(
                "{}  {}  · {:.6} mm²{}",
                if g.selected_region_ids.contains(&s.id) {
                    "[selected]"
                } else {
                    "[ ]"
                },
                s.id,
                s.geometry.area_mm2(),
                s.label
                    .as_ref()
                    .map_or(String::new(), |l| format!(" · {l}"))
            ),
        );
    }
    text(
        &mut out,
        32.,
        bottom + 25.,
        13,
        "Preview recomputed from the embedded SVG. Curve/snap budgets describe geometry; this is not machining verification.",
    );
    for (i, d) in g.diagnostics.iter().take(20).enumerate() {
        text(
            &mut out,
            32.,
            bottom + 55. + i as f64 * 22.,
            12,
            &format!("{}: {}", d.code, d.message),
        );
    }
    out.push_str("</g></svg>\n");
    out
}
