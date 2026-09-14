mod collection_cli;
mod job_cli;
mod svg;
mod target_cli;
mod target_svg;
use cam_core::spike::{Fixture, SCHEMA_VERSION, run_fixture};
use serde_json::json;
use std::{fs, path::PathBuf, process::ExitCode};

const HELP: &str = "Flat V-carve CAM — schema-5 job documents and target geometry\n\nUsage:\n  cam import <artwork.svg> --output <job.json> [--tolerance <mm>]\n  cam inspect <job.json> --output <inspection.json>\n  cam collection open|inspect|plan|export|... (see `cam collection --help`)\n  cam geometry-spike --output <directory> [--fixture <fixture.json>]\n  cam target-demo --output <directory>\n  cam target-preview --input <model.json> --output <directory>\n  cam validate-model --input <model.json>\n\nThere is one job document: schema 5. An older document is refused by name,\nnever converted (docs/flat-v-carve/schema-diet-plan.md). Planning, selection,\nmachine configuration and export live on the collection surface, which the\nworkspace and the browser build use too.\nM0/M1 target geometry commands remain available.\n";

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("CAM_ERROR: {error}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        print!("{HELP}");
        print!("\n{}", cam_server::serve::HELP);
        return Ok(true);
    };
    if command == "--help" || command == "-h" {
        print!("{HELP}");
        print!("\n{}", cam_server::serve::HELP);
        return Ok(true);
    }
    if command == "serve" {
        cam_server::serve::run(args)?;
        return Ok(true);
    }
    if command == "collection" {
        return collection_cli::run(args.collect());
    }
    if matches!(command.as_str(), "import" | "inspect") {
        return job_cli::run(&command, args.collect());
    }
    if matches!(
        command.as_str(),
        "target-demo" | "target-preview" | "validate-model"
    ) {
        return target_cli::run(&command, args.collect());
    }
    if command != "geometry-spike" {
        return Err(format!("unknown command {command:?}; use --help").into());
    }
    let mut output = None;
    let mut fixture_path = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                print!("{HELP}");
                return Ok(true);
            }
            "--output" if output.is_none() => {
                output = Some(PathBuf::from(
                    args.next().ok_or("--output requires a directory")?,
                ))
            }
            "--fixture" if fixture_path.is_none() => {
                fixture_path = Some(PathBuf::from(
                    args.next().ok_or("--fixture requires a file")?,
                ))
            }
            _ => return Err(format!("unknown or repeated argument {arg:?}").into()),
        }
    }
    let output = output.ok_or("--output is required")?;
    let fixtures: Vec<Fixture> = match fixture_path {
        Some(path) => vec![serde_json::from_str(&fs::read_to_string(path)?)?],
        None => serde_json::from_str(include_str!("../../../fixtures/m0.json"))?,
    };
    let mut ids = std::collections::HashSet::new();
    for f in &fixtures {
        if f.id.is_empty()
            || !f
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || !ids.insert(&f.id)
        {
            return Err(
                "fixture IDs must be unique ASCII letters, digits, underscores or hyphens".into(),
            );
        }
    }
    fs::create_dir_all(output.join("repro"))?;
    let mut results = vec![];
    for fixture in fixtures {
        let result = run_fixture(fixture);
        let id = &result.fixture.id;
        fs::write(
            output.join("repro").join(format!("{id}.json")),
            serde_json::to_string_pretty(&result.fixture)? + "\n",
        )?;
        fs::write(
            output.join(format!("{id}.json")),
            serde_json::to_string_pretty(&result)? + "\n",
        )?;
        fs::write(output.join(format!("{id}.svg")), svg::render(&result))?;
        eprintln!("{} {id}", if result.passed { "PASS" } else { "FAIL" });
        if !result.passed {
            for d in &result.diagnostics {
                eprintln!("  {d}")
            }
            for m in result.measurements.iter().filter(|m| !m.passed) {
                eprintln!(
                    "  {}: {} expected {} +/- {}",
                    m.name, m.measured, m.expected, m.tolerance
                )
            }
        }
        results.push(result);
    }
    let passed = results.iter().filter(|r| r.passed).count();
    let report = json!({
        "schema_version":SCHEMA_VERSION,
        "milestone":"M0",
        "engine_version":env!("CARGO_PKG_VERSION"),
        "build":{"rustc":env!("CAM_RUSTC"),"target":env!("CAM_TARGET")},
        "dependencies":{"clipper2-rust":"1.1.0","boostvoronoi":"0.12.1","robust":"1.2.0","geometry_default_features":false},
        "summary":{"total":results.len(),"passed":passed,"failed":results.len()-passed},
        "limitations":["Geometry capability evidence, not a machining verification report.","Full Voronoi diagram; interior medial-axis extraction is M4.","Empty polygon offsets do not exclude exact-fit centerlines or points.","Distance residuals are sampled fixture measurements. The quadratic chord formula is an analytic bound; floating-point reserves are engineering margins, not interval arithmetic."],
        "fixtures":results
    });
    fs::write(
        output.join("report.json"),
        serde_json::to_string_pretty(&report)? + "\n",
    )?;
    eprintln!(
        "{passed}/{} fixtures passed; artifacts: {}",
        results.len(),
        output.display()
    );
    Ok(passed == results.len())
}
