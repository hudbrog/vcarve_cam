//! Diagnostic reconstruction of stock from saved motions; does not authenticate
//! the plan or rerun planning. Reports geometry hashes as well as elapsed time.
use cam_core::{
    job::{Job, ToolGeometry},
    model::{Depth, VBit},
    motion::Motion,
    stock::vbit_removal_at_slice,
    target::Target,
};
use sha2::{Digest, Sha256};
use std::{fs, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err(
            "usage: benchmark_vbit_slices <plan.json> <output-directory> <comma-separated-depths>"
                .into(),
        );
    }
    let input = fs::read(&args[1])?;
    let input_hash = format!("{:x}", Sha256::digest(&input));
    let value: serde_json::Value = serde_json::from_slice(&input)?;
    let job = Job::from_json(&serde_json::to_string(&value["endmill"]["job"])?)?;
    let geometry = job.inspect()?.geometry;
    let settings = job
        .tools
        .iter()
        .find(|t| t.id == job.operation.vbit_id)
        .ok_or("missing V-bit")?;
    let Some(ToolGeometry::Vbit(spec)) = &settings.geometry else {
        return Err("V-bit required".into());
    };
    let tool = VBit::try_from(spec.clone())?;
    let target = Target::for_planning(
        geometry.selected,
        Depth::new(job.operation.max_depth_mm.ok_or("missing depth cap")?)?,
        tool.angle(),
    )?;
    let moves: Vec<Motion> = serde_json::from_value(value["vbit_motions"].clone())?;
    fs::create_dir_all(&args[2])?;
    for depth in args[3].split(',') {
        let depth: f64 = depth.parse()?;
        let start = Instant::now();
        let result = vbit_removal_at_slice(target.region().grid(), &moves, &tool, depth)?;
        let seconds = start.elapsed().as_secs_f64();
        let data = serde_json::to_vec(&result)?;
        let report = serde_json::json!({
            "depth_mm": depth, "seconds": seconds, "plan_sha256": input_hash,
            "grid": target.region().grid(), "slice_sha256": format!("{:x}", Sha256::digest(&data)),
            "lower_area_mm2": result.lower.area_mm2(), "upper_area_mm2": result.upper.area_mm2(),
            "lower_vertices": result.lower.rings().iter().map(|r| r.points().len()).sum::<usize>(),
            "upper_vertices": result.upper.rings().iter().map(|r| r.points().len()).sum::<usize>(),
            "contributing_motions": result.contributing_motion_ids.len(),
            "radial_error_mm": result.capsule_radial_error_mm,
        });
        println!("{report}");
        fs::write(format!("{}/slice-{depth}.json", args[2]), data)?;
        fs::write(
            format!("{}/summary-{depth}.json", args[2]),
            serde_json::to_vec_pretty(&report)?,
        )?;
    }
    Ok(())
}
