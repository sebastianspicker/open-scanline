use super::{
    package_error, PORTABLE_ROOT, PROJECT_LICENSE, PROJECT_README, RUST_DEPENDENCY_LICENSES,
    RUST_DEPENDENCY_LICENSES_NAME, THIRD_PARTY_NOTICES,
};
use crate::error::{Result, ScanError};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BinaryFingerprint {
    pub(super) size: u64,
    pub(super) sha256: [u8; 32],
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
                ));
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
    let (mut source, before, mut snapshot) =
        prepare_binary_snapshot(zip, binary, binary_name, executable)?;
    let fingerprint = copy_source_to_snapshot(&mut source, &mut snapshot, on_snapshot_chunk)?;
    verify_stable_binary(binary, &source, before, &fingerprint)?;
    snapshot.file_mut().rewind()?;
    std::io::copy(snapshot.file_mut(), zip)?;
    Ok(fingerprint)
}

fn prepare_binary_snapshot(
    zip: &mut ZipWriter<File>,
    binary: &Path,
    binary_name: &str,
    executable: SimpleFileOptions,
) -> Result<(File, SourceStamp, PrivateSnapshot)> {
    let (source, stamp) = open_packaged_binary(binary)?;
    begin_binary_entry(zip, binary_name, executable)?;
    Ok((source, stamp, private_snapshot()?))
}

fn begin_binary_entry(
    zip: &mut ZipWriter<File>,
    binary_name: &str,
    executable: SimpleFileOptions,
) -> Result<()> {
    zip.start_file(format!("{PORTABLE_ROOT}/{binary_name}"), executable)
        .map_err(|error| package_error("could not add portable binary", error))
}

fn open_packaged_binary(binary: &Path) -> Result<(File, SourceStamp)> {
    let source = File::open(binary)?;
    let metadata = source.metadata()?;
    if !source_is_executable(&metadata) {
        return Err(ScanError::Invalid(format!(
            "package binary is not executable: {}",
            binary.display()
        )));
    }
    Ok((source, source_stamp(&metadata)))
}

fn copy_source_to_snapshot<F>(
    source: &mut File,
    snapshot: &mut PrivateSnapshot,
    on_snapshot_chunk: &mut F,
) -> Result<BinaryFingerprint>
where
    F: FnMut(u64),
{
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    while let Some(read) = read_snapshot_chunk(source, &mut buffer)? {
        copied = write_snapshot_chunk(snapshot, &buffer[..read], copied)?;
        on_snapshot_chunk(copied);
    }
    snapshot.file_mut().flush()?;
    snapshot.file_mut().rewind()?;
    hash_reader(snapshot.file_mut())
}

fn read_snapshot_chunk(source: &mut File, buffer: &mut [u8]) -> Result<Option<usize>> {
    let read = source.read(buffer)?;
    Ok((read != 0).then_some(read))
}
fn write_snapshot_chunk(snapshot: &mut PrivateSnapshot, bytes: &[u8], copied: u64) -> Result<u64> {
    snapshot.file_mut().write_all(bytes)?;
    copied
        .checked_add(bytes.len() as u64)
        .ok_or_else(|| ScanError::Other("package binary is too large".into()))
}

fn verify_stable_binary(
    binary: &Path,
    source: &File,
    before: SourceStamp,
    fingerprint: &BinaryFingerprint,
) -> Result<()> {
    let after = source_stamp(&source.metadata()?);
    let mut verification = File::open(binary)?;
    let verify_before = source_stamp(&verification.metadata()?);
    let verified = hash_reader(&mut verification)?;
    let verify_after = source_stamp(&verification.metadata()?);
    if binary_snapshot_changed(
        &before,
        &after,
        &verify_before,
        &verify_after,
        fingerprint,
        &verified,
    ) {
        Err(ScanError::Other(format!(
            "package binary changed during packaging: {}",
            binary.display()
        )))
    } else {
        Ok(())
    }
}

fn binary_snapshot_changed(
    before: &SourceStamp,
    after: &SourceStamp,
    verify_before: &SourceStamp,
    verify_after: &SourceStamp,
    fingerprint: &BinaryFingerprint,
    verified: &BinaryFingerprint,
) -> bool {
    before != after
        || after != verify_before
        || verify_before != verify_after
        || fingerprint != verified
}

pub(super) fn write_binary_and_launchers<F>(
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

pub(super) fn write_documentation_and_manifest(
    zip: &mut ZipWriter<File>,
    _root: &Path,
    binary_name: &str,
    regular: SimpleFileOptions,
) -> Result<()> {
    write_archive_static_files(zip, regular)?;
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

fn write_archive_static_files(zip: &mut ZipWriter<File>, regular: SimpleFileOptions) -> Result<()> {
    for (name, contents, context) in [
        (
            "README.md",
            PROJECT_README,
            "could not add package documentation",
        ),
        ("LICENSE", PROJECT_LICENSE, "could not add project license"),
        (
            "THIRD_PARTY_NOTICES.md",
            THIRD_PARTY_NOTICES,
            "could not add third-party notices",
        ),
        (
            RUST_DEPENDENCY_LICENSES_NAME,
            RUST_DEPENDENCY_LICENSES,
            "could not add Rust dependency licenses",
        ),
    ] {
        zip.start_file(format!("{PORTABLE_ROOT}/{name}"), regular)
            .map_err(|error| package_error(context, error))?;
        zip.write_all(contents)?;
    }
    Ok(())
}
