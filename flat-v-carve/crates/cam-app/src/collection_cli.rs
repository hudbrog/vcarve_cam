//! ui-9 collection commands (H4/H5): open/migrate schema-5 documents,
//! resource commands, the applied machine configuration, scope-aware
//! planning — and retained export: one generation, one preparation, the
//! ordered output files written with their manifest and report — all
//! through the shared service projection so CLI and adapters agree.
use cam_core::post::sequence::OutputLayout;
use cam_core::project::v5::resources::AssignmentRole;
use cam_service::collection::{CollectionCommand, CollectionScope};
use cam_service::retained::Retained;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

pub const HELP: &str = "Collection (schema-5) operations\n\nUsage:\n  cam collection open <job.json> --output <schema5-job.json>\n      Open any supported job (schema 1-5); older schemas migrate once into the collection model.\n  cam collection inspect <job.json> --output <inspection.json>\n      Read-only inspection: artwork tree, used-by index, assignments, machine readout.\n  cam collection plan <job.json> --output <summary.json> [--through <operation-id>]\n      Plan all enabled collection operations, or the enabled prefix ending at --through.\n  cam collection apply-profile <job.json> --library <library.json> --library-id <id> --operation <operation-id> --role <endmill|vbit|milling|knife> --tool <library-tool-id> --preset <preset-id> --output <schema5-job.json>\n      Copy one named cutting profile into exactly one assignment; unset preset fields copy as unset.\n  cam collection reset <job.json> --operation <operation-id> --role <role> --output <schema5-job.json>\n      Restore one assignment's copied baseline without any library file.\n  cam collection reapply <job.json> --library <library.json> --library-id <id> --operation <operation-id> --role <role> --output <schema5-job.json>\n      Reapply the stored provenance from the supplied current library revision.\n  cam collection apply-tool <job.json> --library <library.json> --library-id <id> --operation <operation-id> --role <role> --tool <library-tool-id> --output <schema5-job.json>\n      Bind a library tool's geometry to one assignment with copied provenance.\n  cam collection apply-machine <job.json> --profile <profile.json> --name <configuration-name> --output <schema5-job.json>\n      Copy a reusable configuration into the job's one applied machine snapshot.\n  cam collection set-mapping <job.json> --tool <job-tool-id> [--number <T>] [--length <H>] --output <schema5-job.json>\n      Set one job tool's mapping exactly; giving neither number removes the row.\n  cam collection resolve-profile <job.json> --output <profile.json> [--through <operation-id>]\n      Resolve the applied snapshot into the validated schema-2 profile for a scope.\n  cam collection knife-evidence <job.json> --output <new-directory> [--samples <n>] [--through <operation-id>]\n      Export through the applied machine configuration and publish the bounded independent knife-trace report.\n  cam collection export <job.json> --output <new-directory> [--through <operation-id>] [--layout one|sequential]\n      Retained export: plan once, prepare the ordered output and write every checked file with its manifest and report (default layout: sequential files; --layout one writes the single sequence.ngc).\n";

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

fn load_job_value(path: &Path) -> AppResult<serde_json::Value> {
    Ok(serde_json::from_str(&read(
        path,
        cam_service::document::JOB_BYTES,
    )?)?)
}

fn parse_role(value: &str) -> AppResult<AssignmentRole> {
    match value {
        "endmill" => Ok(AssignmentRole::Endmill),
        "vbit" => Ok(AssignmentRole::Vbit),
        "milling" => Ok(AssignmentRole::Milling),
        "knife" => Ok(AssignmentRole::Knife),
        other => Err(format!(
            "unknown assignment role {other:?}; expected endmill, vbit, milling or knife"
        )
        .into()),
    }
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
            | "inspect"
            | "plan"
            | "apply-profile"
            | "reset"
            | "reapply"
            | "apply-tool"
            | "apply-machine"
            | "set-mapping"
            | "resolve-profile"
            | "knife-evidence"
            | "export"
    ) {
        return Err(format!("unknown collection command {command:?}").into());
    }
    let mut input: Option<PathBuf> = None;
    let mut output = None;
    let mut profile = None;
    let mut through = None;
    let mut library = None;
    let mut library_id = None;
    let mut operation = None;
    let mut role = None;
    let mut tool = None;
    let mut preset = None;
    let mut name = None;
    let mut number: Option<u32> = None;
    let mut length: Option<u32> = None;
    let mut samples: Option<usize> = None;
    let mut layout: Option<String> = None;
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
            "--profile" if profile.is_none() && command == "apply-machine" => {
                profile = Some(PathBuf::from(
                    args.next().ok_or("--profile requires a file")?,
                ))
            }
            "--through"
                if through.is_none()
                    && matches!(
                        command.as_str(),
                        "plan" | "resolve-profile" | "knife-evidence" | "export"
                    ) =>
            {
                through = Some(args.next().ok_or("--through requires an operation ID")?)
            }
            "--layout" if layout.is_none() && command == "export" => {
                layout = Some(args.next().ok_or("--layout requires one or sequential")?)
            }
            "--library"
                if library.is_none()
                    && matches!(command.as_str(), "apply-profile" | "reapply" | "apply-tool") =>
            {
                library = Some(PathBuf::from(
                    args.next().ok_or("--library requires a file")?,
                ))
            }
            "--library-id"
                if library_id.is_none()
                    && matches!(command.as_str(), "apply-profile" | "reapply" | "apply-tool") =>
            {
                library_id = Some(args.next().ok_or("--library-id requires an ID")?)
            }
            "--operation"
                if operation.is_none()
                    && matches!(
                        command.as_str(),
                        "apply-profile" | "reset" | "reapply" | "apply-tool"
                    ) =>
            {
                operation = Some(args.next().ok_or("--operation requires an operation ID")?)
            }
            "--role"
                if role.is_none()
                    && matches!(
                        command.as_str(),
                        "apply-profile" | "reset" | "reapply" | "apply-tool"
                    ) =>
            {
                role = Some(parse_role(&args.next().ok_or("--role requires a role")?)?)
            }
            "--tool"
                if tool.is_none()
                    && matches!(
                        command.as_str(),
                        "apply-profile" | "apply-tool" | "set-mapping"
                    ) =>
            {
                tool = Some(args.next().ok_or("--tool requires a tool ID")?)
            }
            "--preset" if preset.is_none() && command == "apply-profile" => {
                preset = Some(args.next().ok_or("--preset requires a preset ID")?)
            }
            "--name" if name.is_none() && command == "apply-machine" => {
                name = Some(args.next().ok_or("--name requires a configuration name")?)
            }
            "--number" if number.is_none() && command == "set-mapping" => {
                let value = args.next().ok_or("--number requires a T number")?;
                number = Some(
                    value
                        .parse::<u32>()
                        .map_err(|e| format!("invalid tool number {value:?}: {e}"))?,
                );
            }
            "--length" if length.is_none() && command == "set-mapping" => {
                let value = args.next().ok_or("--length requires an H number")?;
                length = Some(
                    value
                        .parse::<u32>()
                        .map_err(|e| format!("invalid length offset number {value:?}: {e}"))?,
                );
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
    let input = input.ok_or_else(|| format!("'collection {command}' requires a job path"))?;
    let scope = |through: Option<String>| match through {
        Some(operation_id) => CollectionScope::ThroughOperation { operation_id },
        None => CollectionScope::AllEnabled,
    };
    match command.as_str() {
        "open" => {
            let output = output.ok_or("'collection open' requires --output")?;
            let result = cam_service::collection::execute(CollectionCommand::Open {
                json: read(&input, cam_service::document::JOB_BYTES)?,
            })?;
            let job = serde_json::to_string_pretty(&result["job"])?;
            write(&output, &(job + "\n"))?;
            eprintln!(
                "operations: {} (migrated: {})",
                result["inspection"]["machiningOrder"]
                    .as_array()
                    .map(Vec::len)
                    .unwrap_or(0),
                result["migrated"].as_bool().unwrap_or(false)
            );
            Ok(true)
        }
        "inspect" => {
            let output = output.ok_or("'collection inspect' requires --output")?;
            let result = cam_service::collection::execute(CollectionCommand::Inspect {
                job: load_job_value(&input)?,
            })?;
            write(&output, &(serde_json::to_string_pretty(&result)? + "\n"))?;
            eprintln!(
                "artwork items: {} assignments: {} mapping rows: {}",
                result["artwork"].as_array().map(Vec::len).unwrap_or(0),
                result["inspection"]["assignments"]
                    .as_array()
                    .map(Vec::len)
                    .unwrap_or(0),
                result["inspection"]["machine"]["rows"]
                    .as_array()
                    .map(Vec::len)
                    .unwrap_or(0),
            );
            Ok(true)
        }
        "plan" => {
            let output = output.ok_or("'collection plan' requires --output")?;
            let result = cam_service::collection::execute(CollectionCommand::Plan {
                job: load_job_value(&input)?,
                scope: scope(through),
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
        "apply-profile" | "reset" | "reapply" | "apply-tool" => {
            let output =
                output.ok_or_else(|| format!("'collection {command}' requires --output"))?;
            let operation =
                operation.ok_or_else(|| format!("'collection {command}' requires --operation"))?;
            let role = role.ok_or_else(|| format!("'collection {command}' requires --role"))?;
            let job = load_job_value(&input)?;
            let operation_label = operation.clone();
            let result = match command.as_str() {
                "apply-profile" => {
                    let library_path =
                        library.ok_or("'collection apply-profile' requires --library")?;
                    let library_id =
                        library_id.ok_or("'collection apply-profile' requires --library-id")?;
                    let tool = tool.ok_or("'collection apply-profile' requires --tool")?;
                    let preset = preset.ok_or("'collection apply-profile' requires --preset")?;
                    cam_service::collection::execute(CollectionCommand::ApplyCuttingProfile {
                        job,
                        library: serde_json::from_str(&read(
                            &library_path,
                            cam_core::tool_library::MAX_LIBRARY_BYTES,
                        )?)?,
                        library_id,
                        operation_id: operation,
                        role,
                        library_tool_id: tool,
                        preset_id: preset,
                    })?
                }
                "reset" => cam_service::collection::execute(CollectionCommand::ResetAssignment {
                    job,
                    operation_id: operation,
                    role,
                })?,
                "reapply" => {
                    let library_path = library.ok_or("'collection reapply' requires --library")?;
                    let library_id =
                        library_id.ok_or("'collection reapply' requires --library-id")?;
                    cam_service::collection::execute(CollectionCommand::ReapplyProfile {
                        job,
                        library: serde_json::from_str(&read(
                            &library_path,
                            cam_core::tool_library::MAX_LIBRARY_BYTES,
                        )?)?,
                        library_id,
                        operation_id: operation,
                        role,
                    })?
                }
                _ => {
                    let library_path =
                        library.ok_or("'collection apply-tool' requires --library")?;
                    let library_id =
                        library_id.ok_or("'collection apply-tool' requires --library-id")?;
                    let tool = tool.ok_or("'collection apply-tool' requires --tool")?;
                    cam_service::collection::execute(CollectionCommand::ApplyTool {
                        job,
                        library: serde_json::from_str(&read(
                            &library_path,
                            cam_core::tool_library::MAX_LIBRARY_BYTES,
                        )?)?,
                        library_id,
                        operation_id: operation,
                        role,
                        library_tool_id: tool,
                    })?
                }
            };
            let updated = serde_json::to_string_pretty(&result["job"])?;
            write(&output, &(updated + "\n"))?;
            eprintln!(
                "{} applied to operation '{}' ({}); the job keeps no live library dependency",
                command,
                operation_label,
                serde_json::to_string(&role)?
            );
            Ok(true)
        }
        "apply-machine" => {
            let output = output.ok_or("'collection apply-machine' requires --output")?;
            let profile_path = profile.ok_or("'collection apply-machine' requires --profile")?;
            let name = name.ok_or("'collection apply-machine' requires --name")?;
            let result =
                cam_service::collection::execute(CollectionCommand::ApplyMachineConfiguration {
                    job: load_job_value(&input)?,
                    profile: serde_json::from_str(&read(&profile_path, 64_000)?)?,
                    configuration_name: name,
                })?;
            let updated = serde_json::to_string_pretty(&result["job"])?;
            write(&output, &(updated + "\n"))?;
            eprintln!(
                "machine configuration applied ({} mapping rows); the file is not needed again",
                result["inspection"]["machine"]["rows"]
                    .as_array()
                    .map(Vec::len)
                    .unwrap_or(0),
            );
            Ok(true)
        }
        "set-mapping" => {
            let output = output.ok_or("'collection set-mapping' requires --output")?;
            let tool = tool.ok_or("'collection set-mapping' requires --tool")?;
            let result = cam_service::collection::execute(CollectionCommand::SetToolMapping {
                job: load_job_value(&input)?,
                job_tool_id: tool.clone(),
                tool_number: number,
                length_offset_number: length,
            })?;
            let updated = serde_json::to_string_pretty(&result["job"])?;
            write(&output, &(updated + "\n"))?;
            eprintln!("mapping updated for tool '{tool}'");
            Ok(true)
        }
        "resolve-profile" => {
            let output = output.ok_or("'collection resolve-profile' requires --output")?;
            let result = cam_service::collection::execute(CollectionCommand::ResolveProfile {
                job: load_job_value(&input)?,
                scope: scope(through),
            })?;
            write(
                &output,
                &(serde_json::to_string_pretty(&result["profile"])? + "\n"),
            )?;
            eprintln!(
                "profile resolved with {} mapping rows",
                result["profile"]["tools"]
                    .as_array()
                    .map(Vec::len)
                    .unwrap_or(0)
            );
            Ok(true)
        }
        "knife-evidence" => {
            let output = output.ok_or("'collection knife-evidence' requires --output")?;
            let result = cam_service::collection::execute(CollectionCommand::KnifeEvidence {
                job: load_job_value(&input)?,
                scope: scope(through),
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
        "export" => {
            let output = output.ok_or("'collection export' requires --output (a directory)")?;
            let layout = match layout.as_deref() {
                None | Some("sequential") => OutputLayout::SequentialFiles,
                Some("one") => OutputLayout::OneProgram,
                Some(other) => {
                    return Err(
                        format!("unknown layout {other:?}; expected one or sequential").into(),
                    );
                }
            };
            let job = load_job_value(&input)?;
            // One retained generation, one preparation: every written file
            // comes from the bundle's exact checked bytes.
            let mut runtime = Retained::new();
            let generated = runtime.execute_driven(CollectionCommand::Generate {
                job: job.clone(),
                scope: scope(through),
            })?;
            let task = &generated["task"];
            if task["state"].as_str() != Some("succeeded") {
                return Err(format!(
                    "planning failed: {}",
                    task["diagnostic"]["message"]
                        .as_str()
                        .unwrap_or("no diagnostic")
                )
                .into());
            }
            let plan_handle = task["planHandle"].as_str().unwrap().to_string();
            let prepared = runtime.execute_driven(CollectionCommand::PrepareOutput {
                plan_handle,
                job,
                layout,
            })?;
            let task = &prepared["task"];
            if task["state"].as_str() != Some("succeeded") {
                return Err(format!(
                    "preparation failed: {}",
                    task["diagnostic"]["message"]
                        .as_str()
                        .unwrap_or("no diagnostic")
                )
                .into());
            }
            let prepare_task = task["taskId"].as_str().unwrap().to_string();
            let bundle = runtime.execute(CollectionCommand::PreparedOutput {
                task_id: prepare_task,
            })?["bundle"]
                .clone();
            let bundle_handle = bundle["bundleHandle"].as_str().unwrap().to_string();
            fs::create_dir_all(&output)?;
            let files = bundle["files"].as_array().cloned().unwrap_or_default();
            for file in &files {
                let filename = file["filename"].as_str().unwrap();
                let bytes = runtime.execute(CollectionCommand::ReadPreparedBytes {
                    bundle_handle: bundle_handle.clone(),
                    filename: filename.to_string(),
                })?["file"]["gcode"]
                    .as_str()
                    .unwrap()
                    .to_string();
                write(&output.join(filename), &bytes)?;
            }
            write(
                &output.join("manifest.json"),
                &(serde_json::to_string_pretty(&bundle["manifest"])? + "\n"),
            )?;
            write(
                &output.join("report.json"),
                &(serde_json::to_string_pretty(&bundle["report"])? + "\n"),
            )?;
            eprintln!(
                "exported {} file(s) to {} (layout: {}, execution sha256 {})",
                files.len(),
                output.display(),
                bundle["layout"].as_str().unwrap_or("?"),
                bundle["manifest"]["executionFingerprint"]
                    .as_str()
                    .unwrap_or("?"),
            );
            Ok(true)
        }
        _ => unreachable!("command checked above"),
    }
}
