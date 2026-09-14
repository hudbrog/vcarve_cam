//! The job commands on the one document model: import an SVG into a new
//! schema-5 document, and inspect a document without changing it.
//!
//! Everything else the old job commands did is the collection surface:
//! `cam collection plan` and `cam collection export` operate on the same
//! documents, so there is one implementation of planning and export instead of
//! two.
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

pub const HELP: &str = "Job documents (schema 5)\n\nUsage:\n  cam import <artwork.svg> --output <job.json> [--tolerance <mm>]\n      Create a schema-5 document holding this artwork and its page-sized stock.\n      No operation is invented: add one in the workspace or with `cam collection`.\n  cam inspect <job.json> --output <inspection.json>\n      Read-only inspection of a schema-5 document (an alias of `cam collection inspect`).\n";

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
    fs::write(path, contents)?;
    println!("{}", path.display());
    Ok(())
}

pub fn run(command: &str, args: Vec<String>) -> AppResult<bool> {
    match command {
        "import" => import(args),
        // One inspection implementation, shared with the collection surface.
        "inspect" => {
            super::collection_cli::run(std::iter::once("inspect".to_owned()).chain(args).collect())
        }
        other => Err(format!("unknown job command {other:?}").into()),
    }
}

fn import(args: Vec<String>) -> AppResult<bool> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut tolerance = 0.001;
    let mut args = args.into_iter();
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
            "--tolerance" => {
                tolerance = args
                    .next()
                    .ok_or("--tolerance requires a number")?
                    .parse()
                    .map_err(|_| "--tolerance must be a number in millimeters")?;
            }
            other if input.is_none() => input = Some(PathBuf::from(other)),
            other => return Err(format!("unknown or repeated argument {other:?}").into()),
        }
    }
    let output = output.ok_or("'import' requires --output")?;
    let input = input.ok_or("'import' requires an SVG path")?;
    // A refusal must never cost the caller their artwork: the input is read
    // once at the start and the output is written only after the document is
    // complete, so writing over the source is refused before anything is read.
    if output == input {
        return Err("use a different output path to preserve the input".into());
    }
    let filename = input
        .file_name()
        .ok_or("input filename missing")?
        .to_string_lossy()
        .into_owned();
    let svg = read(&input, cam_core::svg::MAX_SVG_BYTES)?;
    let job = cam_core::project::v5::authoring::from_svg(filename, svg, tolerance)?;
    let json = job.to_json()?;
    // The document budget protects the portable file, exactly as the workspace
    // applies it when saving.
    if json.len() > cam_service::JOB_BYTES {
        return Err("the resulting job exceeds the portable document budget".into());
    }
    write(&output, &json)?;
    eprintln!(
        "Saved a schema-5 document with {} artwork item(s) and no operations; add one with `cam collection` or the workspace",
        job.artwork.len()
    );
    Ok(true)
}
