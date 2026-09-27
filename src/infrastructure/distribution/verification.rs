use super::archive::BinaryFingerprint;
use super::{
    package_error, PORTABLE_ROOT, PROJECT_LICENSE, PROJECT_README, RUST_DEPENDENCY_LICENSES,
    RUST_DEPENDENCY_LICENSES_NAME, THIRD_PARTY_NOTICES,
};
use crate::error::{Result, ScanError};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use zip::ZipArchive;

pub(super) fn verify_archive(
    path: &Path,
    binary_name: &str,
    expected: &BinaryFingerprint,
) -> Result<()> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| package_error("portable archive cannot be opened", error))?;
    verify_binary_entry(&mut archive, binary_name, expected)?;

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

fn verify_binary_entry(
    archive: &mut ZipArchive<File>,
    binary_name: &str,
    expected: &BinaryFingerprint,
) -> Result<()> {
    let entry_name = format!("{PORTABLE_ROOT}/{binary_name}");
    if matching_entries(archive, &entry_name) != 1 {
        return Err(ScanError::Other(format!(
            "portable archive must contain exactly one binary entry named {entry_name}"
        )));
    }
    let mut entry = archive
        .by_name(&entry_name)
        .map_err(|error| package_error("portable archive is missing its binary", error))?;
    validate_binary_metadata(&entry, &entry_name, expected)?;
    verify_binary_contents(&mut entry, expected)
}

fn validate_binary_metadata(
    entry: &zip::read::ZipFile<'_, File>,
    entry_name: &str,
    expected: &BinaryFingerprint,
) -> Result<()> {
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
    Ok(())
}

fn verify_binary_contents(entry: &mut impl Read, expected: &BinaryFingerprint) -> Result<()> {
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
    Ok(())
}

fn matching_entries(archive: &mut ZipArchive<File>, entry_name: &str) -> usize {
    (0..archive.len())
        .filter_map(|index| {
            archive
                .by_index(index)
                .ok()
                .map(|entry| entry.name() == entry_name)
        })
        .filter(|matches| *matches)
        .count()
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
