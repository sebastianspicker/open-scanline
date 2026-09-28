use super::launch;
use super::paths::{self, worker_unavailable};
use super::OnnxInferenceOptions;
use crate::error::{Result, ScanError};
use crate::infrastructure::runtime::{run_command, CommandSpec, TemporaryOutput};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

// All-feature debug binaries can exceed 600 MiB on Linux because they retain
// symbols for the GUI and inference stacks. Keep the staging operation bounded
// while accepting those supported development builds.
const MAX_ONNX_WORKER_BYTES: u64 = 768 * 1024 * 1024;

/// A selected ONNX worker whose identity is pinned at construction.
#[derive(Debug)]
pub(crate) struct OnnxWorker {
    executable: PathBuf,
    identity: WorkerIdentity,
    staged: StagedWorkerExecutable,
}

#[derive(Debug)]
struct StagedWorkerExecutable {
    output: TemporaryOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkerIdentity {
    length: u64,
    modified: std::time::SystemTime,
    sha256: [u8; 32],
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl WorkerIdentity {
    fn capture(executable: &Path) -> Result<Self> {
        let mut file = std::fs::File::open(executable)
            .map_err(|error| unavailable(executable, "inspect", error))?;
        let metadata = file
            .metadata()
            .map_err(|error| unavailable(executable, "inspect", error))?;
        validate_worker_metadata(&metadata, executable)?;
        let sha256 = hash_worker(&mut file, executable, metadata.len())?;
        let modified = metadata
            .modified()
            .map_err(|error| unavailable(executable, "inspect modification time", error))?;
        identity_from_metadata(metadata, modified, sha256)
    }

    fn has_same_content(&self, other: &Self) -> bool {
        self.length == other.length && self.sha256 == other.sha256
    }
}

fn validate_worker_metadata(metadata: &std::fs::Metadata, executable: &Path) -> Result<()> {
    if metadata.is_file() && metadata.len() > 0 && metadata.len() <= MAX_ONNX_WORKER_BYTES {
        return Ok(());
    }
    Err(worker_unavailable(format!(
        "ONNX worker executable {} is empty, not a regular file, or exceeds the {}-byte limit (actual size: {} bytes)",
        executable.display(),
        MAX_ONNX_WORKER_BYTES,
        metadata.len()
    )))
}

fn hash_worker(file: &mut std::fs::File, executable: &Path, length: u64) -> Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    let mut bounded = file.by_ref().take(MAX_ONNX_WORKER_BYTES + 1);
    loop {
        let count = bounded
            .read(&mut buffer)
            .map_err(|error| unavailable(executable, "hash", error))?;
        if count == 0 {
            break;
        }
        copied += count as u64;
        hasher.update(&buffer[..count]);
    }
    if copied != length {
        return Err(worker_unavailable(format!(
            "ONNX worker executable {} changed while it was hashed",
            executable.display()
        )));
    }
    Ok(hasher.finalize().into())
}

#[cfg(unix)]
fn identity_from_metadata(
    metadata: std::fs::Metadata,
    modified: std::time::SystemTime,
    sha256: [u8; 32],
) -> Result<WorkerIdentity> {
    use std::os::unix::fs::MetadataExt;
    Ok(WorkerIdentity {
        length: metadata.len(),
        modified,
        sha256,
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(not(unix))]
fn identity_from_metadata(
    metadata: std::fs::Metadata,
    modified: std::time::SystemTime,
    sha256: [u8; 32],
) -> Result<WorkerIdentity> {
    Ok(WorkerIdentity {
        length: metadata.len(),
        modified,
        sha256,
    })
}

impl StagedWorkerExecutable {
    fn copy_from(source: &Path, source_identity: &WorkerIdentity) -> Result<Self> {
        let extension = source
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("bin");
        let output = TemporaryOutput::new("onnx-worker-image", extension)?;
        stage_worker_image(source, &output)?;
        if !WorkerIdentity::capture(output.path())?.has_same_content(source_identity) {
            return Err(worker_unavailable(format!(
                "ONNX worker executable {} changed while its private execution image was created",
                source.display()
            )));
        }
        Ok(Self { output })
    }
    fn path(&self) -> &Path {
        self.output.path()
    }
}

#[cfg(windows)]
fn stage_worker_image(source: &Path, output: &TemporaryOutput) -> Result<()> {
    // A fresh file avoids copying the source's read-only attribute,
    // so the temporary directory can be removed on drop.
    let mut source_file =
        std::fs::File::open(source).map_err(|error| unavailable(source, "stage", error))?;
    let mut staged_file = std::fs::File::create(output.path())
        .map_err(|error| unavailable(source, "stage", error))?;
    std::io::copy(&mut source_file, &mut staged_file)
        .map_err(|error| unavailable(source, "stage", error))?;
    Ok(())
}

#[cfg(not(windows))]
fn stage_worker_image(source: &Path, output: &TemporaryOutput) -> Result<()> {
    std::fs::copy(source, output.path()).map_err(|error| unavailable(source, "stage", error))?;
    set_staged_permissions(output)
}

#[cfg(not(windows))]
fn set_staged_permissions(output: &TemporaryOutput) -> Result<()> {
    let mut permissions = std::fs::metadata(output.path())?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o500);
    }
    #[cfg(not(unix))]
    permissions.set_readonly(true);
    std::fs::set_permissions(output.path(), permissions)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(output.directory(), std::fs::Permissions::from_mode(0o500))?;
    }
    Ok(())
}

#[cfg(unix)]
impl Drop for StagedWorkerExecutable {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(
            self.output.directory(),
            std::fs::Permissions::from_mode(0o700),
        );
    }
}

impl OnnxWorker {
    pub(crate) fn from_executable(executable: impl AsRef<Path>) -> Result<Self> {
        let requested = executable.as_ref();
        let executable = requested.canonicalize().map_err(|error| {
            ScanError::Invalid(format!(
                "ONNX worker executable not found: {} ({error})",
                requested.display()
            ))
        })?;
        let identity = WorkerIdentity::capture(&executable)?;
        let _containment = TrustedWorkerHandle::acquire(&executable, "ONNX worker executable")?;
        validate_worker_executable(&executable)?;
        let staged = StagedWorkerExecutable::copy_from(&executable, &identity)?;
        let _staged_containment =
            TrustedWorkerHandle::acquire(staged.path(), "private ONNX worker execution image")?;
        Ok(Self {
            executable,
            identity,
            staged,
        })
    }

    pub(crate) fn run(
        &self,
        input: &Path,
        model: &Path,
        options: &OnnxInferenceOptions,
    ) -> Result<super::super::OnnxReport> {
        paths::validate_onnx_paths(input, model)?;
        let (_source, _staged) = self.validate_before_run()?;
        launch::run_isolated_onnx_with_worker(
            self.staged.path(),
            paths::working_directory(&self.executable)?,
            input,
            model,
            options,
        )
    }

    fn validate_before_run(&self) -> Result<(TrustedWorkerHandle, TrustedWorkerHandle)> {
        let canonical = self.executable.canonicalize().map_err(|error| {
            worker_unavailable(format!(
                "ONNX worker executable {} is unavailable: {error}",
                self.executable.display()
            ))
        })?;
        if canonical != self.executable {
            return Err(worker_unavailable(format!(
                "ONNX worker executable {} no longer resolves to its selected path",
                self.executable.display()
            )));
        }
        if WorkerIdentity::capture(&self.executable)? != self.identity {
            return Err(worker_unavailable(format!(
                "ONNX worker executable {} was replaced after runtime construction",
                self.executable.display()
            )));
        }
        let _ = paths::working_directory(&self.executable)?;
        let source = TrustedWorkerHandle::acquire(&self.executable, "ONNX worker executable")?;
        let staged = TrustedWorkerHandle::acquire(
            self.staged.path(),
            "private ONNX worker execution image",
        )?;
        Ok((source, staged))
    }
}

fn validate_worker_executable(executable: &Path) -> Result<()> {
    let expected = format!("open-scanline {}", env!("CARGO_PKG_VERSION"));
    let cancelled = std::sync::Mutex::new(false);
    let output = run_command(
        &CommandSpec {
            program: executable.to_string_lossy().into_owned(),
            args: vec!["--version".into()],
        },
        std::time::Duration::from_secs(5),
        &cancelled,
        "ONNX worker version probe",
        "ONNX worker version probe cancelled",
    )?;
    let reported = std::str::from_utf8(&output.stdout)
        .ok()
        .map(str::trim)
        .unwrap_or_default();
    if output.success && reported == expected {
        Ok(())
    } else {
        Err(ScanError::Unsupported(format!("ONNX worker executable is not the version-matched Open Scanline worker (expected '{expected}')")))
    }
}

fn unavailable(executable: &Path, action: &str, error: std::io::Error) -> ScanError {
    worker_unavailable(format!(
        "could not {action} ONNX worker executable {}: {error}",
        executable.display()
    ))
}

#[cfg(not(windows))]
struct TrustedWorkerHandle;
#[cfg(not(windows))]
impl TrustedWorkerHandle {
    fn acquire(_path: &Path, _role: &str) -> Result<Self> {
        Ok(Self)
    }
}

#[cfg(windows)]
struct TrustedWorkerHandle {
    _file: std::fs::File,
}
#[cfg(windows)]
impl TrustedWorkerHandle {
    fn acquire(path: &Path, role: &str) -> Result<Self> {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 1;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|error| {
                worker_unavailable(format!(
                    "could not retain {role} {} without write/delete sharing: {error}",
                    path.display()
                ))
            })?;
        let metadata = file.metadata().map_err(|error| {
            worker_unavailable(format!(
                "could not inspect retained {role} {}: {error}",
                path.display()
            ))
        })?;
        if is_reparse_or_symlink(&metadata) || !metadata.is_file() || metadata.len() == 0 {
            return Err(worker_unavailable(format!(
                "retained {role} {} is empty, non-regular, or a reparse point",
                path.display()
            )));
        }
        if !has_exactly_one_hard_link(&file).map_err(|error| {
            worker_unavailable(format!(
                "could not inspect retained {role} {} link count: {error}",
                path.display()
            ))
        })? {
            return Err(worker_unavailable(format!(
                "retained {role} {} must have exactly one hard link",
                path.display()
            )));
        }
        Ok(Self { _file: file })
    }
}

#[cfg(windows)]
fn is_reparse_or_symlink(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
}
#[cfg(windows)]
fn has_exactly_one_hard_link(file: &std::fs::File) -> std::io::Result<bool> {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    struct FileInformation {
        file_attributes: u32,
        creation_time_low: u32,
        creation_time_high: u32,
        last_access_time_low: u32,
        last_access_time_high: u32,
        last_write_time_low: u32,
        last_write_time_high: u32,
        volume_serial_number: u32,
        file_size_high: u32,
        file_size_low: u32,
        number_of_links: u32,
        file_index_high: u32,
        file_index_low: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(file: *mut c_void, information: *mut c_void) -> i32;
    }
    let mut information = std::mem::MaybeUninit::<FileInformation>::uninit();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr().cast()) }
        == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { information.assume_init() }.number_of_links == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_worker_image_is_removed_when_its_owner_drops() {
        let source = TemporaryOutput::new("onnx-worker-stage-source", "bin").unwrap();
        std::fs::write(source.path(), b"open-scanline-worker-test-image").unwrap();
        let identity = WorkerIdentity::capture(source.path()).unwrap();
        let staged = StagedWorkerExecutable::copy_from(source.path(), &identity).unwrap();
        let directory = staged.output.directory().to_path_buf();
        assert!(staged.path().is_file());
        drop(staged);
        assert!(!directory.exists());
    }
}
