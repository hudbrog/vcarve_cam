use cam_core::{
    post::{LinuxCncProfile, Program, ProgramLayout, export_plan, verify_programs},
    vcarve::CombinedPlan,
    verification::{VerificationOptions, VerificationStatus},
};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn read(path: &Path, limit: u64) -> Result<String> {
    let mut bytes = vec![];
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(format!("{} exceeds {limit} bytes", path.display()).into());
    }
    Ok(String::from_utf8(bytes)?)
}
fn new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
/// Publish a new directory only after every file has been written. Existing
/// export bundles are immutable here, so a failed run cannot leave old G-code
/// next to a new failure report.
fn bundle(output: &Path, report: &str, programs: &[Program]) -> Result<()> {
    if output.exists() {
        return Err("output directory already exists; choose a new export directory".into());
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let parent = fs::canonicalize(parent)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let temp = parent.join(format!(".cam-export-{}-{stamp}", std::process::id()));
    fs::create_dir(&temp)?;
    let staging = Staging(temp);
    new_file(&staging.0.join("export-report.json"), report.as_bytes())?;
    for program in programs {
        new_file(&staging.0.join(&program.filename), program.gcode.as_bytes())?;
    }
    if output.exists() {
        return Err("output directory appeared during export; choose another directory".into());
    }
    fs::rename(&staging.0, output)?;
    Ok(())
}
pub fn run(command: &str, args: Vec<String>) -> Result<bool> {
    let mut args = args.into_iter();
    let (mut input, mut output, mut profile_path) = (None, None, None);
    let mut layout = ProgramLayout::Combined;
    let mut seen = std::collections::BTreeSet::new();
    let mut program_paths = vec![];
    let mut options = VerificationOptions::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                print!("{}", super::HELP);
                return Ok(true);
            }
            "--output"
            | "--profile"
            | "--layout"
            | "--max-cells"
            | "--max-depth"
            | "--reachability-cells"
            | "--max-depth-bands"
                if seen.insert(arg.clone()) =>
            {
                let value = args.next().ok_or("option requires a value")?;
                match arg.as_str() {
                    "--output" => output = Some(PathBuf::from(value)),
                    "--profile" => profile_path = Some(PathBuf::from(value)),
                    "--layout" => {
                        layout = match value.as_str() {
                            "combined" => ProgramLayout::Combined,
                            "per-tool" => ProgramLayout::PerTool,
                            _ => return Err("--layout requires combined or per-tool".into()),
                        }
                    }
                    "--max-cells" => options.max_cells = value.parse()?,
                    "--max-depth" => options.max_depth = value.parse()?,
                    "--reachability-cells" => options.reachability_max_cells = value.parse()?,
                    "--max-depth-bands" => options.max_depth_bands = value.parse()?,
                    _ => unreachable!(),
                }
            }
            "--program" if command == "verify-gcode" && program_paths.len() < 2 => program_paths
                .push(PathBuf::from(
                    args.next().ok_or("--program requires a file")?,
                )),
            _ if !arg.starts_with('-') && input.is_none() => input = Some(PathBuf::from(arg)),
            _ => return Err(format!("unknown/repeated argument {arg:?}").into()),
        }
    }
    let input = input.ok_or("a combined plan file is required")?;
    let profile_path = profile_path.ok_or("--profile requires an explicit LinuxCNC profile")?;
    let output = output.ok_or("--output is required")?;
    if output.exists() {
        return Err("output already exists; choose a new output path".into());
    }
    let profile = LinuxCncProfile::from_json(&read(&profile_path, 64_000)?)?;
    let plan = CombinedPlan::from_json(&read(&input, 128_000_000)?)?;
    let report = if command == "export" {
        let result = export_plan(&plan, &profile, layout, &options)?;
        let json = serde_json::to_string_pretty(&result.report)? + "\n";
        bundle(&output, &json, &result.programs)?;
        result.report
    } else {
        if program_paths.is_empty() {
            return Err(
                "verify-gcode requires --program; supply per-tool programs in endmill/V-bit order"
                    .into(),
            );
        }
        let programs = program_paths
            .iter()
            .map(|path| {
                Ok(Program {
                    filename: path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .ok_or("program needs a UTF-8 filename")?
                        .into(),
                    gcode: read(path, 128_000_000)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let report = verify_programs(&plan, &profile, layout, &options, &programs)?;
        if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        new_file(
            &output,
            (serde_json::to_string_pretty(&report)? + "\n").as_bytes(),
        )?;
        report
    };
    eprintln!(
        "M6 {:?}; {} checked program(s); output: {}",
        report.status,
        report.programs.len(),
        output.display()
    );
    for diagnostic in &report.diagnostics {
        eprintln!("{diagnostic}");
    }
    Ok(report.status == VerificationStatus::Passed)
}
