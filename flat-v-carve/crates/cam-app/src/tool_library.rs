//! Versioned local library persistence. All participating writers lock one stable
//! sidecar file, compare the revision, then publish a flushed replacement by rename.
use cam_core::{
    geometry::Diagnostic,
    job::Job,
    tool_library::{LibraryChange, MAX_LIBRARY_BYTES, ToolLibrary, ToolSlot},
};
use serde::Serialize;
use std::{
    fs::{self, File, TryLockError},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Serialize)]
pub struct StoreError {
    pub code: String,
    pub message: String,
}
impl StoreError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for StoreError {}
impl From<Diagnostic> for StoreError {
    fn from(value: Diagnostic) -> Self {
        Self {
            code: value.code,
            message: value.message,
        }
    }
}
impl From<io::Error> for StoreError {
    fn from(value: io::Error) -> Self {
        Self::new("LIBRARY_IO", value.to_string())
    }
}
pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Clone, Debug)]
pub struct ToolLibraryStore {
    directory: PathBuf,
}
impl ToolLibraryStore {
    /// The caller selects an application-owned local directory; tool IDs never
    /// participate in paths. No profile or machining defaults are created.
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn initialize(&self) -> StoreResult<ToolLibrary> {
        fs::create_dir_all(&self.directory)?;
        let directory = self.directory.canonicalize()?;
        let _lock = lock(&directory)?;
        match fs::symlink_metadata(directory.join("library.json")) {
            Ok(_) => {
                return Err(StoreError::new(
                    "LIBRARY_EXISTS",
                    "library already exists; initialization never replaces it",
                ));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let library = ToolLibrary::default();
        save(&directory, &library)?;
        Ok(library)
    }
    /// Atomic replacement permits readers to see either complete revision.
    /// Missing/corrupt libraries are errors, never an implicit empty library.
    pub fn load(&self) -> StoreResult<ToolLibrary> {
        load(&self.directory)
    }

    pub fn change(
        &self,
        expected_revision: u64,
        change: LibraryChange,
    ) -> StoreResult<ToolLibrary> {
        let directory = self.directory.canonicalize()?;
        let _lock = lock(&directory)?;
        let current = load(&directory)?;
        let next = current.changed(expected_revision, change)?;
        save(&directory, &next)?;
        Ok(next)
    }
    /// Merge only new tool IDs. Imported revisions never replace the local revision.
    pub fn import_json(&self, expected_revision: u64, json: &str) -> StoreResult<ToolLibrary> {
        let library = ToolLibrary::from_json(json)?;
        self.change(expected_revision, LibraryChange::Import { library })
    }
    pub fn export_json(&self) -> StoreResult<String> {
        Ok(self.load()?.to_json()?)
    }
    /// Resolve the revision the user selected and return a candidate job for review.
    /// This changes neither the stored library nor the supplied job.
    pub fn apply_to_job(
        &self,
        expected_revision: u64,
        job: &Job,
        slot: ToolSlot,
        tool_id: &str,
        preset_id: Option<&str>,
    ) -> StoreResult<Job> {
        let library = self.load()?;
        library.require_revision(expected_revision)?;
        Ok(library.apply_to_job(job, slot, tool_id, preset_id)?)
    }
}

fn lock(directory: &Path) -> StoreResult<File> {
    // Never remove this file: deleting/recreating it could give writers different locks.
    // The OS releases the actual lock when this handle closes, including process exit.
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("library.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(StoreError::new(
            "LIBRARY_BUSY",
            "another library writer is active; retry after reloading",
        )),
        Err(TryLockError::Error(e)) => Err(e.into()),
    }
}
fn load(directory: &Path) -> StoreResult<ToolLibrary> {
    let file = File::open(directory.join("library.json"))?;
    let mut json = String::new();
    file.take(MAX_LIBRARY_BYTES as u64 + 1)
        .read_to_string(&mut json)?;
    Ok(ToolLibrary::from_json(&json)?)
}

static TEMP_ID: AtomicU64 = AtomicU64::new(0);
fn save(directory: &Path, library: &ToolLibrary) -> StoreResult<()> {
    let json = library.to_json()?;
    let (temporary, mut file) = loop {
        let name = format!(
            "library-{}-{}.tmp",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        );
        let path = directory.join(name);
        match File::create_new(&path) {
            Ok(file) => break (path, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    };
    let result = (|| -> io::Result<()> {
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        // Windows must also be able to rename the temporary file after closing it.
        drop(file);
        fs::rename(&temporary, directory.join("library.json"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(Into::into)
}
