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

pub const HELP: &str = "Sequence (schema-4) operations\n\nUsage:\n  cam sequence open <job.json> --output <schema4-job.json>\n      Open a legacy (schema 1-3) or canonical (schema 4) job; migration preserves every value.\n  cam sequence apply-profile <job.json> --profile <legacy-profile.json> --output <schema4-job.json>\n      Move a schema-1 profile's Z datum and spindle directions into the job.\n  cam sequence plan <job.json> --output <summary.json> [--through <operation-id>]\n      Plan all enabled operations, or the enabled prefix ending at --through.\n  cam sequence export <job.json> --profile <profile.json> --output <new-directory>\n      Export the ordered program through the sequence pipeline (no M5 gate).\n  cam sequence add-operation <job.json> --kind <face|profile|drag_knife> --id <operation-id> --tool <tool-id> --output <schema4-job.json>\n      Append an unconfigured operation bound to an existing job tool (F3d).\n  cam sequence apply-knife-tool <job.json> --library <library.json> --operation <operation-id> --tool <tool-id> [--preset <preset-id>] --output <schema4-job.json>\n      Apply a drag-knife library tool and optional typed preset to one knife operation only.\n  cam sequence knife-evidence <job.json> --profile <profile.json> --output <new-directory> [--samples <n>]\n      Export, decode the emitted bytes and publish the bounded independent knife-trace report.\n";

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
        "open"
            | "apply-profile"
            | "plan"
            | "export"
            | "add-operation"
            | "apply-knife-tool"
            | "knife-evidence"
    ) {
        return Err(format!("unknown sequence command {command:?}").into());
    }
    let mut input: Option<PathBuf> = None;
    let mut output = None;
    let mut profile = None;
    let mut through = None;
    let mut kind = None;
    let mut id = None;
    let mut tool = None;
    let mut library = None;
    let mut operation = None;
    let mut preset = None;
    let mut samples: Option<usize> = None;
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
            "--profile" if profile.is_none() && !matches!(command.as_str(), "plan") => {
                profile = Some(PathBuf::from(
                    args.next().ok_or("--profile requires a file")?,
                ))
            }
            "--through" if through.is_none() && command == "plan" => {
                through = Some(args.next().ok_or("--through requires an operation ID")?)
            }
            "--kind" if kind.is_none() && command == "add-operation" => {
                kind = Some(
                    args.next()
                        .ok_or("--kind requires face|profile|drag_knife")?,
                )
            }
            "--id" if id.is_none() && command == "add-operation" => {
                id = Some(args.next().ok_or("--id requires an operation ID")?)
            }
            "--tool"
                if tool.is_none()
                    && matches!(command.as_str(), "add-operation" | "apply-knife-tool") =>
            {
                tool = Some(args.next().ok_or("--tool requires a tool ID")?)
            }
            "--library" if library.is_none() && command == "apply-knife-tool" => {
                library = Some(PathBuf::from(
                    args.next().ok_or("--library requires a file")?,
                ))
            }
            "--operation" if operation.is_none() && command == "apply-knife-tool" => {
                operation = Some(args.next().ok_or("--operation requires an operation ID")?)
            }
            "--preset" if preset.is_none() && command == "apply-knife-tool" => {
                preset = Some(args.next().ok_or("--preset requires a preset ID")?)
            }
            "--samples" if samples.is_none() && command == "knife-evidence" => {
                let value = args.next().ok_or("--samples requires a count")?;
                samples = Some(
                    value
                        .parse::<usize>()
                        .map_err(|e| format!("invalid sample count {value:?}: {e}"))?,
                );
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
        "add-operation" => {
            let output = output.ok_or("'sequence add-operation' requires --output")?;
            let kind = kind.ok_or("'sequence add-operation' requires --kind")?;
            let id = id.ok_or("'sequence add-operation' requires --id")?;
            let tool = tool.ok_or("'sequence add-operation' requires --tool")?;
            let kind = match kind.as_str() {
                "face" => cam_service::sequence::AddOperationKind::Face,
                "profile" => cam_service::sequence::AddOperationKind::Profile,
                "drag_knife" => cam_service::sequence::AddOperationKind::DragKnife,
                other => {
                    return Err(format!(
                        "unknown operation kind {other:?}; expected face, profile or drag_knife"
                    )
                    .into());
                }
            };
            let job = load_job(&input)?;
            let result = cam_service::sequence::execute(SequenceCommand::Edit {
                job: serde_json::to_value(&job)?,
                edits: vec![cam_service::sequence::OperationEdit::Add {
                    name: id.clone(),
                    id,
                    kind,
                    tool_id: tool,
                }],
            })?;
            let updated = serde_json::to_string_pretty(&result["job"])?;
            write(&output, &(updated + "\n"))?;
            eprintln!("operation added; machining values remain unset until edited");
            Ok(true)
        }
        "apply-knife-tool" => {
            let output = output.ok_or("'sequence apply-knife-tool' requires --output")?;
            let library_path = library.ok_or("'sequence apply-knife-tool' requires --library")?;
            let operation = operation.ok_or("'sequence apply-knife-tool' requires --operation")?;
            let tool = tool.ok_or("'sequence apply-knife-tool' requires --tool")?;
            let job = load_job(&input)?;
            let result = cam_service::sequence::execute(SequenceCommand::ApplyKnifeTool {
                job: serde_json::to_value(&job)?,
                library: serde_json::from_str(&read(
                    &library_path,
                    cam_core::tool_library::MAX_LIBRARY_BYTES,
                )?)?,
                operation_id: operation,
                tool_id: tool,
                preset_id: preset,
            })?;
            let updated = serde_json::to_string_pretty(&result["job"])?;
            write(&output, &(updated + "\n"))?;
            eprintln!("knife tool applied to the target assignment only");
            Ok(true)
        }
        "knife-evidence" => {
            let output = output.ok_or("'sequence knife-evidence' requires --output")?;
            let profile_path = profile.ok_or("'sequence knife-evidence' requires --profile")?;
            let job = load_job(&input)?;
            let profile = SequenceProfile::from_json(&read(&profile_path, 64_000)?)?;
            let result = cam_service::sequence::execute(SequenceCommand::KnifeEvidence {
                job: serde_json::to_value(&job)?,
                profile: serde_json::to_value(&profile)?,
                sample_limit: samples,
            })?;
            fs::create_dir_all(&output)?;
            write(
                &output.join("knife-evidence.json"),
                &(serde_json::to_string_pretty(&result["report"])? + "\n"),
            )?;
            write(
                &output.join("knife-evidence-summary.json"),
                &(serde_json::to_string_pretty(&result["summary"])? + "\n"),
            )?;
            eprintln!(
                "knife evidence: {} ({} samples, sha256 {})",
                result["report"]["status"].as_str().unwrap_or("?"),
                result["report"]["samples"]
                    .as_array()
                    .map(Vec::len)
                    .unwrap_or(0),
                result["report"]["programSha256"].as_str().unwrap_or("?"),
            );
            Ok(true)
        }
        _ => unreachable!("command checked above"),
    }
}
