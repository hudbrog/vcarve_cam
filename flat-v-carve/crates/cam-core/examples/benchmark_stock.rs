//! Isolate stock polygon costs from an existing combined motion artifact.
//! This benchmark deliberately does not authenticate a plan or certify machining.
use cam_core::{
    geometry::BooleanOp,
    job::{Job, ToolGeometry},
    model::{Depth, Endmill, VBit},
    motion::Motion,
    stock::{removal_at_slice, vbit_removal_at_slice},
    target::Target,
};
use serde::Deserialize;
use std::{fs, time::Instant};

#[derive(Deserialize)]
struct EndmillInput {
    job: Job,
    motions: Vec<Motion>,
}
#[derive(Deserialize)]
struct Input {
    endmill: EndmillInput,
    vbit_motions: Vec<Motion>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: benchmark_stock <combined-plan.json> <depth-mm> (timing only, no authentication)".into());
    }
    let input: Input = serde_json::from_str(&fs::read_to_string(&args[1])?)?;
    let depth: f64 = args[2].parse()?;
    let job = &input.endmill.job;
    let region = job.inspect()?.geometry.selected;
    let mill = job
        .tools
        .iter()
        .find(|t| t.id == job.operation.endmill_id)
        .unwrap();
    let bit = job
        .tools
        .iter()
        .find(|t| t.id == job.operation.vbit_id)
        .unwrap();
    let Some(ToolGeometry::Endmill(spec)) = &mill.geometry else {
        return Err("endmill required".into());
    };
    let mill = Endmill::try_from(spec.clone())?;
    let Some(ToolGeometry::Vbit(spec)) = &bit.geometry else {
        return Err("V-bit required".into());
    };
    let bit = VBit::try_from(spec.clone())?;
    let target = Target::for_planning(
        region,
        Depth::new(job.operation.max_depth_mm.ok_or("depth required")?)?,
        bit.angle(),
    )?;
    let mut timer = Instant::now();
    let mut lap = |label: &str| {
        eprintln!("{label}: {:.3} s", timer.elapsed().as_secs_f64());
        timer = Instant::now();
    };
    let e = removal_at_slice(
        target.region().grid(),
        &input.endmill.motions,
        mill.radius().mm(),
        depth,
    )?;
    lap("endmill stock");
    let v = vbit_removal_at_slice(target.region().grid(), &input.vbit_motions, &bit, depth)?;
    lap("vbit stock");
    let lower = e.lower.boolean(BooleanOp::Union, &v.lower)?;
    lap("lower union");
    let upper = e.upper.boolean(BooleanOp::Union, &v.upper)?;
    lap("upper union");
    let nominal = target.region().erode(depth * bit.angle().slope())?;
    lap("nominal section");
    let remaining = nominal.boolean(BooleanOp::Difference, &lower)?;
    lap("remaining");
    let overcut = upper.boolean(BooleanOp::Difference, &nominal)?;
    lap("overcut");
    for component in overcut.components() {
        let points: Vec<_> = component.rings_mm().into_iter().flatten().collect();
        let mut min = points[0];
        let mut max = points[0];
        let mut clearance = f64::INFINITY;
        for &p in &points {
            min.x = min.x.min(p.x);
            min.y = min.y.min(p.y);
            max.x = max.x.max(p.x);
            max.y = max.y.max(p.y);
            clearance = clearance
                .min(target.boundary().sample(p)?.signed_distance_mm - depth * bit.angle().slope());
        }
        let center = min.lerp(max, 0.5);
        let disk_margin = target.boundary().sample(center)?.signed_distance_mm
            - depth * bit.angle().slope()
            - min.distance(max) / 2.;
        eprintln!(
            "overcut component: {}",
            serde_json::json!({"area_mm2": component.area_mm2(), "points": points, "min_clearance": clearance, "disk_margin": disk_margin})
        );
    }
    println!(
        "{}",
        serde_json::json!({"depth_mm": depth, "lower_area_mm2": lower.area_mm2(), "upper_area_mm2": upper.area_mm2(), "remaining_area_mm2": remaining.area_mm2(), "overcut_area_mm2": overcut.area_mm2(), "endmill_contributors": e.contributing_motion_ids.len(), "vbit_contributors": v.contributing_motion_ids.len()})
    );
    Ok(())
}
