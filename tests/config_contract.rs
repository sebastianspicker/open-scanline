use open_scanline::config::{
    config_from_value, load_config, save_config, strip_banned, validate_output_name, AppConfig,
};
use open_scanline::core::ScanError;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn default_config_round_trips_through_its_public_file_api() {
    let directory = ScratchDirectory::new("config-round-trip");
    let path = directory.path().join("config.json");
    let default = AppConfig::default();

    assert_eq!(default.last_device_id, "mock");
    assert_eq!(default.output_name, "scan");
    assert_eq!(default.output_format, "png");
    assert_eq!(default.ocr_language, "eng");
    assert_eq!(save_config(&default, &path).unwrap(), path);
    assert_eq!(load_config(Some(&path)).unwrap(), default);
}

#[test]
fn sensitive_config_keys_are_removed_recursively_and_case_insensitively() {
    let input = json!({
        "default_dpi": 300,
        "LICENSE_KEY": "do-not-persist",
        "nested": {
            "serial_number": "also-banned",
            "ordinary": true,
        }
    });

    let stripped = strip_banned(&input);
    assert_eq!(
        stripped,
        json!({
            "default_dpi": 300,
            "nested": {"ordinary": true}
        })
    );
    let config = config_from_value(input).expect("banned metadata should not block config load");
    assert_eq!(config.default_dpi, 300);
}

#[test]
fn config_validation_rejects_unsafe_output_names_and_ocr_languages() {
    for unsafe_name in ["", "../scan", "scan\\name", "CON", "scan.", "scan\nname"] {
        assert!(
            matches!(
                validate_output_name(unsafe_name),
                Err(ScanError::Invalid(_))
            ),
            "unsafe output name was accepted: {unsafe_name:?}"
        );
    }
    assert_eq!(
        validate_output_name("scan-01.final").unwrap(),
        "scan-01.final"
    );

    for language in ["", " \t ", "eng\nfra", &"x".repeat(65)] {
        let mut raw = serde_json::to_value(AppConfig::default()).unwrap();
        raw.as_object_mut()
            .unwrap()
            .insert("ocr_language".into(), Value::String(language.into()));
        assert!(
            matches!(config_from_value(raw), Err(ScanError::Invalid(_))),
            "unsafe OCR language was accepted: {language:?}"
        );
    }
}

struct ScratchDirectory(PathBuf);

impl ScratchDirectory {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "open-scanline-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("scratch directory should be unique");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
