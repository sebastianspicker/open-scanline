//! Portable application packaging.

use crate::core::{Result, ScanError};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const PORTABLE_ROOT: &str = "open-scanline";
const PROJECT_README: &[u8] = include_bytes!("../README.md");
const PROJECT_LICENSE: &[u8] = include_bytes!("../LICENSE");
const THIRD_PARTY_NOTICES: &[u8] = include_bytes!("../THIRD_PARTY_NOTICES.md");
const RUST_DEPENDENCY_LICENSES: &[u8] =
    include_bytes!("../assets/licenses/RUST_DEPENDENCIES_ALL_FEATURES.md");
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "open-scanline-packaging-{label}-{}-{nonce}-{counter}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    fn runnable_binary(dir: &Path, name: &str, invocation_log: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let binary = dir.join(name);
        fs::write(
            &binary,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$1\" >> '{}'\n[ \"$1\" = '--version' ]\n",
                invocation_log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        binary
    }

    fn assert_archive_entry(
        archive: &mut ZipArchive<File>,
        name: &str,
        expected_content: &[u8],
        expected_permissions: u32,
    ) {
        let mut entry = archive.by_name(name).unwrap();
        let mut content = Vec::new();
        entry.read_to_end(&mut content).unwrap();
        assert_eq!(content, expected_content);
        assert_eq!(entry.compression(), CompressionMethod::Deflated);
        assert_eq!(entry.unix_mode().unwrap() & 0o777, expected_permissions);
    }

    fn expected_archive_names(binary_name: &str, _root: &Path) -> Vec<String> {
        let mut names = vec![
            format!("{PORTABLE_ROOT}/{binary_name}"),
            format!("{PORTABLE_ROOT}/run.sh"),
            format!("{PORTABLE_ROOT}/run.bat"),
            format!("{PORTABLE_ROOT}/README.md"),
        ];
        names.push(format!("{PORTABLE_ROOT}/LICENSE"));
        names.push(format!("{PORTABLE_ROOT}/THIRD_PARTY_NOTICES.md"));
        names.push(format!("{PORTABLE_ROOT}/{RUST_DEPENDENCY_LICENSES_NAME}"));
        names.push(format!("{PORTABLE_ROOT}/PORTABLE.txt"));
        names
    }

    fn assert_documentation_entries(archive: &mut ZipArchive<File>, _root: &Path) {
        assert_archive_entry(
            archive,
            &format!("{PORTABLE_ROOT}/README.md"),
            PROJECT_README,
            0o644,
        );
        assert_archive_entry(
            archive,
            &format!("{PORTABLE_ROOT}/LICENSE"),
            PROJECT_LICENSE,
            0o644,
        );
        assert_archive_entry(
            archive,
            &format!("{PORTABLE_ROOT}/THIRD_PARTY_NOTICES.md"),
            THIRD_PARTY_NOTICES,
            0o644,
        );
        assert_archive_entry(
            archive,
            &format!("{PORTABLE_ROOT}/{RUST_DEPENDENCY_LICENSES_NAME}"),
            RUST_DEPENDENCY_LICENSES,
            0o644,
        );
    }

    fn assert_portable_manifest(archive: &mut ZipArchive<File>, binary_name: &str) {
        assert_archive_entry(
            archive,
            &format!("{PORTABLE_ROOT}/PORTABLE.txt"),
            format!(
                "app=open-scanline\nversion={}\nbinary={binary_name}\npackaging_host_os={}\npackaging_host_arch={}\n",
                crate::VERSION,
                std::env::consts::OS,
                std::env::consts::ARCH,
            )
            .as_bytes(),
            0o644,
        );
    }

    fn assert_no_temporary_archives(dir: &Path) {
        let temporary_archives = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().contains(".open-scanline-package-"))
            .collect::<Vec<_>>();
        assert!(
            temporary_archives.is_empty(),
            "temporary archives were left behind: {temporary_archives:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn portable_archive_has_stable_layout_content_and_permissions() {
        let dir = TestDirectory::new("archive");
        let log = dir.0.join("version-invocations.log");
        let binary = runnable_binary(&dir.0, "scan-fixture", &log);
        let binary_bytes = fs::read(&binary).unwrap();
        let out = dir.0.join("portable.zip");

        assert_eq!(
            build_portable(&PackagingOptions {
                binary: binary.clone(),
                out: out.clone(),
            })
            .unwrap(),
            out
        );

        let mut archive = ZipArchive::new(File::open(&out).unwrap()).unwrap();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let names = (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, expected_archive_names("scan-fixture", &root));

        assert_archive_entry(
            &mut archive,
            "open-scanline/scan-fixture",
            &binary_bytes,
            0o755,
        );
        assert_archive_entry(
            &mut archive,
            "open-scanline/run.sh",
            b"#!/usr/bin/env sh\nROOT=$(CDPATH='' cd -- \"$(dirname -- \"$0\")\" && pwd)\nexec \"$ROOT/scan-fixture\" \"$@\"\n",
            0o755,
        );
        assert_archive_entry(
            &mut archive,
            "open-scanline/run.bat",
            b"@echo off\r\n\"%~dp0scan-fixture\" %*\r\n",
            0o644,
        );

        assert_documentation_entries(&mut archive, &root);
        assert_portable_manifest(&mut archive, "scan-fixture");
        assert!(
            !log.exists(),
            "default packaging must not invoke the supplied executable"
        );
    }

    #[cfg(unix)]
    #[test]
    fn default_packaging_does_not_invoke_a_marker_executable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TestDirectory::new("marker-not-invoked");
        let marker = dir.0.join("invoked-marker");
        let binary = dir.0.join("marker-fixture");
        fs::write(
            &binary,
            format!("#!/bin/sh\nprintf invoked > '{}'\n", marker.display()),
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        let out = dir.0.join("portable.zip");

        build_portable(&PackagingOptions {
            binary,
            out: out.clone(),
        })
        .unwrap();
        assert!(out.is_file());
        assert!(!marker.exists());
    }

    #[cfg(unix)]
    #[test]
    fn same_length_source_mutation_is_rejected_without_replacing_output() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TestDirectory::new("same-length-mutation");
        let binary = dir.0.join("changing-scanner");
        let original = vec![b'A'; 256 * 1024];
        let replacement = vec![b'B'; original.len()];
        fs::write(&binary, &original).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        let out = dir.0.join("portable.zip");
        fs::write(&out, b"existing archive").unwrap();
        let mut mutated = false;

        let error = build_portable_with_hook(
            &PackagingOptions {
                binary: binary.clone(),
                out: out.clone(),
            },
            |copied| {
                if !mutated && copied >= 64 * 1024 {
                    fs::write(&binary, &replacement).unwrap();
                    mutated = true;
                }
            },
        )
        .unwrap_err();

        assert!(mutated);
        assert!(error.to_string().contains("changed during packaging"));
        assert_eq!(fs::read(&out).unwrap(), b"existing archive");
        assert_eq!(fs::read(&binary).unwrap(), replacement);
        assert_no_temporary_archives(&dir.0);
    }

    #[cfg(unix)]
    #[test]
    fn corrupted_archive_binary_is_rejected_without_invocation() {
        let dir = TestDirectory::new("corrupt-archive");
        let marker = dir.0.join("invoked-marker");
        let expected_bytes = b"expected source bytes";
        let corrupted = dir.0.join("corrupted.zip");
        let regular = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o755);
        let mut zip = ZipWriter::new(File::create(&corrupted).unwrap());
        zip.start_file("open-scanline/marker-fixture", regular)
            .unwrap();
        zip.write_all(format!("#!/bin/sh\nprintf invoked > '{}'\n", marker.display()).as_bytes())
            .unwrap();
        zip.finish().unwrap();

        let expected = BinaryFingerprint {
            size: expected_bytes.len() as u64,
            sha256: Sha256::digest(expected_bytes).into(),
        };
        let error = verify_archive(&corrupted, "marker-fixture", &expected).unwrap_err();

        assert!(error.to_string().contains("size does not match source"));
        assert!(!marker.exists());
    }

    #[test]
    fn archive_verification_requires_the_exact_embedded_third_party_notice() {
        let dir = TestDirectory::new("missing-notice");
        let path = dir.0.join("missing-notice.zip");
        let binary = b"portable fixture";
        let executable = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o755);
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        zip.start_file("open-scanline/fixture", executable).unwrap();
        zip.write_all(binary).unwrap();
        let regular = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o644);
        zip.start_file("open-scanline/LICENSE", regular).unwrap();
        zip.write_all(PROJECT_LICENSE).unwrap();
        zip.start_file(
            format!("open-scanline/{RUST_DEPENDENCY_LICENSES_NAME}"),
            regular,
        )
        .unwrap();
        zip.write_all(RUST_DEPENDENCY_LICENSES).unwrap();
        zip.finish().unwrap();

        let expected = BinaryFingerprint {
            size: binary.len() as u64,
            sha256: Sha256::digest(binary).into(),
        };
        let error = verify_archive(&path, "fixture", &expected).unwrap_err();
        assert!(error
            .to_string()
            .contains("exactly one third-party notice entry"));
    }

    #[test]
    fn archive_verification_requires_the_exact_embedded_project_license() {
        let dir = TestDirectory::new("missing-project-license");
        let path = dir.0.join("missing-project-license.zip");
        let binary = b"portable fixture";
        let executable = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o755);
        let regular = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o644);
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        zip.start_file("open-scanline/fixture", executable).unwrap();
        zip.write_all(binary).unwrap();
        zip.start_file("open-scanline/THIRD_PARTY_NOTICES.md", regular)
            .unwrap();
        zip.write_all(THIRD_PARTY_NOTICES).unwrap();
        zip.start_file(
            format!("open-scanline/{RUST_DEPENDENCY_LICENSES_NAME}"),
            regular,
        )
        .unwrap();
        zip.write_all(RUST_DEPENDENCY_LICENSES).unwrap();
        zip.finish().unwrap();

        let expected = BinaryFingerprint {
            size: binary.len() as u64,
            sha256: Sha256::digest(binary).into(),
        };
        let error = verify_archive(&path, "fixture", &expected).unwrap_err();
        assert!(error
            .to_string()
            .contains("exactly one project license entry"));
    }

    #[test]
    fn archive_verification_requires_the_exact_embedded_dependency_license_bundle() {
        let dir = TestDirectory::new("missing-dependency-license-bundle");
        let path = dir.0.join("missing-dependency-license-bundle.zip");
        let binary = b"portable fixture";
        let executable = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o755);
        let regular = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o644);
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        zip.start_file("open-scanline/fixture", executable).unwrap();
        zip.write_all(binary).unwrap();
        zip.start_file("open-scanline/LICENSE", regular).unwrap();
        zip.write_all(PROJECT_LICENSE).unwrap();
        zip.start_file("open-scanline/THIRD_PARTY_NOTICES.md", regular)
            .unwrap();
        zip.write_all(THIRD_PARTY_NOTICES).unwrap();
        zip.finish().unwrap();

        let expected = BinaryFingerprint {
            size: binary.len() as u64,
            sha256: Sha256::digest(binary).into(),
        };
        let error = verify_archive(&path, "fixture", &expected).unwrap_err();
        assert!(error
            .to_string()
            .contains("exactly one Rust dependency license bundle entry"));
    }

    #[test]
    fn archive_verification_requires_the_exact_embedded_project_readme() {
        let dir = TestDirectory::new("missing-project-readme");
        let path = dir.0.join("missing-project-readme.zip");
        let binary = b"portable fixture";
        let executable = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o755);
        let regular = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o644);
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        zip.start_file("open-scanline/fixture", executable).unwrap();
        zip.write_all(binary).unwrap();
        zip.start_file("open-scanline/LICENSE", regular).unwrap();
        zip.write_all(PROJECT_LICENSE).unwrap();
        zip.start_file("open-scanline/THIRD_PARTY_NOTICES.md", regular)
            .unwrap();
        zip.write_all(THIRD_PARTY_NOTICES).unwrap();
        zip.start_file(
            format!("open-scanline/{RUST_DEPENDENCY_LICENSES_NAME}"),
            regular,
        )
        .unwrap();
        zip.write_all(RUST_DEPENDENCY_LICENSES).unwrap();
        zip.finish().unwrap();

        let expected = BinaryFingerprint {
            size: binary.len() as u64,
            sha256: Sha256::digest(binary).into(),
        };
        let error = verify_archive(&path, "fixture", &expected).unwrap_err();
        assert!(error
            .to_string()
            .contains("exactly one project README entry"));
    }

    #[test]
    fn direct_output_alias_is_rejected_before_launch_or_truncation() {
        let dir = TestDirectory::new("direct-alias");
        let binary = dir.0.join("scanner");
        let contents = b"do not modify this binary";
        fs::write(&binary, contents).unwrap();

        let error = build_portable(&PackagingOptions {
            binary: binary.clone(),
            out: binary.clone(),
        })
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            format!(
                "invalid: package output aliases package binary: {}",
                binary.display()
            )
        );
        assert_eq!(fs::read(&binary).unwrap(), contents);
        assert_no_temporary_archives(&dir.0);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_output_alias_is_rejected_before_launch_or_truncation() {
        use std::os::unix::fs::symlink;

        let dir = TestDirectory::new("symlink-alias");
        let binary = dir.0.join("scanner");
        let out = dir.0.join("portable.zip");
        let contents = b"do not modify this binary";
        fs::write(&binary, contents).unwrap();
        symlink(&binary, &out).unwrap();

        let error = build_portable(&PackagingOptions {
            binary: binary.clone(),
            out: out.clone(),
        })
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            format!(
                "invalid: package output aliases package binary: {}",
                out.display()
            )
        );
        assert_eq!(fs::read(&binary).unwrap(), contents);
        assert!(fs::symlink_metadata(&out).unwrap().file_type().is_symlink());
        assert_no_temporary_archives(&dir.0);
    }

    #[cfg(unix)]
    #[test]
    fn hard_link_output_alias_is_rejected_before_launch_or_truncation() {
        let dir = TestDirectory::new("hard-link-alias");
        let binary = dir.0.join("scanner");
        let out = dir.0.join("portable.zip");
        let contents = b"do not modify this binary";
        fs::write(&binary, contents).unwrap();
        fs::hard_link(&binary, &out).unwrap();

        let error = build_portable(&PackagingOptions {
            binary: binary.clone(),
            out: out.clone(),
        })
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            format!(
                "invalid: package output aliases package binary: {}",
                out.display()
            )
        );
        assert_eq!(fs::read(&binary).unwrap(), contents);
        assert_no_temporary_archives(&dir.0);
    }

    #[test]
    fn hostile_binary_name_is_rejected_before_output_creation() {
        let dir = TestDirectory::new("hostile-name");
        let binary = dir.0.join("scanner;echo-injected");
        let out = dir.0.join("not-created").join("portable.zip");
        fs::write(&binary, "not launched").unwrap();

        let error = build_portable(&PackagingOptions {
            binary,
            out: out.clone(),
        })
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "invalid: package binary file name must use only ASCII letters, digits, '.', '_' or '-'"
        );
        assert!(!out.exists());
        assert!(!out.parent().unwrap().exists());
    }

    #[test]
    fn temporary_publish_replaces_existing_output() {
        let dir = TestDirectory::new("publish-replace");
        let out = dir.0.join("portable.zip");
        fs::write(&out, "previous archive").unwrap();
        let (temporary, mut file) = TemporaryOutput::create(&out).unwrap();
        file.write_all(b"replacement archive").unwrap();
        file.sync_all().unwrap();
        drop(file);

        temporary.publish(&out).unwrap();

        assert_eq!(fs::read(&out).unwrap(), b"replacement archive");
        assert_no_temporary_archives(&dir.0);
    }

    #[test]
    fn absent_documentation_root_still_embeds_required_license_assets() {
        let dir = TestDirectory::new("documentation");
        let docs = dir.0.join("docs");
        fs::create_dir_all(&docs).unwrap();
        fs::write(docs.join("README.md"), "fixture documentation\n").unwrap();
        let documented = dir.0.join("documented.zip");
        let regular = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o644);
        let mut zip = ZipWriter::new(File::create(&documented).unwrap());
        write_documentation_and_manifest(&mut zip, &docs, "fixture", regular).unwrap();
        zip.finish().unwrap();

        let mut archive = ZipArchive::new(File::open(&documented).unwrap()).unwrap();
        let names = (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "open-scanline/README.md",
                "open-scanline/LICENSE",
                "open-scanline/THIRD_PARTY_NOTICES.md",
                "open-scanline/licenses/RUST_DEPENDENCIES_ALL_FEATURES.md",
                "open-scanline/PORTABLE.txt"
            ]
        );
        let mut readme = String::new();
        archive
            .by_name("open-scanline/README.md")
            .unwrap()
            .read_to_string(&mut readme)
            .unwrap();
        assert_eq!(readme.as_bytes(), PROJECT_README);
        assert_documentation_entries(&mut archive, &docs);

        let undocumented = dir.0.join("undocumented.zip");
        let mut zip = ZipWriter::new(File::create(&undocumented).unwrap());
        write_documentation_and_manifest(&mut zip, &dir.0.join("absent-docs"), "fixture", regular)
            .unwrap();
        zip.finish().unwrap();

        let mut archive = ZipArchive::new(File::open(&undocumented).unwrap()).unwrap();
        assert_eq!(archive.len(), 5);
        assert_eq!(
            archive.by_index(0).unwrap().name(),
            "open-scanline/README.md"
        );
        assert_eq!(archive.by_index(1).unwrap().name(), "open-scanline/LICENSE");
        assert_eq!(
            archive.by_index(2).unwrap().name(),
            "open-scanline/THIRD_PARTY_NOTICES.md"
        );
        assert_eq!(
            archive.by_index(3).unwrap().name(),
            "open-scanline/licenses/RUST_DEPENDENCIES_ALL_FEATURES.md"
        );
        assert_eq!(
            archive.by_index(4).unwrap().name(),
            "open-scanline/PORTABLE.txt"
        );
        assert_documentation_entries(&mut archive, &dir.0.join("absent-docs"));
    }

    #[test]
    fn missing_binary_fails_before_archive_creation() {
        let dir = TestDirectory::new("missing");
        let out = dir.0.join("portable.zip");
        let error = build_portable(&PackagingOptions {
            binary: dir.0.join("missing"),
            out: out.clone(),
        })
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "invalid: package binary does not exist: {}",
                dir.0.join("missing").display()
            )
        );
        assert!(!out.exists());
    }
}
