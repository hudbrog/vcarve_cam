use cam_core::{
    geometry::Diagnostic,
    job::Job,
    svg::{ImportOptions, MAX_SVG_BYTES},
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
    let parsed = if command == "import" {
        Job::from_svg(
            input
                .file_name()
                .ok_or("input filename missing")?
                .to_string_lossy()
                .into_owned(),
            read(&input, MAX_SVG_BYTES)?,
            options,
        )
    } else {
        Job::from_json(&read(&input, 8_000_000)?)
    };
    let result = parsed.and_then(|mut job| {
        if command == "select" || !selection.is_empty() {
            job.selected_region_ids = selection;
        }
        job.inspect().map(|inspection| (job, inspection))
    });
    match result {
        Ok((job, inspection)) => {
            let data = json!({"valid":true,"milestone":"M2","inspection":inspection});
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
                    let data = json!({"schema_version":1,"milestone":"M2","planning_available":false,"plan":null,"missing_machining_fields":inspection.missing_machining_fields,
                        "diagnostics":[{"code":"PLANNING_NOT_IMPLEMENTED","severity":"error","stage":"plan","message":"M2 imports and inspects jobs. Cutting path generation begins in M3; no machining plan was generated."}]});
                    write(
                        output.as_ref().unwrap(),
                        &(serde_json::to_string_pretty(&data)? + "\n"),
                    )?;
                    eprintln!("PLANNING_NOT_IMPLEMENTED: cutting paths begin in M3");
                    return Ok(false);
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
        &json!({"schema_version":1,"milestone":"M2","valid":false,"diagnostics":[d]}),
    )? + "\n";
    match command {
        "inspect" => {
            write(output.unwrap(), &super::job_svg::failure(&d.to_string()))?;
            if let Some(r) = report {
                write(r, &data)?;
            }
        }
        "validate-job" => print!("{data}"),
        "plan" => write(output.unwrap(), &data)?,
        _ => {}
    }
    eprintln!("{d}");
    Ok(())
}
