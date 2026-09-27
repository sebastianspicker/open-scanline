//! On-disk application config (OSL-CONFIG). No DRM / activation keys.

use crate::domain::settings::{
    format_curve_points, is_banned_key, parse_curve_points, strip_banned, validate_hue,
    validate_ocr_language, validate_output_name, validate_saturation,
};
use crate::error::{Result, ScanError};
use crate::infrastructure::config::AppConfig;
use crate::infrastructure::runtime::platform;
use serde_json::Value;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Config JSON holds user preferences and should remain comfortably below this
/// 1 MiB limit. The ceiling prevents an untrusted path from forcing an
/// unbounded allocation before parsing.
pub(crate) const MAX_CONFIG_JSON_BYTES: u64 = 1024 * 1024;

/// Read a JSON input through an already-open regular-file handle with a hard
/// byte ceiling. The second metadata check rejects files that change size while
/// being read, so callers never parse a partial or newly oversized file.
pub(crate) fn read_bounded_json_file(path: &Path, max_bytes: u64, label: &str) -> Result<String> {
    let mut file = open_json_file(path)?;
    let initial = file.metadata()?;
    ensure_bounded_regular_json_file(path, &initial, max_bytes, label)?;
    let bytes = read_json_bytes(&mut file, path, max_bytes, label)?;
    let final_metadata = file.metadata()?;
    ensure_bounded_regular_json_file(path, &final_metadata, max_bytes, label)?;
    ensure_json_file_stable(path, label, &initial, &final_metadata, bytes.len())?;
    String::from_utf8(bytes).map_err(|error| {
        ScanError::Invalid(format!(
            "{label} is not valid UTF-8: {} ({error})",
            path.display()
        ))
    })
}

fn open_json_file(path: &Path) -> Result<File> {
    let mut options = File::options();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    Ok(options.open(path)?)
}

fn read_json_bytes(file: &mut File, path: &Path, max_bytes: u64, label: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.by_ref()
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(json_file_limit_error(path, max_bytes, label));
    }
    Ok(bytes)
}

fn ensure_json_file_stable(
    path: &Path,
    label: &str,
    initial: &std::fs::Metadata,
    final_metadata: &std::fs::Metadata,
    bytes_read: usize,
) -> Result<()> {
    if final_metadata.len() != initial.len() || final_metadata.len() != bytes_read as u64 {
        return Err(ScanError::Invalid(format!(
            "{label} changed while it was being read: {}",
            path.display()
        )));
    }
    Ok(())
}

fn ensure_bounded_regular_json_file(
    path: &Path,
    metadata: &std::fs::Metadata,
    max_bytes: u64,
    label: &str,
) -> Result<()> {
    if !metadata.file_type().is_file() {
        return Err(ScanError::Invalid(format!(
            "{label} must be a regular file: {}",
            path.display()
        )));
    }
    if metadata.len() > max_bytes {
        return Err(json_file_limit_error(path, max_bytes, label));
    }
    Ok(())
}

fn json_file_limit_error(path: &Path, max_bytes: u64, label: &str) -> ScanError {
    ScanError::Invalid(format!(
        "{label} exceeds the {max_bytes}-byte limit: {}",
        path.display()
    ))
}

fn validate_app_config(config: &AppConfig) -> Result<()> {
    validate_output_name(&config.output_name)?;
    validate_config_formats(config)?;
    validate_config_image(config)?;
    validate_config_processing(config)
}

fn validate_config_formats(config: &AppConfig) -> Result<()> {
    validate_config_choice(
        "output format",
        &config.output_format,
        crate::infrastructure::media::supported_extensions(),
    )?;
    validate_config_choice(
        "multipage format",
        &config.multipage_format,
        &["pdf", "tif", "tiff"],
    )?;
    validate_config_choice(
        "OCR engine",
        &config.ocr_engine,
        &crate::domain::export::OcrEngine::NAMES,
    )?;
    validate_ocr_language(&config.ocr_language)?;
    if config.ocr_engine.eq_ignore_ascii_case("ocrs") && config.ocr_language != "eng" {
        return Err(ScanError::Invalid(
            "OCRS model packs support only printed Latin `eng`".into(),
        ));
    }
    Ok(())
}

fn validate_config_image(config: &AppConfig) -> Result<()> {
    crate::domain::acquisition::validate_scan_dpi(config.default_dpi, config.default_dpi)?;
    crate::domain::image::checked_image_len(
        config.default_width,
        config.default_height,
        crate::domain::image::PixelFormat::Rgb8.bpp(),
    )?;
    if !(1..=crate::domain::acquisition::MAX_SCAN_PAGES).contains(&config.batch_pages) {
        return Err(ScanError::Invalid(format!(
            "batch pages must be in 1..={}",
            crate::domain::acquisition::MAX_SCAN_PAGES
        )));
    }
    Ok(())
}

fn validate_config_processing(config: &AppConfig) -> Result<()> {
    validate_saturation(config.saturation)?;
    validate_hue(config.hue)?;
    if let Some(points) = config.curves.as_deref() {
        parse_curve_points(&format_curve_points(Some(points)))?;
    }
    validate_crop(config.crop)
}

fn validate_crop(crop: Option<[i32; 4]>) -> Result<()> {
    let Some([x, y, width, height]) = crop else {
        return Ok(());
    };
    if x < 0 || y < 0 || width < 1 || height < 1 {
        return Err(ScanError::Invalid(
            "config crop must be x,y,w,h with nonnegative origin and positive size".into(),
        ));
    }
    Ok(())
}

fn validate_config_choice(label: &str, value: &str, allowed: &[&str]) -> Result<()> {
    if allowed
        .iter()
        .any(|allowed| value.eq_ignore_ascii_case(allowed))
    {
        Ok(())
    } else {
        Err(ScanError::Invalid(format!(
            "config {label} must be one of: {}",
            allowed.join(", ")
        )))
    }
}

/// Default user-local config path via platform (`config_dir()/config.json`).
pub fn default_config_path() -> PathBuf {
    platform::config_dir().join("config.json")
}

/// Build `AppConfig` from a JSON value, stripping banned keys first.
pub fn config_from_value(raw: Value) -> Result<AppConfig> {
    let cleaned = strip_banned(&raw);
    let cfg: AppConfig = serde_json::from_value(cleaned)
        .map_err(|e| ScanError::Other(format!("config parse: {e}")))?;
    validate_app_config(&cfg)?;
    Ok(cfg)
}

/// Load config from `path`, or default path when `None`. Missing file → defaults.
pub fn load_config(path: Option<&Path>) -> Result<AppConfig> {
    let cfg_path = path
        .map(Path::to_path_buf)
        .unwrap_or_else(default_config_path);
    let text = match read_bounded_json_file(&cfg_path, MAX_CONFIG_JSON_BYTES, "config JSON") {
        Ok(text) => text,
        Err(ScanError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AppConfig::default());
        }
        Err(error) => return Err(error),
    };
    let raw: Value = serde_json::from_str(&text)?;
    config_from_value(raw)
}

/// Save config; banned keys are never written.
///
/// Signature: `save_config(&cfg, path)` → written path.
pub fn save_config(cfg: &AppConfig, path: impl AsRef<Path>) -> Result<PathBuf> {
    validate_app_config(cfg)?;
    let cfg_path = path.as_ref().to_path_buf();
    create_config_parent(&cfg_path)?;
    let value = serializable_config(cfg)?;
    let text = serde_json::to_string_pretty(&value)? + "\n";
    crate::infrastructure::runtime::atomic_publish::write_file_atomic(&cfg_path, text.as_bytes())?;
    Ok(cfg_path)
}

fn create_config_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn serializable_config(cfg: &AppConfig) -> Result<Value> {
    let value = serde_json::to_value(cfg)
        .map_err(|e| ScanError::Other(format!("config serialize: {e}")))?;
    let mut value = strip_banned(&value);
    if let Value::Object(ref mut map) = value {
        map.retain(|k, _| !is_banned_key(k));
    }
    Ok(value)
}
