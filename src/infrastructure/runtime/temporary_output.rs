//! Private temporary output lifecycle for one acquisition attempt.

use crate::error::Result;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static ARTIFACT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// An isolated, per-acquisition output directory that is removed on every
/// return path. The directory is created atomically, so concurrent scans never
/// share an output path; on Unix its contents are private to the current user.
pub(crate) struct TemporaryOutput {
    directory: std::path::PathBuf,
    path: std::path::PathBuf,
}

impl TemporaryOutput {
    pub(crate) fn new(adapter: &str, extension: &str) -> Result<Self> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for _ in 0..128 {
            let sequence = ARTIFACT_COUNTER.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                ".open-scanline-{adapter}-{}-{timestamp}-{sequence}",
                std::process::id(),
            ));
            match create_private_directory(&directory) {
                Ok(()) => {
                    return Ok(Self {
                        path: directory.join(format!("output.{extension}")),
                        directory,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate unique scanner output directory",
        )
        .into())
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }
}

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn create_private_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(path)
    }
    #[cfg(not(unix))]
    fs::DirBuilder::new().create(path)
}
