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
    /// Final permission bits, applied to the temporary just before publication.
    #[cfg(unix)]
    mode: u32,
}

impl OutputTemp {
    pub(in crate::infrastructure::media) fn path(&self) -> &Path {
        &self.path
    }

    pub(in crate::infrastructure::media) fn publish(mut self) -> Result<()> {
        #[cfg(unix)]
        std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(self.mode))?;
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
    let output_mode = existing_output_mode_or_default(destination)?;

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
                // The temporary stays 0600 until `publish` applies the final mode.
                drop(file);
                return Ok(OutputTemp {
                    path,
                    destination: destination.to_path_buf(),
                    published: false,
                    #[cfg(unix)]
                    mode: output_mode,
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

/// Mode for the published file: an existing destination keeps its mode; a
/// new file gets what a normally created file would (`0o666 & !umask`).
#[cfg(unix)]
fn existing_output_mode_or_default(destination: &Path) -> Result<u32> {
    match std::fs::metadata(destination) {
        Ok(metadata) => Ok(metadata.permissions().mode() & 0o7777),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(new_file_mode(process_umask()))
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn new_file_mode(umask: u32) -> u32 {
    0o666 & !umask
}

/// The process umask, read once and cached.
///
/// POSIX offers no read-only query, so this sets the umask to 0 and restores
/// the previous value immediately. A file created by another thread in that
/// brief window would get an unmasked mode; caching keeps the window to a
/// single instant per process.
#[cfg(unix)]
fn process_umask() -> u32 {
    static UMASK: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *UMASK.get_or_init(|| {
        // SAFETY: `umask` has no memory-safety preconditions; the previous
        // value is restored immediately.
        let previous = unsafe { libc::umask(0) };
        unsafe { libc::umask(previous) };
        u32::from(previous)
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn new_file_mode_applies_the_umask_to_0666() {
        assert_eq!(new_file_mode(0o022), 0o644);
        assert_eq!(new_file_mode(0o077), 0o600);
        assert_eq!(new_file_mode(0o002), 0o664);
        assert_eq!(new_file_mode(0), 0o666);
    }
}
