//! Immutable, locally installed OCRS model packs.
//!
//! Model files are never downloaded by Open Scanline.  Installation copies two
//! caller-provided RTen files into a content-addressed pack, validates them,
//! then atomically switches the active pack pointer as the last operation.

use crate::error::{Result, ScanError};
use crate::infrastructure::runtime::atomic_publish::write_file_atomic;
use crate::infrastructure::runtime::platform;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

mod io;
pub(crate) use io::active_verified_model_pair;
#[cfg(feature = "ocrs")]
pub(crate) use io::read_model_source_bytes;
#[cfg(all(test, feature = "ocrs"))]
pub(crate) use io::{
    active_verified_model_pair_in, reset_test_read_counts, test_read_count, verified_model_bytes,
};
use io::{checked_source, copy_model, read_bounded_regular, verify_model, CheckedSource};

pub const MODEL_PACK_FORMAT_VERSION: u32 = 1;
pub const MAX_MODEL_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelFile {
    pub filename: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelPackManifest {
    pub format_version: u32,
    pub engine: String,
    pub language: String,
    pub detection: ModelFile,
    pub recognition: ModelFile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ActivePack {
    format_version: u32,
    pack: String,
}

#[derive(Debug, Clone)]
pub struct InstalledModelPack {
    pub id: String,
    pub root: PathBuf,
    pub manifest: ModelPackManifest,
}

pub(crate) struct VerifiedModelPair {
    #[cfg(feature = "ocrs")]
    pub(crate) key: String,
    pub(crate) detection: Vec<u8>,
    pub(crate) recognition: Vec<u8>,
}

/// Validates a staged model pair before it can become selectable. Tests inject
/// a deterministic validator, while production constructs a real OCRS engine.
pub trait ModelPairValidator: Send + Sync {
    fn validate(&self, detection: &Path, recognition: &Path) -> Result<()>;
}

struct OcrsModelValidator;

impl ModelPairValidator for OcrsModelValidator {
    fn validate(&self, detection: &Path, recognition: &Path) -> Result<()> {
        crate::infrastructure::media::ocr::ocrs_runner::validate_model_files(detection, recognition)
    }
}

pub fn root_dir() -> PathBuf {
    platform::data_dir().join("ocr-models")
}

pub fn install(detection: &Path, recognition: &Path) -> Result<InstalledModelPack> {
    require_compiled()?;
    install_in(&root_dir(), detection, recognition, &OcrsModelValidator)
}

/// Root-injectable install seam used by isolated tests. Production must use
/// [`install`], which resolves the platform data directory.
pub fn install_in(
    root: &Path,
    detection: &Path,
    recognition: &Path,
    validator: &dyn ModelPairValidator,
) -> Result<InstalledModelPack> {
    require_compiled()?;
    let detection = checked_source(detection, "detection model")?;
    let recognition = checked_source(recognition, "recognition model")?;
    let id = format!(
        "v{}-{}-{}",
        MODEL_PACK_FORMAT_VERSION, detection.sha256, recognition.sha256
    );
    let destination = packs_dir_in(root).join(&id);
    if destination.is_dir() {
        return activate_existing_pack(root, &destination, &id, validator);
    }
    install_new_pack(root, &destination, &id, &detection, &recognition, validator)
}

fn install_new_pack(
    root: &Path,
    destination: &Path,
    id: &str,
    detection: &CheckedSource,
    recognition: &CheckedSource,
    validator: &dyn ModelPairValidator,
) -> Result<InstalledModelPack> {
    fs::create_dir_all(packs_dir_in(root))?;
    let staging = staging_path(root, id);
    fs::create_dir(&staging)?;
    let mut pending = PendingStaging::new(staging.clone());
    let staged = stage_model_pack(&staging, id, detection, recognition)?;
    validate_installed_pack(&staged, validator)?;
    publish_staged_pack(&staging, destination, id, validator, &mut pending)?;
    publish_active_in(root, id)?;
    read_pack(destination, id)
}

fn activate_existing_pack(
    root: &Path,
    destination: &Path,
    id: &str,
    validator: &dyn ModelPairValidator,
) -> Result<InstalledModelPack> {
    let pack = read_pack(destination, id)?;
    validate_installed_pack(&pack, validator)?;
    publish_active_in(root, id)?;
    Ok(pack)
}

fn staging_path(root: &Path, id: &str) -> PathBuf {
    packs_dir_in(root).join(format!(
        ".{id}.staging-{}-{}",
        std::process::id(),
        STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn stage_model_pack(
    staging: &Path,
    id: &str,
    detection: &CheckedSource,
    recognition: &CheckedSource,
) -> Result<InstalledModelPack> {
    let manifest = ModelPackManifest {
        format_version: MODEL_PACK_FORMAT_VERSION,
        engine: "ocrs".into(),
        language: "eng".into(),
        detection: copy_model(detection, staging, "detection.rten")?,
        recognition: copy_model(recognition, staging, "recognition.rten")?,
    };
    write_file_atomic(
        &staging.join("manifest.json"),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    let staged = InstalledModelPack {
        id: id.into(),
        root: staging.to_path_buf(),
        manifest,
    };
    Ok(staged)
}

fn validate_installed_pack(
    pack: &InstalledModelPack,
    validator: &dyn ModelPairValidator,
) -> Result<()> {
    validator.validate(
        &pack.root.join(&pack.manifest.detection.filename),
        &pack.root.join(&pack.manifest.recognition.filename),
    )
}

fn publish_staged_pack(
    staging: &Path,
    destination: &Path,
    id: &str,
    validator: &dyn ModelPairValidator,
    pending: &mut PendingStaging,
) -> Result<()> {
    match fs::rename(staging, destination) {
        Ok(()) => pending.published = true,
        Err(error) => {
            if !destination.is_dir() {
                return Err(error.into());
            }
            let existing = read_pack(destination, id)?;
            validate_installed_pack(&existing, validator)?;
        }
    }
    Ok(())
}

pub fn active() -> Result<Option<InstalledModelPack>> {
    active_in(&root_dir())
}

/// Root-injectable active-pack lookup. It verifies the manifest and both
/// model hashes every time selection is read.
pub fn active_in(root: &Path) -> Result<Option<InstalledModelPack>> {
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
    read_pack(&packs_dir_in(root).join(&active.pack), &active.pack).map(Some)
}

pub fn status() -> Result<serde_json::Value> {
    status_in(&root_dir())
}

pub fn status_in(root: &Path) -> Result<serde_json::Value> {
    match active_in(root)? {
        Some(pack) => Ok(serde_json::json!({
            "compiled": cfg!(feature = "ocrs"),
            "installed": true,
            "active": pack.id,
            "engine": pack.manifest.engine,
            "language": pack.manifest.language,
            "detection": pack.manifest.detection,
            "recognition": pack.manifest.recognition,
            "integrity": "ok",
            "ok": true,
        })),
        None => Ok(serde_json::json!({
            "compiled": cfg!(feature = "ocrs"),
            "installed": false,
            "ok": true
        })),
    }
}

fn require_compiled() -> Result<()> {
    if cfg!(feature = "ocrs") {
        Ok(())
    } else {
        Err(ScanError::Unsupported(
            "OCRS support was not compiled into this build".into(),
        ))
    }
}

fn publish_active_in(root: &Path, id: &str) -> Result<()> {
    write_file_atomic(
        &active_path_in(root),
        &serde_json::to_vec_pretty(&ActivePack {
            format_version: MODEL_PACK_FORMAT_VERSION,
            pack: id.into(),
        })?,
    )
}

fn packs_dir_in(root: &Path) -> PathBuf {
    root.join("packs")
}

fn active_path_in(root: &Path) -> PathBuf {
    root.join("active.json")
}

struct PendingStaging {
    path: PathBuf,
    published: bool,
}

impl PendingStaging {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            published: false,
        }
    }
}

impl Drop for PendingStaging {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn read_pack(root: &Path, id: &str) -> Result<InstalledModelPack> {
    let manifest = read_pack_manifest(root)?;
    verify_model(root, &manifest.detection, "detection")?;
    verify_model(root, &manifest.recognition, "recognition")?;
    Ok(InstalledModelPack {
        id: id.into(),
        root: root.into(),
        manifest,
    })
}

fn read_pack_manifest(root: &Path) -> Result<ModelPackManifest> {
    let manifest_path = root.join("manifest.json");
    let manifest: ModelPackManifest = serde_json::from_slice(&read_bounded_regular(
        &manifest_path,
        64 * 1024,
        "OCR model manifest",
    )?)?;
    if manifest.format_version != MODEL_PACK_FORMAT_VERSION
        || manifest.engine != "ocrs"
        || manifest.language != "eng"
    {
        return Err(ScanError::Invalid(
            "OCR model manifest has unsupported format or engine".into(),
        ));
    }
    Ok(manifest)
}

fn valid_pack_id(value: &str) -> bool {
    value.starts_with("v1-")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[cfg(all(test, feature = "ocrs"))]

#[cfg(all(test, not(feature = "ocrs")))]
mod disabled_tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn disabled_status_distinguishes_compilation_from_installation() {
        let status = status_in(Path::new("definitely-not-read")).unwrap();
        assert_eq!(status["compiled"], false);
        assert_eq!(status["installed"], false);
    }

    #[test]
    fn disabled_status_can_report_installed_verified_resources() {
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!(
            "open-scanline-disabled-status-{}-{}",
            std::process::id(),
            u128::from_le_bytes(nonce)
        ));
        let id = "v1-disabled-status-fixture";
        let pack = root.join("packs").join(id);
        fs::create_dir_all(&pack).unwrap();
        let detection = b"detection";
        let recognition = b"recognition";
        fs::write(pack.join("detection.rten"), detection).unwrap();
        fs::write(pack.join("recognition.rten"), recognition).unwrap();
        let manifest = ModelPackManifest {
            format_version: MODEL_PACK_FORMAT_VERSION,
            engine: "ocrs".into(),
            language: "eng".into(),
            detection: fixture_model("detection.rten", detection),
            recognition: fixture_model("recognition.rten", recognition),
        };
        fs::write(
            pack.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        publish_active_in(&root, id).unwrap();
        let status = status_in(&root).unwrap();
        assert_eq!(status["compiled"], false);
        assert_eq!(status["installed"], true);
        let _ = fs::remove_dir_all(root);
    }

    fn fixture_model(filename: &str, bytes: &[u8]) -> ModelFile {
        ModelFile {
            filename: filename.into(),
            bytes: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        }
    }

    #[test]
    fn disabled_install_rejects_before_sources_or_validator() {
        struct PanicValidator;
        impl ModelPairValidator for PanicValidator {
            fn validate(&self, _: &Path, _: &Path) -> Result<()> {
                panic!("validator must not be called")
            }
        }
        let error = install_in(
            Path::new("not-created"),
            Path::new("not-read"),
            Path::new("not-read-either"),
            &PanicValidator,
        )
        .unwrap_err();
        assert!(
            matches!(error, ScanError::Unsupported(message) if message.contains("not compiled"))
        );
    }
}
