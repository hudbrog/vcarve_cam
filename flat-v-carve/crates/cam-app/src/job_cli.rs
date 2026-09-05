use cam_core::{
    geometry::Diagnostic,
    job::Job,
    pocket::{EndmillPlan, PlanStatus, plan_endmill},
    svg::{ImportOptions, MAX_SVG_BYTES},
    vcarve::{CombinedPlan, plan_combined},
    verification::{VerificationOptions, VerificationStatus, verify_plan},
};
use serde_json::json;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;
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
pub fn run(command: &str, args: Vec<String>) -> AppResult<bool> {
    let mut args = args.into_iter();
    let mut input = None;
    let mut output = None;
    let mut report = None;
    let mut options = ImportOptions::default();
    let mut tolerance_set = false;
    let mut selection = vec![];
    let mut stage = None;
    let mut verification_options = VerificationOptions::default();
    let mut verification_flags = std::collections::BTreeSet::new();
    let mut verification_preview = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                print!("{}", super::HELP);
                return Ok(true);
            }
            "--output" if output.is_none() && command != "validate-job" => {
                output = Some(PathBuf::from(
                    args.next().ok_or("--output requires a file")?,
                ))
            }
            "--report" if report.is_none() && command == "inspect" => {
                report = Some(PathBuf::from(
                    args.next().ok_or("--report requires a file")?,
                ))
            }
            "--tolerance" if command == "import" && !tolerance_set => {
                options.geometry_tolerance_mm = args
                    .next()
                    .ok_or("--tolerance requires millimeters")?
                    .parse()?;
                tolerance_set = true;
            }
            "--stage" if command == "plan" && stage.is_none() => {
                let value = args.next().ok_or("--stage requires endmill or combined")?;
                if !matches!(value.as_str(), "endmill" | "combined") {
                    return Err("--stage requires endmill or combined".into());
                }
                stage = Some(value);
            }
            "--max-cells"
            | "--max-depth"
            | "--reachability-cells"
            | "--max-depth-bands"
            | "--decimal-places"
                if command == "verify" && verification_flags.insert(arg.clone()) =>
            {
                let value: usize = args
                    .next()
                    .ok_or("verification flag requires an integer")?
                    .parse()?;
                match arg.as_str() {
                    "--max-cells" => verification_options.max_cells = value,
                    "--max-depth" => verification_options.max_depth = value,
                    "--reachability-cells" => verification_options.reachability_max_cells = value,
                    "--max-depth-bands" => verification_options.max_depth_bands = value,
                    "--decimal-places" => verification_options.decimal_places = Some(value),
                    _ => unreachable!(),
                }
            }
            "--preview" if command == "verify" && verification_preview.is_none() => {
                verification_preview = Some(PathBuf::from(
                    args.next().ok_or("--preview requires an SVG file")?,
                ));
            }
            "--select" if matches!(command, "import" | "select") => {
                selection.push(args.next().ok_or("--select requires a region ID")?)
            }
            _ if !arg.starts_with('-') && input.is_none() => input = Some(PathBuf::from(arg)),
            _ => return Err(format!("unknown/repeated argument {arg:?}").into()),
        }
    }
    let input = input.ok_or("an input SVG or job file is required")?;
    if command != "validate-job" && output.is_none() {
        return Err("--output is required".into());
    }
    if report.as_ref().is_some_and(|r| Some(r) == output.as_ref()) {
        return Err("preview and report must use different output files".into());
    }
    if output.as_ref() == Some(&input) || report.as_ref() == Some(&input) {
        return Err("use a different output path to preserve the input".into());
    }
    verification_options.validate()?;
    if verification_preview
        .as_ref()
        .is_some_and(|p| p == &input || Some(p) == output.as_ref())
    {
        return Err(
            "verification preview must use a different path from the input and JSON report".into(),
        );
    }
    let contents = read(
        &input,
        if command == "import" {
            MAX_SVG_BYTES
        } else if matches!(command, "inspect" | "verify") {
            128_000_000
        } else {
            64_000_000
        },
    )?;
    #[derive(serde::Deserialize)]
    struct ArtifactKind {
        artifact_kind: Option<String>,
    }
    // Skip arrays instead of allocating a second complete motion/report tree.
    let kind = serde_json::from_str::<ArtifactKind>(&contents)
        .ok()
        .and_then(|v| v.artifact_kind);
    let is_plan = kind.is_some();
    if matches!(command, "inspect" | "verify") && kind.as_deref() == Some("combined_plan") {
        return match CombinedPlan::from_json(&contents) {
            Ok(plan) => {
                if command == "verify" {
                    return match verify_plan(&plan, &verification_options) {
                        Ok(verification) => {
                            let passed = verification.status == VerificationStatus::Passed;
                            let data = json!({"valid":true,"milestone":"M5","input_fingerprint":plan.input_fingerprint,
                                "motion_fingerprint":plan.motion_fingerprint,"endmill_status":plan.endmill.analysis.status,
                                "analysis":plan.analysis,"verification":verification});
                            write(
                                output.as_ref().unwrap(),
                                &(serde_json::to_string_pretty(&data)? + "\n"),
                            )?;
                            if let Some(preview) = verification_preview {
                                write(
                                    &preview,
                                    &super::verification_svg::render(
                                        &plan.endmill.job,
                                        &verification,
                                    )?,
                                )?;
                            }
                            eprintln!(
                                "M5 verification: {:?}; {} adaptive cells, {} unresolved; rounded coordinates: {}",
                                verification.status,
                                verification.original.evaluated_cells,
                                verification.original.unresolved_cells,
                                verification.rounded.as_ref().map_or_else(
                                    || "not requested".into(),
                                    |r| format!(
                                        "{:?} at {} decimal places",
                                        r.verification.status, r.decimal_places
                                    )
                                )
                            );
                            Ok(passed)
                        }
                        Err(d) => {
                            failed(command, &d, output.as_deref(), None)?;
                            if let Some(preview) = verification_preview {
                                write(&preview, &super::job_svg::failure(&d.to_string()))?;
                            }
                            Ok(false)
                        }
                    };
                }
                let data = serde_json::to_string_pretty(
                    &json!({"valid":true,"milestone":"M4","input_fingerprint":plan.input_fingerprint,"motion_fingerprint":plan.motion_fingerprint,"endmill_status":plan.endmill.analysis.status,"analysis":plan.analysis}),
                )? + "\n";
                if command == "inspect" {
                    write(
                        output.as_ref().unwrap(),
                        &super::combined_svg::render(&plan),
                    )?;
                    if let Some(report) = report {
                        write(&report, &data)?;
                    }
                } else {
                    write(output.as_ref().unwrap(), &data)?;
                }
                eprintln!(
                    "Combined M4 stage: {:?}; {} endmill and {} V-bit motions, {} quality samples",
                    plan.analysis.status,
                    plan.endmill.motions.len(),
                    plan.vbit_motions.len(),
                    plan.analysis.samples.len()
                );
                Ok(matches!(
                    plan.analysis.status,
                    PlanStatus::Complete | PlanStatus::Empty
                ))
            }
            Err(d) => {
                failed(command, &d, output.as_deref(), report.as_deref())?;
                if let Some(preview) = verification_preview {
                    write(&preview, &super::job_svg::failure(&d.to_string()))?;
                }
                Ok(false)
            }
        };
    }
    if matches!(command, "inspect" | "verify") && (is_plan || command == "verify") {
        if !verification_flags.is_empty() || verification_preview.is_some() {
            return Err("M5 verification options require a combined plan; endmill-only verify retains the M3 stage contract".into());
        }
        return match EndmillPlan::from_json(&contents) {
            Ok(plan) => {
                let data = serde_json::to_string_pretty(
                    &json!({"valid":true,"milestone":"M4","input_fingerprint":plan.input_fingerprint,"motion_fingerprint":plan.motion_fingerprint,"analysis":plan.analysis}),
                )? + "\n";
                if command == "inspect" {
                    write(output.as_ref().unwrap(), &super::plan_svg::render(&plan))?;
                    if let Some(report) = report {
                        write(&report, &data)?;
                    }
                } else {
                    write(output.as_ref().unwrap(), &data)?;
                }
                eprintln!(
                    "Endmill stage: {:?}; recomputed {} motions and {} stock slices",
                    plan.analysis.status,
                    plan.motions.len(),
                    plan.analysis.layers.len()
                );
                Ok(matches!(
                    plan.analysis.status,
                    PlanStatus::Complete | PlanStatus::Empty
                ))
            }
            Err(d) => {
                failed(command, &d, output.as_deref(), report.as_deref())?;
                Ok(false)
            }
        };
    }
    let parsed = if command == "import" {
        Job::from_svg(
            input
                .file_name()
                .ok_or("input filename missing")?
                .to_string_lossy()
                .into_owned(),
            contents,
            options,
        )
    } else {
        Job::from_json(&contents)
    };
    let result = parsed.and_then(|mut job| {
        if command == "select" || !selection.is_empty() {
            job.selected_region_ids = selection;
        }
        job.inspect().map(|inspection| (job, inspection))
    });
    match result {
        Ok((job, inspection)) => {
            let data = json!({"valid":true,"milestone":"M4","inspection":inspection});
            match command {
                "import" | "select" => {
                    write(output.as_ref().unwrap(), &job.to_json()?)?;
                    eprintln!(
                        "Saved {} selected regions to {}",
                        job.selected_region_ids.len(),
                        output.as_ref().unwrap().display()
                    );
                }
                "inspect" => {
                    write(
                        output.as_ref().unwrap(),
                        &super::job_svg::render(&inspection),
                    )?;
                    if let Some(report) = report {
                        write(&report, &(serde_json::to_string_pretty(&data)? + "\n"))?;
                    }
                    eprintln!(
                        "Inspected {} selected regions; {} machining settings remain unset",
                        job.selected_region_ids.len(),
                        inspection.missing_machining_fields.len()
                    );
                }
                "validate-job" => println!("{}", serde_json::to_string_pretty(&data)?),
                "plan" => {
                    if stage.as_deref() == Some("combined")
                        || stage.is_none() && job.vbit_planning.is_some()
                    {
                        return match plan_combined(&job) {
                            Ok(plan) => {
                                write(output.as_ref().unwrap(), &plan.to_json()?)?;
                                eprintln!(
                                    "Combined M4 stage: {:?}; {} endmill and {} V-bit motions",
                                    plan.analysis.status,
                                    plan.endmill.motions.len(),
                                    plan.vbit_motions.len()
                                );
                                for d in &plan.analysis.diagnostics {
                                    eprintln!("{d}");
                                }
                                Ok(matches!(
                                    plan.analysis.status,
                                    PlanStatus::Complete | PlanStatus::Empty
                                ))
                            }
                            Err(d) => {
                                failed(command, &d, output.as_deref(), report.as_deref())?;
                                Ok(false)
                            }
                        };
                    }
                    return match plan_endmill(&job) {
                        Ok(plan) => {
                            write(output.as_ref().unwrap(), &plan.to_json()?)?;
                            eprintln!(
                                "Endmill stage: {:?}; {} motions, {} stock slices",
                                plan.analysis.status,
                                plan.motions.len(),
                                plan.analysis.layers.len()
                            );
                            for d in &plan.analysis.diagnostics {
                                eprintln!("{d}");
                            }
                            Ok(matches!(
                                plan.analysis.status,
                                PlanStatus::Complete | PlanStatus::Empty
                            ))
                        }
                        Err(d) => {
                            failed(command, &d, output.as_deref(), report.as_deref())?;
                            Ok(false)
                        }
                    };
                }
                _ => unreachable!(),
            }
            for d in inspection.geometry.diagnostics {
                eprintln!("{d}");
            }
            Ok(true)
        }
        Err(diagnostic) => {
            failed(command, &diagnostic, output.as_deref(), report.as_deref())?;
            Ok(false)
        }
    }
}
fn failed(
    command: &str,
    d: &Diagnostic,
    output: Option<&Path>,
    report: Option<&Path>,
) -> AppResult<()> {
    let data = serde_json::to_string_pretty(
        &json!({"schema_version":1,"milestone":if command == "verify" {"M5"} else {"M4"},"valid":false,"diagnostics":[d]}),
    )? + "\n";
    match command {
        "inspect" => {
            write(output.unwrap(), &super::job_svg::failure(&d.to_string()))?;
            if let Some(r) = report {
                write(r, &data)?;
            }
        }
        "validate-job" => print!("{data}"),
        "plan" | "verify" => write(output.unwrap(), &data)?,
        _ => {}
    }
    eprintln!("{d}");
    Ok(())
}
