use super::package_error;
use crate::error::{Result, ScanError};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_OUTPUT_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub(super) struct TemporaryOutput {
    pub(super) path: PathBuf,
}

impl TemporaryOutput {
    pub(super) fn create(out: &Path) -> Result<(Self, File)> {
        let parent = out
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let file_name = out
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| ScanError::Invalid("package output has no file name".into()))?;
        fs::create_dir_all(parent)?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);

        for _ in 0..128 {
            let counter = TEMP_OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(
                ".{file_name}.open-scanline-package-{nonce}-{}-{counter}.tmp",
                std::process::id()
            ));
            match File::options().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((Self { path }, file)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(package_error("could not create temporary archive", error));
                }
            }
        }

        Err(ScanError::Other(
            "could not create a unique temporary archive".into(),
        ))
    }

    pub(super) fn publish(self, out: &Path) -> Result<()> {
        publish_temporary_file(&self.path, out)
            .map_err(|error| package_error("could not publish portable archive", error))
    }
}

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(not(windows))]
fn publish_temporary_file(temporary: &Path, out: &Path) -> std::io::Result<()> {
    fs::rename(temporary, out)
}

#[cfg(windows)]
fn publish_temporary_file(temporary: &Path, out: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }

    let existing = temporary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replacement = out
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both vectors are stable, NUL-terminated UTF-16 strings for the duration of the
    // call. The temporary archive and destination are siblings, so this is a same-volume move.
    let success = unsafe {
        MoveFileExW(
            existing.as_ptr(),
            replacement.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if success == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
