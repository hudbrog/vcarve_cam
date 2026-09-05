mod combined_svg;
mod job_cli;
mod job_svg;
mod plan_svg;
mod post_cli;
mod svg;
mod target_cli;
mod target_svg;
mod verification_svg;
use cam_core::spike::{Fixture, SCHEMA_VERSION, run_fixture};
use serde_json::json;
use std::{fs, path::PathBuf, process::ExitCode};

const HELP: &str = "Flat V-carve CAM — SVG jobs and target geometry\n\nUsage:\n  cam import <artwork.svg> --output <job.json> [--tolerance <mm>] [--select <region-id> ...]\n  cam inspect <job-or-plan.json> --output <preview.svg> [--report <report.json>]\n  cam select <job.json> --output <job.json> [--select <region-id> ...]\n  cam validate-job <job.json>\n  cam plan <job.json> --output <plan.json> [--stage endmill|combined]\n  cam verify <plan.json> --output <report.json> [--decimal-places <0..9>] [--preview <findings.svg>]\n      [--max-cells <count>] [--max-depth <count>] [--reachability-cells <count>] [--max-depth-bands <count>]\n  cam export <plan.json> --profile <machine.json> --output <new-directory> [--layout combined|per-tool]\n  cam verify-gcode <plan.json> --profile <machine.json> --program <program.ngc> --output <new-report.json>\n      [--layout combined|per-tool] [--program <second.ngc>] [--max-cells <count>]\n  cam geometry-spike --output <directory> [--fixture <fixture.json>]\n  cam target-demo --output <directory>\n  cam target-preview --input <model.json> --output <directory>\n  cam validate-model --input <model.json>\n\nM4 plans combined endmill/V-bit work when vbit_planning is configured.\nUse --stage endmill to generate only the roughing stage.\nInspect shows M4 planning evidence. Verify adds M5 continuous stock/error bounds for combined plans.\nM5 failed or inconclusive results exit 1; endmill-only verify retains the M3 stage contract.\nFor verify, output coordinate precision is checked when --decimal-places is supplied; export always uses profile precision.\nM6 export checks original and emitted motions and publishes a new directory with G-code and a report.\nA failed or inconclusive export publishes a report only; existing outputs are never overwritten.\nverify-gcode reads saved numeric output; it is not a general LinuxCNC interpreter.\nM0/M1 commands remain available.\n";

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
        return Ok(true);
    };
    if command == "--help" || command == "-h" {
        print!("{HELP}");
        return Ok(true);
    }
    if matches!(command.as_str(), "export" | "verify-gcode") {
        return post_cli::run(&command, args.collect());
    }
    if matches!(
        command.as_str(),
        "import" | "inspect" | "select" | "validate-job" | "plan" | "verify"
    ) {
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
