use cam_app::tool_library::ToolLibraryStore;
use cam_core::{
    job::Job,
    tool_library::{CuttingPreset, LibraryChange, LibraryTool, MAX_LIBRARY_BYTES, ToolSlot},
};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;
pub const HELP: &str = "Local tool library (all dimensions in mm; cutting values are explicit)\n\n\
  cam tool-library init <directory>\n\
  cam tool-library list <directory>\n\
  cam tool-library change <directory> --expected-revision <n> --input <change.json>\n\
  cam tool-library capture <directory> --expected-revision <n> --job <job.json>\n\
      --slot endmill|vbit --tool <new-id> --name <name>\n\
      [--preset <new-id> --preset-name <name> [--material <label>] [--machine <label>]]\n\
  cam tool-library import <directory> --expected-revision <n> --input <library.json>\n\
  cam tool-library export <directory> --output <new-library.json>\n\
  cam tool-library apply <directory> --expected-revision <n> --job <job.json>\n\
      --slot endmill|vbit --tool <id> [--preset <id>] --output <new-job.json>\n\n\
init/list/change/capture/import return the complete library JSON on stdout.\n\
change accepts add_tool, replace_tool, remove_tool, duplicate_tool, add_preset,\n\
replace_preset, remove_preset, duplicate_preset, or import (tagged by kind).\n\
Import merges new tool IDs; ID conflicts reject the whole transaction.\n\
Capture saves geometry/capabilities; cutting values require an explicit preset.\n\
Apply copies into the requested job slot; omitting a preset clears cutting values.\n\
Existing export/apply outputs are never overwritten. Errors exit 2.\n";

fn required<'a>(options: &'a BTreeMap<String, String>, key: &str) -> AppResult<&'a str> {
    options
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("{key} is required").into())
}
fn read(path: &str, limit: usize) -> AppResult<String> {
    let mut text = String::new();
    File::open(path)?
        .take(limit as u64 + 1)
        .read_to_string(&mut text)?;
    if text.len() > limit {
        return Err(format!("input exceeds {limit} bytes").into());
    }
    Ok(text)
}
fn write_new(path: &str, text: &str) -> AppResult<()> {
    let path = Path::new(path);
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let mut file = File::create_new(path)?;
    let result = file
        .write_all(text.as_bytes())
        .and_then(|_| file.sync_all());
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    Ok(result?)
}
pub fn run(args: Vec<String>) -> AppResult<bool> {
    let mut args = args.into_iter();
    let command = args.next().unwrap_or_else(|| "--help".into());
    if matches!(command.as_str(), "--help" | "-h") {
        print!("{HELP}");
        return Ok(true);
    }
    let allowed: &[&str] = match command.as_str() {
        "init" | "list" => &[],
        "change" | "import" => &["--expected-revision", "--input"],
        "export" => &["--output"],
        "capture" => &[
            "--expected-revision",
            "--job",
            "--slot",
            "--tool",
            "--name",
            "--preset",
            "--preset-name",
            "--material",
            "--machine",
        ],
        "apply" => &[
            "--expected-revision",
            "--job",
            "--slot",
            "--tool",
            "--preset",
            "--output",
        ],
        _ => {
            return Err(
                format!("unknown library command {command:?}; use tool-library --help").into(),
            );
        }
    };
    let directory = args.next().ok_or("a library directory is required")?;
    if directory.starts_with('-') {
        return Err("a library directory is required before options".into());
    }
    let mut options = BTreeMap::new();
    while let Some(key) = args.next() {
        if !allowed.contains(&key.as_str()) || options.contains_key(&key) {
            return Err(format!("unknown/repeated argument {key:?}").into());
        }
        let value = args
            .next()
            .ok_or_else(|| format!("{key} requires a value"))?;
        options.insert(key, value);
    }
    let store = ToolLibraryStore::new(directory);
    let revision = || -> AppResult<u64> { Ok(required(&options, "--expected-revision")?.parse()?) };
    let library = match command.as_str() {
        "init" => store.initialize()?,
        "list" => store.load()?,
        "change" => {
            let change = LibraryChange::from_json(&read(
                required(&options, "--input")?,
                MAX_LIBRARY_BYTES,
            )?)?;
            store.change(revision()?, change)?
        }
        "import" => store.import_json(
            revision()?,
            &read(required(&options, "--input")?, MAX_LIBRARY_BYTES)?,
        )?,
        "export" => {
            write_new(required(&options, "--output")?, &store.export_json()?)?;
            return Ok(true);
        }
        "capture" | "apply" => {
            let expected_revision = revision()?;
            let job = Job::from_json(&read(required(&options, "--job")?, 64_000_000)?)?;
            let slot = match required(&options, "--slot")? {
                "endmill" => ToolSlot::Endmill,
                "vbit" => ToolSlot::Vbit,
                _ => return Err("--slot must be endmill or vbit".into()),
            };
            let tool_id = required(&options, "--tool")?;
            if command == "apply" {
                let candidate = store.apply_to_job(
                    expected_revision,
                    &job,
                    slot,
                    tool_id,
                    options.get("--preset").map(String::as_str),
                )?;
                let json = candidate.to_json()?;
                if json.len() > 64_000_000 {
                    return Err("job exceeds the 64 MB reload limit".into());
                }
                write_new(required(&options, "--output")?, &json)?;
                return Ok(true);
            }
            let settings = job
                .tools
                .iter()
                .find(|t| t.id == slot.job_id(&job))
                .ok_or("job tool slot not found")?;
            let mut tool = LibraryTool::from_settings(
                tool_id.into(),
                required(&options, "--name")?.into(),
                settings,
            )?;
            if let Some(preset_id) = options.get("--preset") {
                let mut preset = CuttingPreset::from_settings(
                    preset_id.clone(),
                    required(&options, "--preset-name")?.into(),
                    settings,
                )?;
                preset.material = options.get("--material").cloned();
                preset.machine = options.get("--machine").cloned();
                tool.cutting_presets.push(preset);
            } else if ["--preset-name", "--material", "--machine"]
                .iter()
                .any(|key| options.contains_key(*key))
            {
                return Err("cutting preset metadata requires --preset".into());
            }
            store.change(expected_revision, LibraryChange::AddTool { tool })?
        }
        _ => unreachable!(),
    };
    print!("{}", library.to_json()?);
    Ok(true)
}
