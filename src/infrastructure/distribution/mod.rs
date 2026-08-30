//! Portable application packaging.

use crate::error::{Result, ScanError};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const PORTABLE_ROOT: &str = "open-scanline";
const PROJECT_README: &[u8] = include_bytes!("../../../README.md");
const PROJECT_LICENSE: &[u8] = include_bytes!("../../../LICENSE");
const THIRD_PARTY_NOTICES: &[u8] = include_bytes!("../../../THIRD_PARTY_NOTICES.md");
const RUST_DEPENDENCY_LICENSES: &[u8] =
    include_bytes!("../../../assets/licenses/RUST_DEPENDENCIES_ALL_FEATURES.md");
const RUST_DEPENDENCY_LICENSES_NAME: &str = "licenses/RUST_DEPENDENCIES_ALL_FEATURES.md";
static TEMP_OUTPUT_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagingOptions {
    pub binary: PathBuf,
    pub out: PathBuf,
}

fn package_error(context: &str, error: impl std::fmt::Display) -> ScanError {
    ScanError::Other(format!("{context}: {error}"))
}

struct TemporaryOutput {
    path: PathBuf,
}

impl TemporaryOutput {
    fn create(out: &Path) -> Result<(Self, File)> {
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
                    return Err(package_error("could not create temporary archive", error))
                }
            }
        }

        Err(ScanError::Other(
            "could not create a unique temporary archive".into(),
        ))
    }

    fn publish(self, out: &Path) -> Result<()> {
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

fn valid_binary_name(binary_name: &str) -> bool {
    !binary_name.is_empty()
        && binary_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(unix)]
fn files_have_same_identity(binary: &Path, out: &Path) -> std::io::Result<bool> {
    use std::os::unix::fs::MetadataExt;

    let binary = fs::metadata(binary)?;
    let out = fs::metadata(out)?;
    Ok(binary.dev() == out.dev() && binary.ino() == out.ino())
}

#[cfg(windows)]
fn files_have_same_identity(binary: &Path, out: &Path) -> std::io::Result<bool> {
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
        // SAFETY: `file` is an open Windows file handle and `information` points to writable
        // storage with the exact layout required by GetFileInformationByHandle.
        let success =
            unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) };
        if success == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: a successful call initializes every field in FileInformation.
        let information = unsafe { information.assume_init() };
        Ok((
            information.volume_serial_number,
            information.file_index_high,
            information.file_index_low,
        ))
    }

    Ok(identity(&File::open(binary)?)? == identity(&File::open(out)?)?)
}

#[cfg(not(any(unix, windows)))]
fn files_have_same_identity(_binary: &Path, _out: &Path) -> std::io::Result<bool> {
    Ok(false)
}

fn output_aliases_binary(binary: &Path, out: &Path) -> Result<bool> {
    match fs::metadata(out) {
        Ok(_) => {
            if files_have_same_identity(binary, out)
                .map_err(|error| package_error("could not inspect package output", error))?
            {
                return Ok(true);
            }
            Ok(fs::canonicalize(binary)? == fs::canonicalize(out)?)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(package_error("could not inspect package output", error)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BinaryFingerprint {
    size: u64,
    sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceStamp {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

fn source_stamp(metadata: &fs::Metadata) -> SourceStamp {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    SourceStamp {
        len: metadata.len(),
        modified: metadata.modified().ok(),
        #[cfg(unix)]
        device: metadata.dev(),
        #[cfg(unix)]
        inode: metadata.ino(),
    }
}

fn hash_reader(reader: &mut File) -> Result<BinaryFingerprint> {
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size = size
            .checked_add(read as u64)
            .ok_or_else(|| ScanError::Other("package binary is too large".into()))?;
    }
    Ok(BinaryFingerprint {
        size,
        sha256: hasher.finalize().into(),
    })
}

struct PrivateSnapshot {
    path: PathBuf,
    file: File,
}

impl PrivateSnapshot {
    fn file_mut(&mut self) -> &mut File {
        &mut self.file
    }
}

impl Drop for PrivateSnapshot {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn private_snapshot() -> Result<PrivateSnapshot> {
    let directory = std::env::temp_dir();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    for attempt in 0..128_u32 {
        let path = directory.join(format!(
            ".open-scanline-binary-snapshot-{nonce}-{}-{attempt}",
            std::process::id()
        ));
        let mut options = File::options();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => return Ok(PrivateSnapshot { path, file }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(package_error(
                    "could not create private binary snapshot",
                    error,
                ))
            }
        }
    }
    Err(ScanError::Other(
        "could not create a unique private binary snapshot".into(),
    ))
}

fn source_is_executable(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        true
    }
}

fn copy_binary_and_fingerprint<F>(
    zip: &mut ZipWriter<File>,
    binary: &Path,
    binary_name: &str,
    executable: SimpleFileOptions,
    on_snapshot_chunk: &mut F,
) -> Result<BinaryFingerprint>
where
    F: FnMut(u64),
{
    let mut source = File::open(binary)?;
    let metadata = source.metadata()?;
    let before = source_stamp(&metadata);
    if !source_is_executable(&metadata) {
        return Err(ScanError::Invalid(format!(
            "package binary is not executable: {}",
            binary.display()
        )));
    }

    zip.start_file(format!("{PORTABLE_ROOT}/{binary_name}"), executable)
        .map_err(|error| package_error("could not add portable binary", error))?;

    let mut snapshot = private_snapshot()?;
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        snapshot.file_mut().write_all(&buffer[..read])?;
        copied = copied
            .checked_add(read as u64)
            .ok_or_else(|| ScanError::Other("package binary is too large".into()))?;
        on_snapshot_chunk(copied);
    }
    snapshot.file_mut().flush()?;
    snapshot.file_mut().rewind()?;
    let fingerprint = hash_reader(snapshot.file_mut())?;
    let after = source_stamp(&source.metadata()?);
    let mut verification = File::open(binary)?;
    let verify_before = source_stamp(&verification.metadata()?);
    let verified = hash_reader(&mut verification)?;
    let verify_after = source_stamp(&verification.metadata()?);
    if before != after
        || after != verify_before
        || verify_before != verify_after
        || fingerprint != verified
    {
        return Err(ScanError::Other(format!(
            "package binary changed during packaging: {}",
            binary.display()
        )));
    }
    snapshot.file_mut().rewind()?;
    std::io::copy(snapshot.file_mut(), zip)?;

    Ok(fingerprint)
}

fn write_binary_and_launchers<F>(
    zip: &mut ZipWriter<File>,
    binary: &Path,
    binary_name: &str,
    regular: SimpleFileOptions,
    on_snapshot_chunk: &mut F,
) -> Result<BinaryFingerprint>
where
    F: FnMut(u64),
{
    let executable = regular.unix_permissions(0o755);
    let fingerprint =
        copy_binary_and_fingerprint(zip, binary, binary_name, executable, on_snapshot_chunk)?;

    let run_sh = format!(
        "#!/usr/bin/env sh\nROOT=$(CDPATH='' cd -- \"$(dirname -- \"$0\")\" && pwd)\nexec \"$ROOT/{binary_name}\" \"$@\"\n"
    );
    zip.start_file(format!("{PORTABLE_ROOT}/run.sh"), executable)
        .map_err(|error| package_error("could not add Unix launcher", error))?;
    zip.write_all(run_sh.as_bytes())?;

    let run_bat = format!("@echo off\r\n\"%~dp0{binary_name}\" %*\r\n");
    zip.start_file(format!("{PORTABLE_ROOT}/run.bat"), regular)
        .map_err(|error| package_error("could not add Windows launcher", error))?;
    zip.write_all(run_bat.as_bytes())?;
    Ok(fingerprint)
}

fn write_documentation_and_manifest(
    zip: &mut ZipWriter<File>,
    _root: &Path,
    binary_name: &str,
    regular: SimpleFileOptions,
) -> Result<()> {
    zip.start_file(format!("{PORTABLE_ROOT}/README.md"), regular)
        .map_err(|error| package_error("could not add package documentation", error))?;
    zip.write_all(PROJECT_README)?;
    zip.start_file(format!("{PORTABLE_ROOT}/LICENSE"), regular)
        .map_err(|error| package_error("could not add project license", error))?;
    zip.write_all(PROJECT_LICENSE)?;
    zip.start_file(format!("{PORTABLE_ROOT}/THIRD_PARTY_NOTICES.md"), regular)
        .map_err(|error| package_error("could not add third-party notices", error))?;
    zip.write_all(THIRD_PARTY_NOTICES)?;
    zip.start_file(
        format!("{PORTABLE_ROOT}/{RUST_DEPENDENCY_LICENSES_NAME}"),
        regular,
    )
    .map_err(|error| package_error("could not add Rust dependency licenses", error))?;
    zip.write_all(RUST_DEPENDENCY_LICENSES)?;
    zip.start_file(format!("{PORTABLE_ROOT}/PORTABLE.txt"), regular)
        .map_err(|error| package_error("could not add package manifest", error))?;
    writeln!(
        zip,
        "app=open-scanline\nversion={}\nbinary={binary_name}\npackaging_host_os={}\npackaging_host_arch={}",
        crate::VERSION,
        std::env::consts::OS,
        std::env::consts::ARCH,
    )?;
    Ok(())
}

/// Create a portable ZIP around one already-built, runnable executable.
pub fn build_portable(options: &PackagingOptions) -> Result<PathBuf> {
    build_portable_with_hook(options, |_| {})
}

fn build_portable_with_hook<F>(
    options: &PackagingOptions,
    mut on_snapshot_chunk: F,
) -> Result<PathBuf>
where
    F: FnMut(u64),
{
    if !options.binary.is_file() {
        return Err(ScanError::Invalid(format!(
            "package binary does not exist: {}",
            options.binary.display()
        )));
    }
    let binary_name = options
        .binary
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| ScanError::Invalid("package binary has no file name".into()))?;
    if !valid_binary_name(binary_name) {
        return Err(ScanError::Invalid(
            "package binary file name must use only ASCII letters, digits, '.', '_' or '-'".into(),
        ));
    }
    if output_aliases_binary(&options.binary, &options.out)? {
        return Err(ScanError::Invalid(format!(
            "package output aliases package binary: {}",
            options.out.display()
        )));
    }

    let (temporary, file) = TemporaryOutput::create(&options.out)?;

    let mut zip = ZipWriter::new(file);
    let regular = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let binary_fingerprint = write_binary_and_launchers(
        &mut zip,
        &options.binary,
        binary_name,
        regular,
        &mut on_snapshot_chunk,
    )?;
    write_documentation_and_manifest(
        &mut zip,
        Path::new(env!("CARGO_MANIFEST_DIR")),
        binary_name,
        regular,
    )?;
    let file = zip
        .finish()
        .map_err(|error| package_error("could not finish portable archive", error))?;
    file.sync_all()?;
    drop(file);
    verify_archive(&temporary.path, binary_name, &binary_fingerprint)?;
    temporary.publish(&options.out)?;
    Ok(options.out.clone())
}

fn verify_archive(path: &Path, binary_name: &str, expected: &BinaryFingerprint) -> Result<()> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| package_error("portable archive cannot be opened", error))?;
    let entry_name = format!("open-scanline/{binary_name}");
    let matching_entries = (0..archive.len())
        .filter_map(|index| {
            archive
                .by_index(index)
                .ok()
                .map(|entry| entry.name() == entry_name)
        })
        .filter(|matches| *matches)
        .count();
    if matching_entries != 1 {
        return Err(ScanError::Other(format!(
            "portable archive must contain exactly one binary entry named {entry_name}"
        )));
    }
    let mut entry = archive
        .by_name(&entry_name)
        .map_err(|error| package_error("portable archive is missing its binary", error))?;
    if entry.name() != entry_name {
        return Err(ScanError::Other(
            "portable archive binary entry name is invalid".into(),
        ));
    }
    if entry.size() != expected.size {
        return Err(ScanError::Other(format!(
            "portable archive binary size does not match source (expected {}, got {})",
            expected.size,
            entry.size()
        )));
    }
    if entry.unix_mode().map(|mode| mode & 0o777) != Some(0o755) {
        return Err(ScanError::Other(
            "portable archive binary is missing executable metadata".into(),
        ));
    }

    let mut hasher = Sha256::new();
    let mut read_total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = entry.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        read_total = read_total
            .checked_add(read as u64)
            .ok_or_else(|| ScanError::Other("portable archive binary is too large".into()))?;
        if read_total > expected.size {
            return Err(ScanError::Other(
                "portable archive binary exceeds its declared size".into(),
            ));
        }
        hasher.update(&buffer[..read]);
    }
    let observed_hash: [u8; 32] = hasher.finalize().into();
    if read_total != expected.size || observed_hash != expected.sha256 {
        return Err(ScanError::Other(
            "portable archive binary hash does not match source".into(),
        ));
    }
    drop(entry);

    verify_embedded_regular_file(&mut archive, "LICENSE", PROJECT_LICENSE, "project license")?;
    verify_embedded_regular_file(
        &mut archive,
        "THIRD_PARTY_NOTICES.md",
        THIRD_PARTY_NOTICES,
        "third-party notice",
    )?;
    verify_embedded_regular_file(
        &mut archive,
        RUST_DEPENDENCY_LICENSES_NAME,
        RUST_DEPENDENCY_LICENSES,
        "Rust dependency license bundle",
    )?;
    verify_embedded_regular_file(&mut archive, "README.md", PROJECT_README, "project README")?;
    Ok(())
}

fn verify_embedded_regular_file(
    archive: &mut ZipArchive<File>,
    relative_name: &str,
    expected_contents: &[u8],
    description: &str,
) -> Result<()> {
    let entry_name = format!("{PORTABLE_ROOT}/{relative_name}");
    let matching_entries = (0..archive.len())
        .filter_map(|index| {
            archive
                .by_index(index)
                .ok()
                .map(|entry| entry.name() == entry_name)
        })
        .filter(|matches| *matches)
        .count();
    if matching_entries != 1 {
        return Err(ScanError::Other(format!(
            "portable archive must contain exactly one {description} entry named {entry_name}"
        )));
    }
    let mut entry = archive.by_name(&entry_name).map_err(|error| {
        package_error(&format!("portable archive is missing {description}"), error)
    })?;
    if entry.size() != expected_contents.len() as u64
        || entry.unix_mode().map(|mode| mode & 0o777) != Some(0o644)
    {
        return Err(ScanError::Other(format!(
            "portable archive {description} metadata is invalid"
        )));
    }
    let mut contents = vec![0_u8; expected_contents.len()];
    entry.read_exact(&mut contents)?;
    if contents != expected_contents {
        return Err(ScanError::Other(format!(
            "portable archive {description} does not match the embedded contents"
        )));
    }
    Ok(())
}
