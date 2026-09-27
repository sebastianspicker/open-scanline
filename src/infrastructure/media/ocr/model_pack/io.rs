use super::*;
use crate::workflows::operation::CancellationToken;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};

#[cfg(test)]
thread_local! {
    static TEST_READ_COUNTS: std::cell::RefCell<std::collections::HashMap<PathBuf, usize>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

pub(super) struct CheckedSource {
    pub(super) path: PathBuf,
    pub(super) bytes: u64,
    pub(super) sha256: String,
}

pub(super) fn checked_source(path: &Path, label: &str) -> Result<CheckedSource> {
    let metadata = fs::symlink_metadata(path)?;
    validate_source_metadata(path, &metadata, label)?;
    let (bytes, sha256) = hash_file(path, MAX_MODEL_BYTES)?;
    if bytes != metadata.len() {
        return Err(ScanError::Invalid(format!(
            "{label} changed while it was read: {}",
            path.display()
        )));
    }
    Ok(CheckedSource {
        path: path.into(),
        bytes,
        sha256,
    })
}

fn validate_source_metadata(path: &Path, metadata: &fs::Metadata, label: &str) -> Result<()> {
    if !metadata.file_type().is_file() {
        return Err(ScanError::Invalid(format!(
            "{label} must be a regular file: {}",
            path.display()
        )));
    }
    if metadata.len() > MAX_MODEL_BYTES {
        return Err(ScanError::Invalid(format!(
            "{label} exceeds the {MAX_MODEL_BYTES}-byte limit"
        )));
    }
    Ok(())
}

pub(super) fn copy_model(
    source: &CheckedSource,
    destination: &Path,
    filename: &str,
) -> Result<ModelFile> {
    let copied = copy_model_file(source, destination, filename)?;
    validate_copied_model(source, copied)?;
    let model = ModelFile {
        filename: filename.into(),
        bytes: copied,
        sha256: source.sha256.clone(),
    };
    verify_model(destination, &model, filename)?;
    Ok(model)
}

fn copy_model_file(source: &CheckedSource, destination: &Path, filename: &str) -> Result<u64> {
    let target = destination.join(filename);
    let mut input = File::open(&source.path)?;
    let mut output = File::options().write(true).create_new(true).open(&target)?;
    let copied = std::io::copy(
        &mut std::io::Read::by_ref(&mut input).take(MAX_MODEL_BYTES + 1),
        &mut output,
    )?;
    output.flush()?;
    output.sync_all()?;
    Ok(copied)
}

fn validate_copied_model(source: &CheckedSource, copied: u64) -> Result<()> {
    if copied != source.bytes || copied > MAX_MODEL_BYTES {
        return Err(ScanError::Invalid(format!(
            "model changed while copying: {}",
            source.path.display()
        )));
    }
    Ok(())
}

pub(super) fn verify_model(root: &Path, file: &ModelFile, label: &str) -> Result<()> {
    let _ = read_verified_model(root, file, label)?;
    Ok(())
}

#[cfg(all(test, feature = "ocrs"))]
pub(crate) fn verified_model_bytes(pack: &InstalledModelPack) -> Result<(Vec<u8>, Vec<u8>)> {
    Ok((
        read_verified_model(&pack.root, &pack.manifest.detection, "detection")?,
        read_verified_model(&pack.root, &pack.manifest.recognition, "recognition")?,
    ))
}

pub(crate) fn active_verified_model_pair(
    cancellation: Option<&CancellationToken>,
) -> Result<Option<VerifiedModelPair>> {
    active_verified_model_pair_in(&root_dir(), cancellation)
}

pub(crate) fn active_verified_model_pair_in(
    root: &Path,
    cancellation: Option<&CancellationToken>,
) -> Result<Option<VerifiedModelPair>> {
    require_compiled()?;
    check_model_cancellation(cancellation)?;
    let Some((pack_root, manifest)) = selected_manifest_in(root)? else {
        return Ok(None);
    };
    let detection =
        read_selected_model(&pack_root, &manifest.detection, "detection", cancellation)?;
    let recognition = read_selected_model(
        &pack_root,
        &manifest.recognition,
        "recognition",
        cancellation,
    )?;
    Ok(Some(VerifiedModelPair {
        #[cfg(feature = "ocrs")]
        key: format!(
            "{}:{}",
            manifest.detection.sha256, manifest.recognition.sha256
        ),
        detection,
        recognition,
    }))
}

fn selected_manifest_in(root: &Path) -> Result<Option<(PathBuf, ModelPackManifest)>> {
    let path = active_path_in(root);
    if !path.try_exists()? {
        return Ok(None);
    }
    let bytes = read_bounded_regular(&path, 64 * 1024, "OCR active model pointer")?;
    let active: ActivePack = serde_json::from_slice(&bytes)?;
    if active.format_version != MODEL_PACK_FORMAT_VERSION || !valid_pack_id(&active.pack) {
        return Err(ScanError::Invalid(
            "OCR active model pointer is invalid".into(),
        ));
    }
    let pack_root = packs_dir_in(root).join(&active.pack);
    let manifest = read_pack_manifest(&pack_root)?;
    Ok(Some((pack_root, manifest)))
}

#[cfg(feature = "ocrs")]
pub(crate) fn read_model_source_bytes(path: &Path, label: &str) -> Result<Vec<u8>> {
    read_bounded_regular(path, MAX_MODEL_BYTES, label)
}

fn read_verified_model(root: &Path, file: &ModelFile, label: &str) -> Result<Vec<u8>> {
    read_selected_model(root, file, label, None)
}

fn read_selected_model(
    root: &Path,
    file: &ModelFile,
    label: &str,
    cancellation: Option<&CancellationToken>,
) -> Result<Vec<u8>> {
    validate_model_filename(file, label)?;
    let bytes = read_bounded_regular_cancelled(
        &root.join(&file.filename),
        MAX_MODEL_BYTES,
        &format!("OCR {label} model"),
        cancellation,
    )?;
    if bytes.len() as u64 != file.bytes || sha256_bytes(&bytes) != file.sha256 {
        return Err(ScanError::Invalid(format!(
            "OCR {label} model hash mismatch"
        )));
    }
    Ok(bytes)
}

fn validate_model_filename(file: &ModelFile, label: &str) -> Result<()> {
    if file.filename == "detection.rten" || file.filename == "recognition.rten" {
        Ok(())
    } else {
        Err(ScanError::Invalid(format!(
            "OCR {label} model filename is invalid"
        )))
    }
}

pub(super) fn read_bounded_regular(path: &Path, limit: u64, label: &str) -> Result<Vec<u8>> {
    read_bounded_regular_cancelled(path, limit, label, None)
}

fn read_bounded_regular_cancelled(
    path: &Path,
    limit: u64,
    label: &str,
    cancellation: Option<&CancellationToken>,
) -> Result<Vec<u8>> {
    let path_metadata = fs::symlink_metadata(path)?;
    validate_bounded_regular_metadata(&path_metadata, limit, label)?;
    let mut file = File::open(path)?;
    record_test_read(path);
    let opened_metadata = file.metadata()?;
    validate_bounded_regular_metadata(&opened_metadata, limit, label)?;
    read_bounded_file(&mut file, opened_metadata.len(), limit, label, cancellation)
}

#[cfg(test)]
fn record_test_read(path: &Path) {
    TEST_READ_COUNTS.with(|counts| {
        *counts.borrow_mut().entry(path.to_path_buf()).or_default() += 1;
    });
}

#[cfg(not(test))]
fn record_test_read(_: &Path) {}

#[cfg(all(test, feature = "ocrs"))]
pub(crate) fn reset_test_read_counts() {
    TEST_READ_COUNTS.with(|counts| counts.borrow_mut().clear());
}

#[cfg(all(test, feature = "ocrs"))]
pub(crate) fn test_read_count(path: &Path) -> usize {
    TEST_READ_COUNTS.with(|counts| counts.borrow().get(path).copied().unwrap_or(0))
}

fn validate_bounded_regular_metadata(
    metadata: &fs::Metadata,
    limit: u64,
    label: &str,
) -> Result<()> {
    if !metadata.file_type().is_file() || metadata.len() > limit {
        return Err(ScanError::Invalid(format!(
            "{label} is not a bounded regular file"
        )));
    }
    Ok(())
}

fn read_bounded_file(
    file: &mut File,
    expected_len: u64,
    limit: u64,
    label: &str,
    cancellation: Option<&CancellationToken>,
) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(expected_len as usize);
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        check_model_cancellation(cancellation)?;
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        append_bounded(&mut bytes, &buffer[..count], limit, label)?;
    }
    validate_read_length(bytes.len() as u64, expected_len, limit, label)?;
    Ok(bytes)
}

fn append_bounded(bytes: &mut Vec<u8>, chunk: &[u8], limit: u64, label: &str) -> Result<()> {
    if bytes.len().saturating_add(chunk.len()) as u64 > limit {
        return Err(changed_file_error(label));
    }
    bytes.extend_from_slice(chunk);
    Ok(())
}

fn validate_read_length(actual: u64, expected: u64, limit: u64, label: &str) -> Result<()> {
    if actual > limit || actual != expected {
        Err(changed_file_error(label))
    } else {
        Ok(())
    }
}

fn changed_file_error(label: &str) -> ScanError {
    ScanError::Invalid(format!(
        "{label} changed while it was read or exceeded its size limit"
    ))
}

fn check_model_cancellation(cancellation: Option<&CancellationToken>) -> Result<()> {
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        Err(ScanError::Cancelled("OCRS OCR cancelled".into()))
    } else {
        Ok(())
    }
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hash_file(path: &Path, limit: u64) -> Result<(u64, String)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes = bytes.saturating_add(count as u64);
        if bytes > limit {
            return Err(ScanError::Invalid(format!(
                "model exceeds the {limit}-byte limit"
            )));
        }
        hasher.update(&buffer[..count]);
    }
    Ok((bytes, format!("{:x}", hasher.finalize())))
}
