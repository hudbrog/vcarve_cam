//! Import-only scaling benchmark. Copies a single path's real curve data into
//! separated tiles, keeping tolerance and physical feature size unchanged.
use cam_core::svg::{ImportOptions, import_svg};
use std::{fs, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: benchmark_import <single-path.svg> <copies>".into());
    }
    let source = fs::read_to_string(&args[1])?;
    let count: usize = args[2].parse()?;
    if !(1..=1000).contains(&count) {
        return Err("copies must be 1..1000".into());
    }
    let doc = roxmltree::Document::parse(&source)?;
    let paths: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name("path"))
        .collect();
    if paths.len() != 1
        || paths[0]
            .ancestors()
            .any(|n| n.attribute("transform").is_some())
        || doc.root_element().attribute("width") != Some("210mm")
        || doc.root_element().attribute("height") != Some("297mm")
    {
        return Err(
            "flower benchmark expects one untransformed filled path on a 210 x 297 mm page".into(),
        );
    }
    let d = paths[0].attribute("d").ok_or("missing path data")?;
    let columns = (count as f64).sqrt().ceil() as usize;
    let rows = count.div_ceil(columns);
    let mut repeated = format!(
        "<svg width='{}mm' height='{}mm' viewBox='0 0 {} {}'>",
        210 * columns,
        297 * rows,
        210 * columns,
        297 * rows
    );
    for i in 0..count {
        repeated.push_str(&format!(
            "<path id='copy-{i}' transform='translate({} {})' fill='black' d='{d}'/>",
            i % columns * 210,
            i / columns * 297
        ));
    }
    repeated.push_str("</svg>");
    let timer = Instant::now();
    let result = import_svg(
        &repeated,
        &ImportOptions {
            geometry_tolerance_mm: 0.005,
            ..Default::default()
        },
        None,
    )?;
    let seconds = timer.elapsed().as_secs_f64();
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "engine_version": env!("CARGO_PKG_VERSION"), "copies":count, "input_bytes":repeated.len(),
            "tolerance_mm":0.005, "seconds":seconds, "sources":result.sources.len(), "components":result.components.len(),
            "boundary_vertices":result.selected.rings().iter().map(|r| r.points().len()).sum::<usize>(),
            "area_mm2":result.selected.area_mm2(), "coalescence_diagnostics":result.diagnostics.iter().filter(|d| d.code == "SNAPPED_VERTEX_COALESCED").count()
        }))?
    );
    Ok(())
}
