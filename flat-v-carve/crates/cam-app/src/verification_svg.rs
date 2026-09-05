use cam_core::{
    job::Job,
    verification::{VerificationReport, VerificationStatus},
};
use std::fmt::Write;

pub fn render(
    job: &Job,
    report: &VerificationReport,
) -> Result<String, Box<dyn std::error::Error>> {
    let geometry = job.inspect()?.geometry;
    let bounds = report.original.domain;
    let width = bounds.max.x - bounds.min.x;
    let height = bounds.max.y - bounds.min.y;
    let scale = (840. / width).min(510. / height);
    let mut svg = String::from(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"960\" height=\"690\" viewBox=\"0 0 960 690\"><rect width=\"960\" height=\"690\" fill=\"#f8fafc\"/><g font-family=\"Arial, sans-serif\" fill=\"#0f172a\">",
    );
    write!(
        svg,
        "<text x=\"36\" y=\"42\" font-size=\"24\">M5 bounded verification: {:?}</text><text x=\"36\" y=\"70\" font-size=\"14\">Original: {:?}; {} cells, {} unresolved</text>",
        report.status,
        report.original.status,
        report.original.evaluated_cells,
        report.original.unresolved_cells
    )?;
    let rounded = report.rounded.as_ref().map_or_else(
        || "Rounded coordinates: not requested".into(),
        |r| {
            format!(
                "Rounded coordinates: {:?} ({} decimal places)",
                r.verification.status, r.decimal_places
            )
        },
    );
    write!(
        svg,
        "<text x=\"36\" y=\"94\" font-size=\"14\">{rounded}</text></g><g transform=\"translate(60 620) scale({scale} -{scale}) translate({} {})\">",
        -bounds.min.x, -bounds.min.y
    )?;
    let mut path = String::new();
    for ring in geometry.selected.rings_mm() {
        for (i, p) in ring.iter().enumerate() {
            write!(path, "{}{} {} ", if i == 0 { "M" } else { "L" }, p.x, p.y)?;
        }
        path.push('Z');
    }
    write!(
        svg,
        "<path d=\"{path}\" fill=\"#dbeafe\" fill-rule=\"evenodd\" stroke=\"#334155\" stroke-width=\"{}\"/>",
        1. / scale
    )?;
    for f in report.original.findings.iter().chain(
        report
            .rounded
            .iter()
            .flat_map(|r| r.verification.findings.iter()),
    ) {
        let color = if f.status == VerificationStatus::Failed {
            "#dc2626"
        } else {
            "#d97706"
        };
        if let Some(b) = f.cell {
            write!(
                svg,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{color}\" fill-opacity=\"0.35\" stroke=\"{color}\" stroke-width=\"{}\"/>",
                b.min.x,
                b.min.y,
                b.max.x - b.min.x,
                b.max.y - b.min.y,
                0.6 / scale
            )?;
        } else {
            write!(
                svg,
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{color}\"/>",
                f.location.x,
                f.location.y,
                3. / scale
            )?;
        }
    }
    svg.push_str("</g><text x=\"36\" y=\"655\" font-family=\"Arial, sans-serif\" font-size=\"14\" fill=\"#334155\">Red: failed witness / motion. Amber: unresolved bound. Locations and limits are in the JSON report.</text></svg>\n");
    Ok(svg)
}
