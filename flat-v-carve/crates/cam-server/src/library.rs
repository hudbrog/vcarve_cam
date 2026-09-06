//! Local tool-library adapter. Browser inputs never select filesystem paths.
use crate::document::{JOB_BYTES, fingerprint};
use cam_app::tool_library::{StoreError, ToolLibraryStore};
use cam_core::{
    job::Job,
    tool_library::{CuttingPreset, LibraryChange, LibraryTool, MAX_LIBRARY_BYTES, ToolSlot},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{fs, io, path::PathBuf};

pub const METADATA_BYTES: usize = 16_100_000;
#[derive(Clone)]
pub struct Library {
    directory: PathBuf,
    store: ToolLibraryStore,
}
pub struct Failure(pub u16, pub String, pub String);
impl From<StoreError> for Failure {
    fn from(e: StoreError) -> Self {
        let status = match e.code.as_str() {
            "LIBRARY_CONFLICT" | "LIBRARY_EXISTS" => 409,
            "LIBRARY_BUSY" => 503,
            "LIBRARY_IO" => 500,
            "LIBRARY_RESOURCE_LIMIT" => 413,
            _ => 422,
        };
        Self(status, e.code, e.message)
    }
}
impl From<cam_core::geometry::Diagnostic> for Failure {
    fn from(e: cam_core::geometry::Diagnostic) -> Self {
        StoreError::from(e).into()
    }
}
fn fail(code: &str, message: &str) -> Failure {
    Failure(409, code.into(), message.into())
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub api_version: String,
    pub instance_id: String,
    pub request_id: String,
    pub command: Command,
}
#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Load {},
    Initialize {},
    Change {
        expected_revision: u64,
        change_json: String,
    },
    Import {
        expected_revision: u64,
        json: String,
    },
    Apply {
        expected_revision: u64,
        job: JobInput,
        slot: ToolSlot,
        tool_id: String,
        preset_id: Option<String>,
    },
    Capture {
        expected_revision: u64,
        job: JobInput,
        slot: ToolSlot,
        tool_id: String,
        name: String,
        preset: Option<CapturePreset>,
    },
}
impl Command {
    pub fn has_job(&self) -> bool {
        matches!(self, Self::Apply { .. } | Self::Capture { .. })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobInput {
    pub json: String,
    pub revision: u64,
    pub document_fingerprint: String,
}
impl JobInput {
    fn checked(&self) -> Result<Job, Failure> {
        if self.json.len() > JOB_BYTES || self.revision > 9_007_199_254_740_991 {
            return Err(Failure(
                413,
                "LIBRARY_JOB_LIMIT".into(),
                "Job size or revision exceeds the service limit.".into(),
            ));
        }
        let job = Job::from_json(&self.json)?;
        if fingerprint(&job) != self.document_fingerprint {
            return Err(fail(
                "STALE_DOCUMENT",
                "The job differs from its current Rust validation receipt.",
            ));
        }
        Ok(job)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturePreset {
    pub id: String,
    pub name: String,
    pub material: Option<String>,
    pub machine: Option<String>,
}

impl Library {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            store: ToolLibraryStore::new(directory.clone()),
            directory,
        }
    }
    pub fn location(&self) -> String {
        self.directory.display().to_string()
    }
    fn snapshot(&self) -> Result<Value, Failure> {
        match fs::symlink_metadata(self.directory.join("library.json")) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(json!({"state":"missing","library":null}));
            }
            Err(e) => return Err(StoreError::from(e).into()),
            Ok(_) => (),
        }
        Ok(json!({"state":"ready","library":self.store.load()?}))
    }
    pub fn execute(&self, command: Command) -> Result<Value, Failure> {
        let library = match command {
            Command::Load {} => return self.snapshot(),
            Command::Initialize {} => self.store.initialize()?,
            Command::Change {
                expected_revision,
                change_json,
            } => self
                .store
                .change(expected_revision, LibraryChange::from_json(&change_json)?)?,
            Command::Import {
                expected_revision,
                json,
            } => self.store.import_json(expected_revision, &json)?,
            Command::Apply {
                expected_revision,
                job,
                slot,
                tool_id,
                preset_id,
            } => {
                let original = job.checked()?;
                let candidate = self.store.apply_to_job(
                    expected_revision,
                    &original,
                    slot,
                    &tool_id,
                    preset_id.as_deref(),
                )?;
                return Ok(
                    json!({"libraryRevision":expected_revision,"jobRevision":job.revision,
                    "sourceFingerprint":job.document_fingerprint,"candidateFingerprint":fingerprint(&candidate),
                    "slot":slot,"toolId":tool_id,"presetId":preset_id,"job":candidate}),
                );
            }
            Command::Capture {
                expected_revision,
                job,
                slot,
                tool_id,
                name,
                preset,
            } => {
                let job = job.checked()?;
                let settings = job
                    .tools
                    .iter()
                    .find(|tool| tool.id == slot.job_id(&job))
                    .ok_or_else(|| fail("LIBRARY_NOT_FOUND", "Job tool slot was not found."))?;
                let mut tool = LibraryTool::from_settings(tool_id, name, settings)?;
                if let Some(preset) = preset {
                    let mut saved = CuttingPreset::from_settings(preset.id, preset.name, settings)?;
                    saved.material = preset.material;
                    saved.machine = preset.machine;
                    tool.cutting_presets.push(saved);
                }
                self.store
                    .change(expected_revision, LibraryChange::AddTool { tool })?
            }
        };
        // Store methods validate and enforce the final serialized 8 MB bound.
        Ok(json!({"state":"ready","library":library}))
    }
}

/// Stable platform application-data location; tests/portable users can override it.
pub fn default_directory() -> io::Result<PathBuf> {
    let root = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("FlatVCarve"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|p| p.join("Library/Application Support/FlatVCarve"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|p| p.join(".local/share"))
            })
            .map(|p| p.join("flat-v-carve"))
    };
    root.map(|p| p.join("tool-library")).ok_or_else(|| {
        io::Error::other("Application data directory is unavailable; supply --library-dir.")
    })
}

pub fn limits(location: String) -> Value {
    json!({"schemaVersion":1,"maxBytes":MAX_LIBRARY_BYTES,"maxTools":cam_core::tool_library::MAX_TOOLS,
        "maxPresetsPerTool":cam_core::tool_library::MAX_PRESETS_PER_TOOL,"location":location})
}
