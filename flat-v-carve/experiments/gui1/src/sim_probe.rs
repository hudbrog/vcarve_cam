//! Headless cross-language qualification entry points; no UI or GPU required.
use crate::{
    compute,
    sim::{Field, Input, Playback},
    sim_setup,
};
use serde_json::json;
use std::path::Path;

pub fn fixture(flower: bool, path: &Path) -> Result<(), String> {
    let input = if flower {
        compute::FLOWER
    } else {
        compute::SMALL
    };
    let job = cam_core::job::Job::from_json(input).map_err(|e| e.to_string())?;
    let (plan, _) =
        cam_core::vcarve::plan_combined_with_receipt(&job).map_err(|e| e.to_string())?;
    let inspection = cam_service::inspection::Inspection::combined(&plan);
    let slices: Vec<_> = inspection.slices.into_iter().map(|s| s.info).collect();
    let setup = sim_setup::build(&job, &plan, &slices)?;
    let motions: Vec<_> = plan
        .endmill
        .motions
        .iter()
        .chain(&plan.vbit_motions)
        .collect();
    let output = json!({"inputSha256":compute::hash(input.as_bytes()),"job":job,"motions":motions,"slices":slices,"input":setup});
    std::fs::write(path, serde_json::to_vec(&output).unwrap()).map_err(|e| e.to_string())
}
pub fn run(input: &str, output: &Path) -> Result<(), String> {
    let input: Input = serde_json::from_str(input).map_err(|e| e.to_string())?;
    if input.motions.len() > compute::MAX_SEGMENTS || input.prefixes.len() > 64 {
        return Err("Simulation probe admission exceeded".into());
    }
    let field = Field::new(input.stock, &input.tools, input.resolution.cell_mm)?;
    let mut playback = Playback::new(field, 64 * 1024 * 1024);
    std::fs::create_dir_all(output).map_err(|e| e.to_string())?;
    let mut snapshots = Vec::new();
    for (i, &prefix) in input.prefixes.iter().enumerate() {
        let start = std::time::Instant::now();
        playback.seek(&input.motions, prefix)?;
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        let field = &playback.field;
        let cells = field.cell_bytes();
        std::fs::write(output.join(format!("{i}.cells")), &cells).map_err(|e| e.to_string())?;
        snapshots.push(json!({"prefix":prefix,"cols":field.cols,"rows":field.rows,"stats":field.stats,
            "checksum":field.checksum(),"cellSha256":compute::hash(&cells),"versions":field.versions,
            "fieldBytes":field.allocated_bytes(),"checkpointBytesUpperBound":playback.checkpoint_bytes(),"seekMs":elapsed}));
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&snapshots).unwrap(),
    )
    .map_err(|e| e.to_string())
}
