//! Native file operations used by dialogs, drops, recovery and headless probes.
//! No UI-thread I/O. Atomic sibling replacement preserves the previous file on
//! a failed write; participating recovery writers share an OS-released lock.
use crate::recovery::{MAX_BYTES, Snapshot, Stored};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static TEMP_ID: AtomicU64 = AtomicU64::new(0);
pub fn read_text(path: &Path, limit: usize) -> Result<String, String> {
    let mut text = String::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > limit {
        return Err(format!("Input exceeds {limit} bytes"));
    }
    Ok(text)
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("Destination has no parent")?;
    let (temporary, mut file) = loop {
        let candidate = parent.join(format!(
            ".cam-gui-{}-{}.tmp",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        match File::create_new(&candidate) {
            Ok(file) => break (candidate, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    };
    let result = (|| -> std::io::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e| e.to_string())
}
pub struct Store {
    directory: PathBuf,
}

impl Store {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }
    pub fn default_location() -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        Ok(Self::new(
            exe.parent()
                .ok_or("Missing executable directory")?
                .join("cam-gui-recovery"),
        ))
    }
    pub fn resources_location() -> Result<Self, String> {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
            .ok_or("User resource directory is unavailable")?;
        Ok(Self::new(root.join("flat-v-carve/resources")))
    }
    fn lock(&self) -> Result<File, String> {
        fs::create_dir_all(&self.directory).map_err(|e| e.to_string())?;
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.directory.join("session.lock"))
            .map_err(|e| e.to_string())?;
        file.try_lock()
            .map_err(|e| format!("Local storage is busy or unavailable: {e}"))?;
        Ok(file)
    }
    fn read_unlocked(&self) -> Result<Option<Stored>, String> {
        let path = self.directory.join("session.json");
        match fs::metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
            Ok(_) => Stored::decode(&read_text(&path, MAX_BYTES)?).map(Some),
        }
    }
    pub fn load(&self) -> Result<Option<Stored>, String> {
        let _lock = self.lock()?;
        self.read_unlocked()
    }
    fn resources_unlocked(&self) -> Result<Option<crate::resources::StoredCatalog>, String> {
        let path = self.directory.join("resources.json");
        match fs::metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
            Ok(_) => {
                let text = read_text(&path, crate::resources::MAX_BYTES + 1024)?;
                let stored: crate::resources::StoredCatalog =
                    serde_json::from_str(&text).map_err(|e| e.to_string())?;
                stored.validate()?;
                Ok(Some(stored))
            }
        }
    }
    pub fn load_resources(&self) -> Result<Option<crate::resources::StoredCatalog>, String> {
        let _lock = self.lock()?;
        self.resources_unlocked()
    }
    pub fn save_resources(
        &self,
        expected: Option<u64>,
        snapshot: crate::resources::Catalog,
    ) -> Result<crate::resources::StoredCatalog, String> {
        let stored = crate::resources::StoredCatalog::next(expected, snapshot)?;
        let _lock = self.lock()?;
        if self.resources_unlocked()?.as_ref().map(|s| s.revision) != expected {
            return Err("Library revision conflict. Compare the stored revision; your edit buffer is preserved.".into());
        }
        atomic_write(
            &self.directory.join("resources.json"),
            &serde_json::to_vec(&stored).map_err(|e| e.to_string())?,
        )?;
        Ok(stored)
    }
    pub fn save(&self, expected: Option<u64>, snapshot: Snapshot) -> Result<u64, String> {
        snapshot.validate()?;
        let _lock = self.lock()?;
        let current = self.read_unlocked()?;
        if current.as_ref().map(|r| r.revision) != expected {
            return Err(
                "Revision conflict; reload recovery before choosing which draft to keep".into(),
            );
        }
        let revision = expected
            .unwrap_or(0)
            .checked_add(1)
            .filter(|r| *r <= 9_007_199_254_740_991)
            .ok_or("Recovery revision limit")?;
        let bytes =
            serde_json::to_vec(&Stored { revision, snapshot }).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_BYTES {
            return Err("Recovery exceeds 9 MB".into());
        }
        atomic_write(&self.directory.join("session.json"), &bytes)?;
        Ok(revision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_conflict_and_denied_replacement_preserve_last_good_draft() {
        let directory =
            std::env::temp_dir().join(format!("cam-gui-store-test-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let store = Store::new(directory.clone());
        let mut doc =
            crate::app::Document::new(crate::session::open(crate::session::FLOWER).unwrap());
        assert_eq!(store.save(None, doc.snapshot()).unwrap(), 1);
        doc.edit(0, "1.2".into()).unwrap();
        assert!(
            store
                .save(None, doc.snapshot())
                .unwrap_err()
                .contains("Revision conflict")
        );
        assert_eq!(store.load().unwrap().unwrap().revision, 1);
        let path = directory.join("session.json");
        let old = fs::read(&path).unwrap();
        // A deny-delete handle deterministically rejects replacement on Windows.
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let locked = File::options()
                .read(true)
                .share_mode(1)
                .open(&path)
                .unwrap();
            assert!(store.save(Some(1), doc.snapshot()).is_err());
            assert_eq!(fs::read(&path).unwrap(), old);
            drop(locked);
        }
        assert_eq!(store.save(Some(1), doc.snapshot()).unwrap(), 2);
        assert_ne!(fs::read(&path).unwrap(), old);
        assert_eq!(store.load().unwrap().unwrap().snapshot.draft, doc.raw);
        fs::remove_file(path).unwrap();
        fs::remove_file(directory.join("session.lock")).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
