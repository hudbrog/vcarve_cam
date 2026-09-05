use cam_core::preview::{ModelInput, PreviewStatus, build_preview};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

const DEMOS: &[&str] = &[
    include_str!("../../../fixtures/m1/wide_channel.json"),
    include_str!("../../../fixtures/m1/narrow_channel.json"),
    include_str!("../../../fixtures/m1/finite_tip_corner.json"),
    include_str!("../../../fixtures/m1/island.json"),
    include_str!("../../../fixtures/m1/endmill_exact_line.json"),
    include_str!("../../../fixtures/m1/endmill_exact_point.json"),
    include_str!("../../../fixtures/m1/triangle_incenter.json"),
    include_str!("../../../fixtures/m1/mixed_components.json"),
];

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

pub fn run(command: &str, args: Vec<String>) -> AppResult<bool> {
    let mut args = args.into_iter();
    let mut input = None;
    let mut output = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" if input.is_none() && command != "target-demo" => {
                input = Some(PathBuf::from(
                    args.next().ok_or("--input requires a JSON file")?,
                ))
            }
            "--output" if output.is_none() && command != "validate-model" => {
                output = Some(PathBuf::from(
                    args.next().ok_or("--output requires a directory")?,
                ))
            }
            "--help" | "-h" => {
                print!("{}", super::HELP);
                return Ok(true);
            }
            _ => return Err(format!("unknown or repeated argument {arg:?}").into()),
        }
    }
    if command == "target-demo" {
        let output = output.ok_or("--output is required")?;
        let mut models = vec![];
        for raw in DEMOS {
            let input: ModelInput = serde_json::from_str(raw)?;
            let id = input.id.clone();
            let complete = write_preview(input, &output.join(&id))?;
            eprintln!(
                "{} {id}",
                if complete { "COMPLETE" } else { "INCONCLUSIVE" }
            );
            models.push(json!({"id":id,"complete":complete,"report":format!("{id}/report.json"),"svg":format!("{id}/preview.svg")}));
        }
        let complete = models.iter().all(|m| m["complete"] == true);
        fs::write(
            output.join("report.json"),
            serde_json::to_string_pretty(
                &json!({"schema_version":1,"milestone":"M1","complete":complete,"models":models}),
            )? + "\n",
        )?;
        eprintln!(
            "{}/{} previews complete; artifacts: {}",
            models.iter().filter(|m| m["complete"] == true).count(),
            models.len(),
            output.display()
        );
        return Ok(complete);
    }
    let input: ModelInput =
        serde_json::from_str(&fs::read_to_string(input.ok_or("--input is required")?)?)?;
    if command == "validate-model" {
        return match input.validate() {
            Ok(model) => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"schema_version":1,"valid":true,"id":input.id,
                    "grid":model.target.region().grid(),"normalized_area_mm2":model.target.region().area_mm2(),
                    "endmill":model.endmill,"vbit":model.vbit,"diagnostics":model.target.region().diagnostics()})
                    )?
                );
                Ok(true)
            }
            Err(diagnostic) => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"schema_version":1,"valid":false,"diagnostics":[diagnostic]})
                    )?
                );
                Ok(false)
            }
        };
    }
    write_preview(input, &output.ok_or("--output is required")?)
}

fn write_preview(input: ModelInput, output: &Path) -> AppResult<bool> {
    fs::create_dir_all(output)?;
    fs::write(
        output.join("input.json"),
        serde_json::to_string_pretty(&input)? + "\n",
    )?;
    match build_preview(input) {
        Ok(preview) => {
            let complete = preview.status == PreviewStatus::Complete;
            fs::write(
                output.join("preview.svg"),
                super::target_svg::render(&preview),
            )?;
            let report = json!({"schema_version":1,"milestone":"M1","engine_version":env!("CARGO_PKG_VERSION"),
                "build":{"rustc":env!("CAM_RUSTC"),"target":env!("CAM_TARGET")},"preview":preview,
                "meaning":"Complete means the requested geometry preview resolution was met; it is not machining verification.",
                "limitations":["Procedural polygon inputs; SVG import and versioned jobs are M2.",
                    "Center-contact geometry has zero numerical margin and no planned entry motion.",
                    "Finite-tip bounds apply to the normalized target, modeled cone and arbitrary feasible poses; actual swept-stock verification is M5.",
                    "Profiles sample selected lines only. Floating-point reserves are engineering margins; input snapping uncertainty is recorded separately."]});
            fs::write(
                output.join("report.json"),
                serde_json::to_string_pretty(&report)? + "\n",
            )?;
            for diagnostic in &preview.diagnostics {
                eprintln!("{diagnostic}");
            }
            Ok(complete)
        }
        Err(diagnostic) => {
            fs::write(
                output.join("preview.svg"),
                super::target_svg::failure(&diagnostic.to_string()),
            )?;
            fs::write(
                output.join("report.json"),
                serde_json::to_string_pretty(
                    &json!({"schema_version":1,"milestone":"M1","valid":false,"diagnostics":[diagnostic]}),
                )? + "\n",
            )?;
            eprintln!("{diagnostic}");
            Ok(false)
        }
    }
}
