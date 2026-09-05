use cam_core::{job::Job, pocket::plan_endmill, vcarve::plan_combined};
use std::{collections::BTreeMap, fs, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: benchmark_pipeline <job.json> <plan.json>".into());
    }
    let timer = Instant::now();
    let job = Job::from_json(&fs::read_to_string(&args[1])?)?;
    eprintln!("Job loaded: {:.3} s", timer.elapsed().as_secs_f64());
    let timer = Instant::now();
    let endmill = plan_endmill(&job)?;
    let endmill_seconds = timer.elapsed().as_secs_f64();
    eprintln!(
        "Endmill: {:.3} s, {} motions, {:?}",
        timer.elapsed().as_secs_f64(),
        endmill.motions.len(),
        endmill.analysis.status
    );
    let timer = Instant::now();
    let plan = plan_combined(&job)?;
    let combined_seconds = timer.elapsed().as_secs_f64();
    eprintln!(
        "Combined (including endmill): {:.3} s, {} V-bit motions, {:?}",
        timer.elapsed().as_secs_f64(),
        plan.vbit_motions.len(),
        plan.analysis.status
    );
    let mut families = BTreeMap::<String, [usize; 4]>::new();
    for execution in &plan.executions {
        let key = format!(
            "{:?}/{}",
            execution.candidate.family,
            if execution.final_finish {
                "finish"
            } else {
                "depth_pass"
            }
        );
        let counts = families.entry(key).or_default();
        counts[0] += 1;
        counts[1] += execution.candidate.points.len();
        counts[2] += execution.end_motion_id - execution.first_motion_id;
        counts[3] += usize::from(execution.pruned_air);
    }
    let json = plan.to_json()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "engine_version": env!("CARGO_PKG_VERSION"),
            "endmill_seconds": endmill_seconds,
            "combined_seconds_including_endmill": combined_seconds,
            "endmill_motions": plan.endmill.motions.len(),
            "endmill_status": plan.endmill.analysis.status,
            "endmill_layers": plan.endmill.analysis.layers.iter().map(|layer| serde_json::json!({
                "depth_mm": layer.depth_mm, "status": layer.status,
                "missing_floor_area_mm2": layer.missing_floor_beyond_tolerance.area_mm2(),
                "diagnostics": layer.diagnostics
            })).collect::<Vec<_>>(),
            "vbit_motions": plan.vbit_motions.len(),
            "combined_status": plan.analysis.status,
            "diagnostics": plan.analysis.diagnostics,
            "generation_issues": plan.generation_issues,
            "finish_paths_expected": plan.analysis.finish_paths_expected,
            "finish_paths_executed": plan.analysis.finish_paths_executed,
            "max_sampled_missed_reachable_mm": plan.analysis.max_sampled_missed_reachable_mm,
            "quality_samples": plan.analysis.samples.len(),
            "missing_floor_area_mm2": plan.analysis.missing_floor_beyond_tolerance.area_mm2(),
            "medial_branches": plan.analysis.medial_axis.branches.len(),
            "family_count_columns": ["executions", "candidate_points", "motions", "pruned_air"],
            "families": families,
            "plan_bytes": json.len()
        }))?
    );
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[2])?;
    std::io::Write::write_all(&mut output, json.as_bytes())?;
    Ok(())
}
