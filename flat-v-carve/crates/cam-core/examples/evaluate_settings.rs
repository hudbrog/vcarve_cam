//! Replay settings variants and compare their recorded removal with one fixed,
//! fine-input target. This is a sampled sensitivity study, not a volume proof.
use cam_core::{
    geometry::Point,
    job::{Job, ToolGeometry},
    model::{Depth, Endmill, VBit},
    motion::Motion,
    stock::{removed_depth_at, vbit_removed_depth_at},
    svg::Bounds,
    target::Target,
    vcarve::CombinedPlan,
};
use serde_json::json;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    time::Instant,
};

type AnyResult<T> = Result<T, Box<dyn std::error::Error>>;

// Independent broad phase; the public analytic queries evaluate entire original
// sweeps in each bin. No planner stock cache or raster removal is reused.
struct Bins {
    cells: HashMap<(i64, i64), Vec<Motion>>,
    width: f64,
}
impl Bins {
    fn new(motions: &[Motion], radius: impl Fn(&Motion) -> f64) -> AnyResult<Self> {
        let mut cells: HashMap<_, Vec<_>> = HashMap::new();
        let mut seen = HashSet::new();
        let mut entries = 0;
        let width = 2.;
        for m in motions.iter().filter(|m| m.kind.cutting()) {
            let key = [
                m.start.x.to_bits(),
                m.start.y.to_bits(),
                m.start.z.to_bits(),
                m.end.x.to_bits(),
                m.end.y.to_bits(),
                m.end.z.to_bits(),
            ];
            if !seen.insert(key) {
                continue;
            }
            let r = radius(m) + 1e-8;
            let x0 = ((m.start.x.min(m.end.x) - r) / width).floor() as i64;
            let x1 = ((m.start.x.max(m.end.x) + r) / width).floor() as i64;
            let y0 = ((m.start.y.min(m.end.y) - r) / width).floor() as i64;
            let y1 = ((m.start.y.max(m.end.y) + r) / width).floor() as i64;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    entries += 1;
                    if entries > 2_000_000 {
                        return Err("comparison bin budget exceeded".into());
                    }
                    cells.entry((x, y)).or_default().push(m.clone());
                }
            }
        }
        Ok(Self { cells, width })
    }
    fn at(&self, p: Point) -> &[Motion] {
        self.cells
            .get(&(
                (p.x / self.width).floor() as i64,
                (p.y / self.width).floor() as i64,
            ))
            .map_or(&[], Vec::as_slice)
    }
}

fn tools(job: &Job) -> AnyResult<(Endmill, VBit)> {
    let Some(ToolGeometry::Endmill(mill)) = &job
        .tools
        .iter()
        .find(|t| t.id == job.operation.endmill_id)
        .ok_or("missing endmill")?
        .geometry
    else {
        return Err("missing endmill geometry".into());
    };
    let Some(ToolGeometry::Vbit(bit)) = &job
        .tools
        .iter()
        .find(|t| t.id == job.operation.vbit_id)
        .ok_or("missing V-bit")?
        .geometry
    else {
        return Err("missing V-bit geometry".into());
    };
    Ok((
        Endmill::try_from(mill.clone())?,
        VBit::try_from(bit.clone())?,
    ))
}

fn same_target(a: &Job, b: &Job) -> AnyResult<bool> {
    let identity = |j: &Job| {
        json!({
            "source": j.source, "placement": j.import.placement,
            "selection": j.selected_region_ids, "depth": j.operation.max_depth_mm,
            "tools": j.tools.iter().map(|t| json!({"id": t.id, "geometry": t.geometry})).collect::<Vec<_>>()
        })
    };
    Ok(identity(a) == identity(b))
}

fn main() -> AnyResult<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 4 {
        return Err("usage: evaluate_settings <reference.plan.json> <new-output-dir> <variant.plan.json> [...]".into());
    }
    fs::create_dir(&args[2])?;
    let reference = CombinedPlan::from_json(&fs::read_to_string(&args[1])?)?;
    let (mill, bit) = tools(&reference.endmill.job)?;
    if bit.tip_radius().mm() != 0. {
        return Err("study requires the pointed reference V-bit".into());
    }
    let geometry = reference.endmill.job.inspect()?.geometry;
    let target = Target::new(
        geometry.selected,
        Depth::new(
            reference
                .endmill
                .job
                .operation
                .max_depth_mm
                .ok_or("missing depth")?,
        )?,
        bit.angle(),
    )?;
    let bounds = Bounds::of(target.region()).ok_or("empty target")?;
    let mut points = vec![];
    let mut seen = BTreeSet::new();
    let mut add = |p: Point| -> AnyResult<()> {
        let key = target.region().grid().quantize(p)?;
        if seen.insert((key.x, key.y)) {
            points.push((p, target.nominal_depth(p)?.mm()));
        }
        Ok(())
    };
    // Fixed lattice includes the surrounding uncut material for overcut samples.
    let spacing = 0.25;
    let nx = ((bounds.max.x - bounds.min.x + 2.) / spacing).ceil() as usize;
    let ny = ((bounds.max.y - bounds.min.y + 2.) / spacing).ceil() as usize;
    if nx * ny > 2_000_000 {
        return Err("comparison sample budget exceeded".into());
    }
    for y in 0..=ny {
        for x in 0..=nx {
            add(Point::new(
                bounds.min.x - 1. + x as f64 * spacing,
                bounds.min.y - 1. + y as f64 * spacing,
            ))?;
        }
    }
    for s in &reference.analysis.samples {
        add(s.point)?;
    }
    // Fixed reference cut witnesses also cover thin details between grid points.
    for m in reference.vbit_motions.iter().filter(|m| m.kind.cutting()) {
        let n = ((m.end.x - m.start.x).hypot(m.end.y - m.start.y) / 0.1)
            .ceil()
            .max(2.) as usize;
        for k in 0..=n {
            add(m.start.lerp(m.end, k as f64 / n as f64).xy())?;
        }
    }
    eprintln!("{} fixed reference samples", points.len());
    for file in &args[3..] {
        let timer = Instant::now();
        let plan = CombinedPlan::from_json(&fs::read_to_string(file)?)?;
        let replay_seconds = timer.elapsed().as_secs_f64();
        if !same_target(&reference.endmill.job, &plan.endmill.job)? {
            return Err(
                "source, placement, depth, selection and cutter geometry must match".into(),
            );
        }
        let candidate_geometry = plan.endmill.job.inspect()?.geometry;
        let vertices: usize = candidate_geometry
            .selected
            .rings()
            .iter()
            .map(|r| r.points().len())
            .sum();
        let candidate_target =
            Target::new(candidate_geometry.selected, target.depth_cap(), bit.angle())?;
        let endmill_bins = Bins::new(&plan.endmill.motions, |_| mill.radius().mm())?;
        let bit_bins = Bins::new(&plan.vbit_motions, |m| {
            bit.tip_radius().mm() + m.start.depth().max(m.end.depth()) * bit.angle().slope()
        })?;
        let mut residuals = vec![];
        let mut max_under: f64 = 0.;
        let mut max_over: f64 = 0.;
        let mut max_floor: f64 = 0.;
        let mut max_wall: f64 = 0.;
        let mut max_geometry: f64 = 0.;
        let mut worst_under = Point::new(0., 0.);
        let mut above_01 = 0;
        for &(p, nominal) in &points {
            let removed = removed_depth_at(endmill_bins.at(p), mill.radius().mm(), p)?
                .max(vbit_removed_depth_at(bit_bins.at(p), &bit, p)?);
            let residual = (nominal - removed).max(0.);
            if residual > max_under {
                max_under = residual;
                worst_under = p;
            }
            max_over = max_over.max(removed - nominal);
            max_geometry =
                max_geometry.max((candidate_target.nominal_depth(p)?.mm() - nominal).abs());
            if nominal > 0. {
                residuals.push(residual);
                above_01 += usize::from(residual > 0.1 + 1e-10);
                if nominal == target.depth_cap().mm() {
                    max_floor = max_floor.max(residual);
                } else {
                    max_wall = max_wall.max(residual);
                }
            }
        }
        residuals.sort_by(f64::total_cmp);
        let quantile = |q: f64| residuals[((residuals.len() - 1) as f64 * q).round() as usize];
        // Extra variant witnesses are reported separately, keeping the primary
        // comparison set exactly identical between every variant.
        let mut own_max_under: f64 = 0.;
        let mut own_max_over: f64 = 0.;
        for s in &plan.analysis.samples {
            let nominal = target.nominal_depth(s.point)?.mm();
            own_max_under = own_max_under.max(nominal - s.removed_mm);
            own_max_over = own_max_over.max(s.removed_mm - nominal);
        }
        let report = json!({
            "file": file, "engine_version": plan.engine_version, "replay_seconds": replay_seconds,
            "evaluation_seconds_including_replay": timer.elapsed().as_secs_f64(),
            "geometry_vertices": vertices, "flattening_bound_mm": candidate_geometry.flattening_bound_mm,
            "source_snap_bound_mm": candidate_geometry.source_snap_bound_mm,
            "status": plan.analysis.status, "diagnostics": plan.analysis.diagnostics,
            "endmill_status": plan.endmill.analysis.status,
            "endmill_generation_issues": plan.endmill.generation_issues,
            "generation_issues": plan.generation_issues,
            "quality_samples": plan.analysis.samples.len(),
            "max_own_target_sampled_residual_mm": plan.analysis.max_sampled_residual_mm,
            "missing_floor_beyond_tolerance_mm2": plan.analysis.missing_floor_beyond_tolerance.area_mm2(),
            "max_slice_possible_overcut_mm2": plan.analysis.slices.iter().map(|s|s.possible_overcut.area_mm2()).fold(0.,f64::max),
            "finish_expected": plan.analysis.finish_paths_expected, "finish_executed": plan.analysis.finish_paths_executed,
            "fixed_reference_samples": points.len(), "fixed_reference_material_samples": residuals.len(),
            "fixed_grid_spacing_mm": spacing,
            "max_reference_residual_mm": max_under, "max_reference_excess_depth_mm": max_over,
            "max_reference_floor_residual_mm": max_floor, "max_reference_wall_residual_mm": max_wall,
            "reference_residual_p95_mm": quantile(0.95), "reference_residual_p99_mm": quantile(0.99),
            "reference_samples_above_01_mm": above_01, "worst_reference_residual_point": worst_under,
            "max_sampled_target_change_mm": max_geometry,
            "own_witness_max_reference_residual_mm": own_max_under,
            "own_witness_max_reference_excess_depth_mm": own_max_over,
            "limitation": "Sampled depth errors against the original 0.001 mm input target; not a global error bound, M5 volume proof, or physical machine accuracy guarantee."
        });
        let id = std::path::Path::new(file)
            .parent()
            .and_then(|p| p.file_name())
            .ok_or("missing variant name")?
            .to_string_lossy();
        fs::write(
            std::path::Path::new(&args[2]).join(format!("{id}.json")),
            serde_json::to_string_pretty(&report)?,
        )?;
        println!(
            "{}",
            json!({"id": id, "status": plan.analysis.status, "max_reference_residual_mm": max_under, "max_reference_excess_depth_mm": max_over, "seconds": timer.elapsed().as_secs_f64()})
        );
    }
    Ok(())
}
