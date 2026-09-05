use cam_core::{
    geometry::Point,
    spike::{FixtureResult, Operation},
};
use std::fmt::Write;

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn render(result: &FixtureResult) -> String {
    let mut all = result.fixture.rings.clone();
    if let Operation::Boolean { other, .. } = &result.fixture.operation {
        all.extend(other.clone());
    }
    let points: Vec<Point> = all
        .iter()
        .flatten()
        .copied()
        .filter(|p| p.x.is_finite() && p.y.is_finite())
        .collect();
    let min_x = points.iter().map(|p| p.x).reduce(f64::min).unwrap_or(0.0);
    let min_y = points.iter().map(|p| p.y).reduce(f64::min).unwrap_or(0.0);
    let max_x = points.iter().map(|p| p.x).reduce(f64::max).unwrap_or(10.0);
    let max_y = points.iter().map(|p| p.y).reduce(f64::max).unwrap_or(10.0);
    let scale = (680.0 / (max_x - min_x).max(1.0)).min(440.0 / (max_y - min_y).max(1.0));
    let xy = |p: Point| (60.0 + (p.x - min_x) * scale, 540.0 - (p.y - min_y) * scale);
    let path = |rings: &[Vec<Point>]| {
        let mut d = String::new();
        for ring in rings {
            for (i, &p) in ring.iter().enumerate() {
                let (x, y) = xy(p);
                let _ = write!(d, "{} {x:.6} {y:.6} ", if i == 0 { 'M' } else { 'L' });
            }
            d.push_str("Z ");
        }
        d
    };
    let status = if result.passed { "PASS" } else { "FAIL" };
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 800 640\" width=\"800\" height=\"640\"><title>{}</title><rect width=\"800\" height=\"640\" fill=\"#f8fafc\"/><g font-family=\"sans-serif\" fill=\"#0f172a\"><text x=\"28\" y=\"32\" font-size=\"21\">{} · {status}</text><text x=\"28\" y=\"58\" font-size=\"12\">M0 geometry evidence · millimeters · +Y up</text></g>",
        escape(&result.fixture.description),
        escape(&result.fixture.id)
    );
    let _ = write!(
        svg,
        "<path d=\"{}\" fill=\"#e2e8f0\" fill-rule=\"evenodd\" stroke=\"#64748b\" stroke-width=\"1.5\"/>",
        path(&all)
    );
    if let Some(region) = &result.output_region {
        let _ = write!(
            svg,
            "<path d=\"{}\" fill=\"#38bdf8\" fill-opacity=\"0.22\" fill-rule=\"evenodd\" stroke=\"#0284c7\" stroke-width=\"1.5\"/>",
            path(&region.rings_mm())
        );
    }
    for preview in &result.edge_previews {
        let curved = result
            .voronoi
            .as_ref()
            .is_some_and(|v| v.edges[preview.edge].curved);
        let mut coordinates = String::new();
        for &p in &preview.linearization.points {
            let (x, y) = xy(p);
            let _ = write!(coordinates, "{x:.6},{y:.6} ");
        }
        let _ = write!(
            svg,
            "<polyline points=\"{coordinates}\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.6\"/>",
            if curved { "#d97706" } else { "#059669" }
        );
    }
    let _ = write!(
        svg,
        "<rect x=\"0\" y=\"565\" width=\"800\" height=\"75\" fill=\"#f8fafc\"/><g font-family=\"sans-serif\" font-size=\"12\" fill=\"#334155\"><text x=\"28\" y=\"586\">Gray: input · Blue: result · Green: straight Voronoi · Orange: curved Voronoi</text><text x=\"28\" y=\"608\">{}</text><text x=\"28\" y=\"627\">Finite diagram edges shown; this is not a toolpath or a filtered medial axis.</text></g></svg>",
        escape(
            &result
                .diagnostics
                .iter()
                .map(|d| d.code.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    );
    svg
}
