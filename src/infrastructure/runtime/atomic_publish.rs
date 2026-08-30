//! Same-directory atomic file publication for configuration-like data.

use crate::error::{Result, ScanError};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const CREATE_ATTEMPTS: usize = 128;

struct PendingFile {
    path: PathBuf,
    published: bool,
}

impl Drop for PendingFile {
    fn drop(&mut self) {
        if !self.published {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Write all bytes to a create-new sibling, flush them, and atomically publish
/// the completed file over `destination`.
pub(crate) fn write_file_atomic(destination: &Path, bytes: &[u8]) -> Result<()> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    if !parent.as_os_str().is_empty() {
        std::fs::create_dir_all(parent)?;
    }
    let (mut pending, mut file) = reserve_sibling(destination, parent)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    replace_file_atomic(&pending.path, destination)?;
    pending.published = true;
    sync_parent(parent);
    Ok(())
}

/// Return whether two output paths resolve to the same destination.
///
/// This covers lexical aliases, symlinked parent directories, symlinks, and
/// existing hard links. Missing leaf paths are resolved against their nearest
/// existing ancestor so collision checks can run before output directories are
/// created.
pub fn output_paths_alias(left: &Path, right: &Path) -> Result<bool> {
    if resolve_destination(left)? == resolve_destination(right)? {
        return Ok(true);
    }

    let left_exists = existing_metadata(left)?;
    let right_exists = existing_metadata(right)?;
    match (left_exists, right_exists) {
        (Some(_), Some(_)) => Ok(files_have_same_identity(left, right)?),
        _ => Ok(false),
    }
}

/// Reject output leaves that cannot be atomically replaced as regular files.
pub fn validate_output_leaf(path: &Path, label: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(()),
        Ok(_) => Err(ScanError::Invalid(format!(
            "{label} must be a regular file or a missing path: {}",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn existing_metadata(path: &Path) -> std::io::Result<Option<fs::Metadata>> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn resolve_destination(path: &Path) -> std::io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut ancestor = absolute.as_path();
    let mut missing = Vec::new();
    loop {
        match fs::canonicalize(ancestor) {
            Ok(mut resolved) => {
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return Ok(normalize_path(&resolved));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(name) = ancestor.file_name() else {
                    return Ok(normalize_path(&absolute));
                };
                missing.push(name.to_os_string());
                let Some(parent) = ancestor.parent() else {
                    return Ok(normalize_path(&absolute));
                };
                ancestor = parent;
            }
            Err(error) => return Err(error),
        }
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

#[cfg(unix)]
fn files_have_same_identity(left: &Path, right: &Path) -> std::io::Result<bool> {
    use std::os::unix::fs::MetadataExt;

    let left = fs::metadata(left)?;
    let right = fs::metadata(right)?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}

#[cfg(windows)]
fn files_have_same_identity(left: &Path, right: &Path) -> std::io::Result<bool> {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;

    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    struct FileInformation {
        attributes: u32,
        creation_time: FileTime,
        last_access_time: FileTime,
        last_write_time: FileTime,
        volume_serial_number: u32,
        file_size_high: u32,
        file_size_low: u32,
        number_of_links: u32,
        file_index_high: u32,
        file_index_low: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(file: *mut c_void, information: *mut FileInformation) -> i32;
    }

    fn identity(file: &File) -> std::io::Result<(u32, u32, u32)> {
        let mut information = std::mem::MaybeUninit::<FileInformation>::uninit();
        // SAFETY: `file` is open and `information` has the exact writable FFI layout.
        let success =
            unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) };
        if success == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: a successful call initialized the complete structure.
        let information = unsafe { information.assume_init() };
        Ok((
            information.volume_serial_number,
            information.file_index_high,
            information.file_index_low,
        ))
    }

    Ok(identity(&File::open(left)?)? == identity(&File::open(right)?)?)
}

#[cfg(not(any(unix, windows)))]
fn files_have_same_identity(_left: &Path, _right: &Path) -> std::io::Result<bool> {
    Ok(false)
}

fn reserve_sibling(destination: &Path, parent: &Path) -> Result<(PendingFile, File)> {
    let name = destination.file_name().ok_or_else(|| {
        ScanError::Invalid(format!(
            "output path has no file name: {}",
            destination.display()
        ))
    })?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    for _ in 0..CREATE_ATTEMPTS {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".{}.open-scanline-{}-{timestamp}-{sequence}.tmp",
            name.to_string_lossy(),
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => {
                return Ok((
                    PendingFile {
                        path,
                        published: false,
                    },
                    file,
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not reserve an atomic output file",
    )
    .into())
}

#[cfg(not(windows))]
pub(crate) fn replace_file_atomic(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(windows)]
pub(crate) fn replace_file_atomic(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let success = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if success == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn sync_parent(parent: &Path) {
    let _ = File::open(parent).and_then(|directory| directory.sync_all());
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) {}
