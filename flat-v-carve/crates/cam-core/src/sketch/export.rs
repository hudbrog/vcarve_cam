//! Portable SVG snapshot of verified published geometry. This uses the same
//! tessellation as CAM; it does not run a solver or serialize construction data.
use super::*;
use crate::{
    project::v5::GeometryRefKind,
    svg::{ImportOptions, MAX_SVG_BYTES},
};
use std::fmt::Write;

/// Export the fixed sketch page in millimeters, without artwork placement.
/// SVG page XY corresponds to local XY minus `frame.min` (Y is flipped by SVG).
/// Reimporting with the same setup coordinates therefore requires subtracting
/// `frame.min` from the original placement origin, exactly as materialization does.
/// Closed wires are grouped by verified region membership with even-odd fill;
/// open wires remain unfilled strokes. Curves use CAM's bounded tessellation,
/// except an isolated circle, which retains its standard SVG analytic element.
pub fn to_svg(doc: &SketchDocument, options: &ImportOptions) -> Result<String> {
    let materialized = materialize::materialize(doc, options, None)?;
    let geometry = &materialized.geometry;
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}mm\" height=\"{}mm\" viewBox=\"0 0 {} {}\">\n",
        doc.frame.width_mm, doc.frame.height_mm, doc.frame.width_mm, doc.frame.height_mm
    );
    svg.push_str("<desc>Sketch geometry snapshot. Millimeters in the fixed sketch frame; placement, construction geometry and constraints are excluded.</desc>\n");
    let path = |points: &[Point], closed: bool| {
        let mut d = String::new();
        for (i, p) in points.iter().enumerate() {
            write!(
                d,
                "{} {} {} ",
                if i == 0 { "M" } else { "L" },
                p.x,
                doc.frame.height_mm - p.y
            )
            .expect("string");
        }
        if closed {
            d.push('Z');
        }
        d
    };
    for proof in materialized
        .provenance
        .iter()
        .filter(|p| p.kind == GeometryRefKind::FilledComponent)
    {
        let wire = &proof.wires[0];
        if proof.wires.len() == 1
            && wire.1.len() == 1
            && let SketchGeometry::Circle { center, radius_mm } =
                doc.entity(&wire.1[0].entity_id)?.geometry
        {
            writeln!(
                svg,
                "<circle id=\"{}\" cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"black\"/>",
                proof.local_id,
                center.x - doc.frame.min.x,
                doc.frame.height_mm - (center.y - doc.frame.min.y),
                radius_mm
            )
            .expect("string");
        } else {
            write!(
                svg,
                "<path id=\"{}\" fill=\"black\" fill-rule=\"evenodd\" d=\"",
                proof.local_id
            )
            .expect("string");
            for (id, _, _) in &proof.wires {
                let chain = geometry
                    .chains
                    .iter()
                    .find(|c| &c.source_id == id)
                    .expect("verified wire");
                svg.push_str(&path(&chain.points, true));
                svg.push(' ');
            }
            svg.push_str("\"/>\n");
        }
        check_budget(&svg)?;
    }
    for chain in geometry.chains.iter().filter(|c| !c.closed) {
        writeln!(
            svg,
            "<path id=\"{}\" fill=\"none\" stroke=\"black\" stroke-width=\"0.1\" d=\"{}\"/>",
            chain.id,
            path(&chain.points, false)
        )
        .expect("string");
        check_budget(&svg)?;
    }
    svg.push_str("</svg>\n");
    check_budget(&svg)?;
    Ok(svg)
}

fn check_budget(svg: &str) -> Result<()> {
    if svg.len() > MAX_SVG_BYTES {
        return Err(error(
            "SKETCH_LIMIT",
            "exported SVG exceeds the source byte limit",
        ));
    }
    Ok(())
}
