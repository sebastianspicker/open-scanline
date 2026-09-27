use super::*;
use std::fs::File;
use std::time::{SystemTime, UNIX_EPOCH};

struct Accept;
impl ModelPairValidator for Accept {
    fn validate(&self, _detection: &Path, _recognition: &Path) -> Result<()> {
        Ok(())
    }
}

struct Reject;
impl ModelPairValidator for Reject {
    fn validate(&self, _detection: &Path, _recognition: &Path) -> Result<()> {
        Err(ScanError::Invalid("test validator rejected pair".into()))
    }
}

struct Scratch(PathBuf);
impl Scratch {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "open-scanline-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn install_pair(root: &Path, scratch: &Scratch, suffix: &str) -> InstalledModelPack {
    let detection = scratch.write(
        &format!("detection-{suffix}"),
        format!("detection-{suffix}").as_bytes(),
    );
    let recognition = scratch.write(
        &format!("recognition-{suffix}"),
        format!("recognition-{suffix}").as_bytes(),
    );
    install_in(root, &detection, &recognition, &Accept).unwrap()
}

#[test]
fn active_lookup_handles_missing_and_rejects_corrupt_pointers_and_manifests() {
    let scratch = Scratch::new("ocr-pack-active");
    assert!(active_in(&scratch.0).unwrap().is_none());
    fs::write(active_path_in(&scratch.0), b"not json").unwrap();
    assert!(active_in(&scratch.0).is_err());
    let pack = install_pair(&scratch.0, &scratch, "one");
    fs::write(pack.root.join("manifest.json"), b"{}").unwrap();
    assert!(active_in(&scratch.0).is_err());
}

#[test]
fn active_lookup_rejects_unbounded_or_non_regular_pointers() {
    let scratch = Scratch::new("ocr-pack-pointer-bounds");
    fs::write(active_path_in(&scratch.0), vec![b'x'; 64 * 1024 + 1]).unwrap();
    assert!(active_in(&scratch.0).is_err());
    fs::remove_file(active_path_in(&scratch.0)).unwrap();
    fs::create_dir(active_path_in(&scratch.0)).unwrap();
    assert!(active_in(&scratch.0).is_err());
}

#[test]
fn oversized_sparse_sources_are_rejected_without_reading_them() {
    let scratch = Scratch::new("ocr-pack-large");
    let source = scratch.0.join("oversized.rten");
    File::create(&source)
        .unwrap()
        .set_len(MAX_MODEL_BYTES + 1)
        .unwrap();
    assert!(checked_source(&source, "model").is_err());
}

#[test]
fn selected_pack_detects_hash_mutation() {
    let scratch = Scratch::new("ocr-pack-hash");
    let pack = install_pair(&scratch.0, &scratch, "one");
    let path = pack.root.join("detection.rten");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, b"mutated---one").unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
    assert!(active_in(&scratch.0).is_err());
}

#[test]
fn rejected_install_keeps_active_pointer_and_cleans_staging() {
    let scratch = Scratch::new("ocr-pack-atomic");
    install_pair(&scratch.0, &scratch, "one");
    let before = fs::read(active_path_in(&scratch.0)).unwrap();
    let detection = scratch.write("rejected-detection", b"different detection");
    let recognition = scratch.write("rejected-recognition", b"different recognition");
    assert!(install_in(&scratch.0, &detection, &recognition, &Reject).is_err());
    assert_eq!(fs::read(active_path_in(&scratch.0)).unwrap(), before);
    assert!(fs::read_dir(packs_dir_in(&scratch.0))
        .unwrap()
        .all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".staging-")));
}

#[test]
fn reinstalling_the_same_pair_is_idempotent() {
    let scratch = Scratch::new("ocr-pack-idempotent");
    let detection = scratch.write("detection", b"detection");
    let recognition = scratch.write("recognition", b"recognition");
    let first = install_in(&scratch.0, &detection, &recognition, &Accept).unwrap();
    let second = install_in(&scratch.0, &detection, &recognition, &Accept).unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(fs::read_dir(packs_dir_in(&scratch.0)).unwrap().count(), 1);
    assert_eq!(status_in(&scratch.0).unwrap()["integrity"], "ok");
}
