use super::create_output_parent;
use crate::error::{Result, ScanError};
use std::fs::OpenOptions;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const TEMP_CREATE_ATTEMPTS: u64 = 128;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A create-new sibling temporary that is removed unless it is published.
pub(in crate::infrastructure::media) struct OutputTemp {
    path: PathBuf,
    destination: PathBuf,
    published: bool,
}

impl OutputTemp {
    pub(in crate::infrastructure::media) fn path(&self) -> &Path {
        &self.path
    }

    pub(in crate::infrastructure::media) fn publish(mut self) -> Result<()> {
        crate::infrastructure::runtime::atomic_publish::replace_file_atomic(
            &self.path,
            &self.destination,
        )?;
        self.published = true;
        Ok(())
    }
}

impl Drop for OutputTemp {
    fn drop(&mut self) {
        if !self.published {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub(in crate::infrastructure::media) fn create_output_temp(
    destination: &Path,
) -> Result<OutputTemp> {
    create_output_parent(destination)?;
    let naming = OutputTempNaming::for_destination(destination)?;
    #[cfg(unix)]
    let output_mode = existing_output_mode_or_private(destination)?;

    for _ in 0..TEMP_CREATE_ATTEMPTS {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = naming.path(sequence);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => {
                #[cfg(unix)]
                if let Err(error) =
                    file.set_permissions(std::fs::Permissions::from_mode(output_mode))
                {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    return Err(error.into());
                }
                #[cfg(not(unix))]
                drop(file);
                return Ok(OutputTemp {
                    path,
                    destination: destination.to_path_buf(),
                    published: false,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }

    Err(ScanError::Other(format!(
        "could not reserve a unique temporary output beside {}",
        destination.display()
    )))
}

struct OutputTempNaming {
    parent: PathBuf,
    stem: String,
    extension: String,
    timestamp: u128,
}

impl OutputTempNaming {
    fn for_destination(destination: &Path) -> Result<Self> {
        let file_name = destination.file_name().ok_or_else(|| {
            ScanError::Invalid(format!(
                "output path has no file name: {}",
                destination.display()
            ))
        })?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        Ok(Self {
            parent: destination
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf(),
            stem: destination
                .file_stem()
                .unwrap_or(file_name)
                .to_string_lossy()
                .into_owned(),
            extension: destination
                .extension()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default(),
            timestamp,
        })
    }

    fn path(&self, sequence: u64) -> PathBuf {
        let suffix = if self.extension.is_empty() {
            String::new()
        } else {
            format!(".{}", self.extension)
        };
        self.parent.join(format!(
            ".{}.open-scanline-{}-{}-{sequence}{suffix}",
            self.stem,
            std::process::id(),
            self.timestamp,
        ))
    }
}

#[cfg(unix)]
fn existing_output_mode_or_private(destination: &Path) -> Result<u32> {
    match std::fs::metadata(destination) {
        Ok(metadata) => Ok(metadata.permissions().mode() & 0o7777),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0o600),
        Err(error) => Err(error.into()),
    }
}
