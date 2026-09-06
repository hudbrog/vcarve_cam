//! Authenticate the new plan and query it at the previous plan's profile
//! witnesses. The old artifact is comparison data, not trusted machining input.
use cam_core::{
    job::{Job, ToolGeometry},
    model::VBit,
    motion::{Motion, Position},
    stock::vbit_removed_depth_at,
    vcarve::{CombinedPlan, Execution, PathFamily},
};
use serde::Deserialize;
use std::{collections::HashMap, fs::File, io::BufReader};

#[derive(Deserialize)]
struct PreviousEndmill {
    job: Job,
}
#[derive(Deserialize)]
struct PreviousPlan {
    endmill: PreviousEndmill,
    executions: Vec<Execution>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: compare_plan_coverage <previous.plan.json> <new.plan.json>".into());
    }
    let previous: PreviousPlan = serde_json::from_reader(BufReader::new(File::open(&args[1])?))?;
    let plan = CombinedPlan::from_reader(BufReader::new(File::open(&args[2])?))?;
    let job = &plan.endmill.job;
    if serde_json::to_value(&previous.endmill.job)? != serde_json::to_value(job)? {
        return Err("comparison requires the same saved job".into());
    }
    let slot = job
        .tools
        .iter()
        .find(|t| t.id == job.operation.vbit_id)
        .unwrap();
    let Some(ToolGeometry::Vbit(spec)) = &slot.geometry else {
        return Err("missing V-bit".into());
    };
    let tool = VBit::try_from(spec.clone())?;
    // An independent coarse grid selects full recorded sweeps for the public
    // analytic query. This does not reuse the planner's private stock index.
    let radius = |p: Position| tool.tip_radius().mm() + p.depth() * tool.angle().slope();
    let cell = radius(Position::new(
        cam_core::geometry::Point::new(0., 0.),
        -job.operation.max_depth_mm.unwrap(),
    ))
    .max(0.1);
    let mut bins: HashMap<(i64, i64), Vec<Motion>> = HashMap::new();
    let mut entries = 0;
    for m in plan.vbit_motions.iter().filter(|m| m.kind.cutting()) {
        let r = radius(m.start).max(radius(m.end)) + 1e-9;
        let x0 = ((m.start.x.min(m.end.x) - r) / cell).floor() as i64;
        let x1 = ((m.start.x.max(m.end.x) + r) / cell).floor() as i64;
        let y0 = ((m.start.y.min(m.end.y) - r) / cell).floor() as i64;
        let y1 = ((m.start.y.max(m.end.y) + r) / cell).floor() as i64;
        for y in y0..=y1 {
            for x in x0..=x1 {
                entries += 1;
                if entries > 2_000_000 {
                    return Err("comparison grid budget exceeded".into());
                }
                bins.entry((x, y)).or_default().push(m.clone());
            }
        }
    }
    let mut samples = 0;
    let mut max_missed: f64 = 0.;
    let cap = job.operation.max_depth_mm.unwrap();
    for e in previous.executions.iter().filter(|e| {
        !e.pruned_air
            && (e.final_finish || e.candidate.family == PathFamily::Floor && e.pass_depth_mm == cap)
    }) {
        let points = &e.candidate.points;
        for i in 0..points.len() {
            let a = points[i];
            let b = points[(i + 1).min(points.len() - 1)];
            for k in 0..=4 {
                let p = a.lerp(b, k as f64 / 4.);
                let bin = bins.get(&((p.x / cell).floor() as i64, (p.y / cell).floor() as i64));
                let depth = vbit_removed_depth_at(bin.map_or(&[], Vec::as_slice), &tool, p.xy())?;
                max_missed = max_missed.max(p.depth() - depth);
                samples += 1;
            }
        }
    }
    let budget = job
        .tolerances
        .motion_tolerance_mm
        .unwrap()
        .min(job.tolerances.verification_tolerance_mm.unwrap())
        / 8.;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "new_engine_version":plan.engine_version,
            "new_plan_status":plan.analysis.status,
            "previous_profile_samples":samples,
            "max_previous_profile_missed_depth_mm":max_missed,
            "optimization_depth_budget_mm":budget,
            "within_optimization_budget": max_missed <= budget + 1e-10,
            "new_max_sampled_missed_reachable_mm":plan.analysis.max_sampled_missed_reachable_mm,
            "new_missing_floor_area_mm2":plan.analysis.missing_floor_beyond_tolerance.area_mm2(),
            "new_diagnostics":plan.analysis.diagnostics,
        }))?
    );
    if max_missed > budget + 1e-10 {
        return Err("previous profile coverage exceeds the optimization budget".into());
    }
    Ok(())
}
