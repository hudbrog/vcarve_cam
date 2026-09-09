//! ui-8 sequence commands: open/migrate canonical jobs, plan the ordered
//! operation list, and export through the sequence pipeline.
use cam_core::{
    post::sequence::{PreparedExecution, SequenceProfile},
    project::{CamJob, migrate::migrate_legacy_json},
    sequence::{OperationPlan, PlanLimits, TrustedPlan},
};
use cam_service::sequence::{PlanScope, SequenceCommand};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

pub const HELP: &str = "Sequence (schema-4) operations\n\nUsage:\n  cam sequence open <job.json> --output <schema4-job.json>\n      Open a legacy (schema 1-3) or canonical (schema 4) job; migration preserves every value.\n  cam sequence apply-profile <job.json> --profile <legacy-profile.json> --output <schema4-job.json>\n      Move a schema-1 profile's Z datum and spindle directions into the job.\n  cam sequence plan <job.json> --output <summary.json> [--through <operation-id>]\n      Plan all enabled operations, or the enabled prefix ending at --through.\n  cam sequence export <job.json> --profile <profile.json> --output <new-directory>\n      Export the ordered program through the sequence pipeline (no M5 gate).\n";

fn read(path: &Path, limit: usize) -> AppResult<String> {
    let file = fs::File::open(path)?;
    let mut bytes = vec![];
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(format!("input exceeds {limit} bytes").into());
    }
    Ok(String::from_utf8(bytes)?)
}

fn write(path: &Path, contents: &str) -> AppResult<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

fn load_job(path: &Path) -> AppResult<CamJob> {
    let raw = read(path, 64_000_000)?;
    let version: serde_json::Value = serde_json::from_str(&raw)?;
    let job = if version.get("schema_version").and_then(|v| v.as_u64())
        == Some(cam_core::project::CAM_JOB_SCHEMA_VERSION as u64)
    {
        CamJob::from_json(&raw)?
    } else {
        migrate_legacy_json(&raw)?
    };
    Ok(job)
}

pub fn run(args: Vec<String>) -> AppResult<bool> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        print!("{HELP}");
        return Ok(true);
    };
    if !matches!(
        command.as_str(),
        "open" | "apply-profile" | "plan" | "export"
    ) {
        return Err(format!("unknown sequence command {command:?}").into());
    }
    let mut input: Option<PathBuf> = None;
    let mut output = None;
    let mut profile = None;
    let mut through = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                print!("{HELP}");
                return Ok(true);
            }
            "--output" if output.is_none() => {
                output = Some(PathBuf::from(
                    args.next().ok_or("--output requires a path")?,
                ))
            }
            "--profile" if profile.is_none() && command != "plan" => {
                profile = Some(PathBuf::from(
                    args.next().ok_or("--profile requires a file")?,
                ))
            }
            "--through" if through.is_none() && command == "plan" => {
                through = Some(args.next().ok_or("--through requires an operation ID")?)
            }
            other if !other.starts_with("--") && input.is_none() => {
                input = Some(PathBuf::from(other))
            }
            other => return Err(format!("unknown or misplaced argument {other:?}").into()),
        }
    }
    let input = input.ok_or_else(|| format!("'sequence {command}' requires a job path"))?;
    match command.as_str() {
        "open" => {
            let output = output.ok_or("'sequence open' requires --output")?;
            let job = load_job(&input)?;
            write(&output, &job.to_json()?)?;
            eprintln!(
                "operations: {}",
                job.operations
                    .iter()
                    .map(|op| format!("{}{}", if op.enabled { "" } else { " (disabled)" }, op.id))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            Ok(true)
        }
        "apply-profile" => {
            let output = output.ok_or("'sequence apply-profile' requires --output")?;
            let profile_path = profile.ok_or("'sequence apply-profile' requires --profile")?;
            let job = load_job(&input)?;
            let legacy = cam_core::post::LinuxCncProfile::from_json(&read(&profile_path, 64_000)?)?;
            let applied = cam_core::post::sequence::apply_legacy_profile(&legacy, &job)?;
            write(&output, &applied.to_json()?)?;
            eprintln!("applied profile {}", legacy.id);
            Ok(true)
        }
        "plan" => {
            let output = output.ok_or("'sequence plan' requires --output")?;
            let job = load_job(&input)?;
            let scope = match &through {
                Some(id) => PlanScope::ThroughOperation {
                    operation_id: id.clone(),
                },
                None => PlanScope::AllEnabled,
            };
            // Reuse the shared projection so CLI and adapters agree byte-for-byte.
            let result = cam_service::sequence::execute(SequenceCommand::Plan {
                job: serde_json::to_value(&job)?,
                scope,
            })?;
            write(&output, &(serde_json::to_string_pretty(&result)? + "\n"))?;
            let summary = &result["summary"];
            eprintln!(
                "motions: {} stages: {} checks: {}",
                summary["motionCount"],
                summary["stages"].as_array().map(Vec::len).unwrap_or(0),
                summary["basicChecks"]["status"].as_str().unwrap_or("?")
            );
            Ok(true)
        }
        "export" => {
            let output = output.ok_or("'sequence export' requires --output")?;
            let profile_path = profile.ok_or("'sequence export' requires --profile")?;
            let job = load_job(&input)?;
            let profile = SequenceProfile::from_json(&read(&profile_path, 64_000)?)?;
            let plan = OperationPlan::plan_job(&job, &PlanLimits::default())?;
            let trusted = TrustedPlan::from_generated(plan);
            let prepared = PreparedExecution::prepare(&trusted, &profile)?;
            let export = prepared.export(&trusted, &profile)?;
            fs::create_dir_all(&output)?;
            let program = &export.program;
            let program_path = output.join(&program.filename);
            fs::write(&program_path, &program.gcode)?;
            write(
                &output.join("sequence-export-report.json"),
                &(serde_json::to_string_pretty(&export.report)? + "\n"),
            )?;
            eprintln!(
                "program: {} motions: {} precision: {} places",
                program_path.display(),
                export.report.motion_count,
                export.report.output_decimal_places
            );
            Ok(true)
        }
        _ => unreachable!("command checked above"),
    }
}
