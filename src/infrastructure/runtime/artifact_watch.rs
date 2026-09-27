//! Direct-to-file artifact quotas for supervised native commands.

use super::seams::ArtifactQuota;
use crate::domain::acquisition::ScanRequest;
use crate::domain::image::{checked_image_len, MAX_IMAGE_BYTES};
use crate::error::{Result, ScanError};
use std::fs;
use std::path::Path;

/// A private artifact directory and the quota enforced while a child runs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ArtifactWatch<'a> {
    pub(crate) directory: &'a Path,
    pub(crate) quota: ArtifactQuota,
}

const ARTIFACT_PER_IMAGE_OVERHEAD: u64 = 1024 * 1024;

/// Derive a checked, request-specific native artifact budget.
///
/// Each requested logical side receives enough room for its decoded pixel
/// buffer plus a small encoded-image overhead. The whole directory remains
/// capped near the product-wide decoded-image ceiling, so a feeder cannot use
/// its page limit to turn a bounded scan into an unbounded disk write.
pub(crate) fn artifact_quota_for_request(
    request: &ScanRequest,
    max_pages: u32,
) -> Result<ArtifactQuota> {
    let (width, height) = request
        .region
        .map(|region| (region.width.max(1), region.height.max(1)))
        .unwrap_or((request.width.max(1), request.height.max(1)));
    let image_bytes = u64::try_from(checked_image_len(
        width,
        height,
        request.pixel_format.bpp(),
    )?)
    .map_err(|_| ScanError::Invalid("image size does not fit the artifact quota".into()))?;
    let per_image = image_bytes
        .checked_add(ARTIFACT_PER_IMAGE_OVERHEAD)
        .ok_or_else(|| ScanError::Invalid("artifact quota overflow".into()))?;
    let requested_bytes = per_image
        .checked_mul(u64::from(max_pages))
        .ok_or_else(|| ScanError::Invalid("artifact quota overflow".into()))?;
    let global_limit = u64::try_from(MAX_IMAGE_BYTES)
        .expect("usize image limit always fits in u64")
        .checked_add(ARTIFACT_PER_IMAGE_OVERHEAD)
        .expect("fixed artifact overhead fits in u64");
    Ok(ArtifactQuota {
        max_files: u64::from(max_pages),
        max_bytes: requested_bytes.min(global_limit),
    })
}

/// Reject an artifact directory that has exceeded its file or byte budget.
/// Native SANE and WIA output is deliberately flat; nested directories are
/// rejected instead of traversed, keeping quota checks bounded even for a
/// malicious or broken driver. Symlinks and other non-regular entries are
/// rejected rather than followed into caller-controlled locations.
pub(crate) fn validate_artifact_quota(directory: &Path, quota: ArtifactQuota) -> Result<()> {
    let mut files = 0_u64;
    let mut bytes = 0_u64;

    let entries = fs::read_dir(directory).map_err(|error| {
        ScanError::Unsupported(format!(
            "could not inspect native artifact directory {}: {error}",
            directory.display()
        ))
    })?;
    for entry in entries {
        add_artifact_entry(entry?, quota, &mut files, &mut bytes)?;
    }
    Ok(())
}

fn add_artifact_entry(
    entry: fs::DirEntry,
    quota: ArtifactQuota,
    files: &mut u64,
    bytes: &mut u64,
) -> Result<()> {
    // DirEntry::metadata performs fresh non-following metadata access on Unix.
    // Windows may return enumeration-time metadata, so use a fresh path lookup.
    // https://doc.rust-lang.org/std/fs/struct.DirEntry.html#method.metadata
    let metadata = fresh_entry_metadata(&entry).map_err(|error| {
        ScanError::Unsupported(format!(
            "could not inspect native artifact {}: {error}",
            entry.path().display()
        ))
    })?;
    validate_artifact_entry(&entry, &metadata)?;
    *files = files.saturating_add(1);
    *bytes = bytes.saturating_add(metadata.len());
    if *files > quota.max_files || *bytes > quota.max_bytes {
        return Err(ScanError::Unsupported(format!(
            "native artifact output exceeded the {} file / {} byte quota",
            quota.max_files, quota.max_bytes
        )));
    }
    Ok(())
}

fn fresh_entry_metadata(entry: &fs::DirEntry) -> std::io::Result<fs::Metadata> {
    #[cfg(unix)]
    {
        entry.metadata()
    }
    #[cfg(not(unix))]
    {
        fs::symlink_metadata(entry.path())
    }
}

fn validate_artifact_entry(entry: &fs::DirEntry, metadata: &fs::Metadata) -> Result<()> {
    if metadata.file_type().is_file() {
        return Ok(());
    }
    let kind = if metadata.file_type().is_dir() {
        "nested directory"
    } else {
        "non-regular entry"
    };
    Err(ScanError::Unsupported(format!(
        "native artifact output contains unexpected {kind} {}",
        entry.path().display()
    )))
}

