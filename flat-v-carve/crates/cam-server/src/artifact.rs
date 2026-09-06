//! Service-created temporary plan files. HTTP accepts task IDs, never these paths.
use axum::body::{Body, Bytes};
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::io::AsyncReadExt;

#[derive(Debug)]
pub struct PlanFile {
    path: PathBuf,
    // On Windows the OS also cleans up after unexpected service termination.
    _reservation: File,
}

impl PlanFile {
    pub fn create() -> io::Result<Arc<Self>> {
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(io::Error::other)?;
        let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir().join(format!("flat-v-carve-{id}.plan.json"));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x0400_0000); // FILE_FLAG_DELETE_ON_CLOSE
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let reservation = options.open(&path)?;
        Ok(Arc::new(Self {
            path,
            _reservation: reservation,
        }))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn byte_len(&self) -> io::Result<u64> {
        Ok(File::open(&self.path)?.metadata()?.len())
    }

    // The stream owns a lease, so result eviction cannot delete an active
    // download. Read at most 64 KiB per poll instead of cloning the full plan.
    pub async fn body(self: Arc<Self>) -> io::Result<Body> {
        let file = tokio::fs::File::open(&self.path).await?;
        let stream =
            futures_util::stream::try_unfold((file, self), |(mut file, lease)| async move {
                let mut bytes = vec![0; 64 * 1024];
                let count = file.read(&mut bytes).await?;
                if count == 0 {
                    return Ok::<_, io::Error>(None);
                }
                bytes.truncate(count);
                Ok(Some((Bytes::from(bytes), (file, lease))))
            });
        Ok(Body::from_stream(stream))
    }
}

impl Drop for PlanFile {
    fn drop(&mut self) {
        // Only the exact create_new file is removed; no directory traversal or
        // recursive cleanup. Source leases outlive queued/running child workers.
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn leases_keep_evicted_sources_and_downloads_alive_then_remove_the_file() {
        let artifact = PlanFile::create().unwrap();
        let path = artifact.path().to_owned();
        let bytes = vec![b'x'; 150_000];
        fs::write(&path, &bytes).unwrap();
        let queued_worker = artifact.clone();
        let body = artifact.clone().body().await.unwrap();
        drop(artifact);
        assert!(path.exists());
        drop(queued_worker);
        assert!(path.exists());
        assert_eq!(to_bytes(body, bytes.len()).await.unwrap().as_ref(), bytes);
        assert!(!path.exists());
    }

    #[test]
    fn failed_or_cancelled_partial_artifacts_are_removed() {
        let artifact = PlanFile::create().unwrap();
        let path = artifact.path().to_owned();
        fs::write(&path, b"{partial").unwrap();
        drop(artifact);
        assert!(!path.exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_process_exit_removes_plans_without_rust_destructors() {
        use std::{io::Write, os::windows::process::CommandExt, process::Command};
        const CHILD: &str = "CAM_TEST_ARTIFACT_PROCESS_EXIT";
        if std::env::var_os(CHILD).is_some() {
            let artifact = PlanFile::create().unwrap();
            println!("CAM_TEST_ARTIFACT={}", artifact.path().display());
            std::io::stdout().flush().unwrap();
            std::process::exit(0); // Deliberately bypass PlanFile::drop.
        }
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "artifact::tests::windows_process_exit_removes_plans_without_rust_destructors",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .creation_flags(0x0800_0000)
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        let path = stdout
            .split("CAM_TEST_ARTIFACT=")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap();
        assert!(!Path::new(path).exists());
    }
}
