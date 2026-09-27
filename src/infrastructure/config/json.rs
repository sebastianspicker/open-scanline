//! On-disk application config (OSL-CONFIG). No DRM / activation keys.

use crate::error::{Result, ScanError};
use crate::infrastructure::runtime::platform;
use crate::workflows::settings::{validate_ocr_language, AppConfig};
use serde_json::Value;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Never persist or re-load activation / license / serial / DRM material.
/// Matching is case-insensitive on the key name.
const BANNED_CONFIG_KEYS: &[&str] = &[
    "activation",
    "activation_key",
    "license",
    "license_key",
    "serial",
    "serial_number",
    "drm",
    "drm_key",
    "product_key",
];

/// Maximum UTF-8 length for a GUI output file stem. This leaves room for the
/// fixed suffixes/extensions on filesystems whose component limit is 255 bytes.
pub const MAX_OUTPUT_NAME_BYTES: usize = 128;

/// Config JSON holds user preferences and should remain comfortably below this
/// 1 MiB limit. The ceiling prevents an untrusted path from forcing an
/// unbounded allocation before parsing.
pub(crate) const MAX_CONFIG_JSON_BYTES: u64 = 1024 * 1024;

type ParsedCurvePoints = Option<Vec<[i32; 2]>>;

struct CurvePoint {
    x: i32,
    y: i32,
}

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
        &["offline", "ocrs", "tesseract"],
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
    if !(1..=crate::workflows::ports::acquisition::MAX_SCAN_PAGES).contains(&config.batch_pages) {
        return Err(ScanError::Invalid(format!(
            "batch pages must be in 1..={}",
            crate::workflows::ports::acquisition::MAX_SCAN_PAGES
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

/// Validate a saturation control value accepted by the CLI and GUI.
pub fn validate_saturation(value: f64) -> Result<f64> {
    if value.is_finite() && (-100.0..=100.0).contains(&value) {
        Ok(value)
    } else {
        Err(ScanError::Other(
            "saturation must be a finite number from -100 to 100".into(),
        ))
    }
}

/// Validate a hue control value accepted by the CLI and GUI.
pub fn validate_hue(value: f64) -> Result<f64> {
    if value.is_finite() && (-180.0..=180.0).contains(&value) {
        Ok(value)
    } else {
        Err(ScanError::Other(
            "hue must be a finite number of degrees from -180 to 180".into(),
        ))
    }
}

/// Validate a portable filename component used as the stem of GUI output files.
///
/// The policy is intentionally cross-platform: it rejects Windows-invalid
/// names even when the current application runs on another operating system.
pub fn validate_output_name(value: &str) -> Result<&str> {
    if value.is_empty() || value.len() > MAX_OUTPUT_NAME_BYTES {
        return Err(ScanError::Invalid(format!(
            "output name must contain 1..={MAX_OUTPUT_NAME_BYTES} UTF-8 bytes"
        )));
    }
    if value == "." || value == ".." {
        return Err(ScanError::Invalid(
            "output name must be one normal file name component".into(),
        ));
    }
    if value.ends_with(['.', ' ']) {
        return Err(ScanError::Invalid(
            "output name must not end with a dot or space".into(),
        ));
    }
    if value.chars().any(invalid_output_name_character) {
        return Err(ScanError::Invalid(
            "output name contains a path separator, control character, or Windows-reserved character"
                .into(),
        ));
    }
    if is_windows_reserved_name(value) {
        return Err(ScanError::Invalid(
            "output name uses a Windows-reserved device name".into(),
        ));
    }
    Ok(value)
}

fn invalid_output_name_character(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
        )
}

fn is_windows_reserved_name(value: &str) -> bool {
    let device_name = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || is_windows_numbered_device(&device_name)
}

fn is_windows_numbered_device(device_name: &str) -> bool {
    if device_name.len() != 4 {
        return false;
    }
    if !device_name.starts_with("COM") && !device_name.starts_with("LPT") {
        return false;
    }
    let digit = device_name.as_bytes()[3];
    if digit < 49 {
        return false;
    }
    digit <= 57
}

fn normalize_curve_input(input: &str) -> &str {
    input.trim()
}

/// Parse tone-curve control points from `x:y,x:y` text.
///
/// A supplied curve must contain at least two points. Both components are
/// integers in 0..=255, and x values must be strictly increasing.
pub fn parse_curve_points(input: &str) -> Result<ParsedCurvePoints> {
    let input = normalize_curve_input(input);
    if input.is_empty() {
        return Ok(None);
    }

    let mut points = Vec::new();
    let mut previous_x = None;
    for (index, raw_point) in input.split(',').enumerate() {
        let point = parse_curve_point(raw_point, index + 1)?;
        validate_curve_x(previous_x, point.x)?;
        previous_x = Some(point.x);
        points.push([point.x, point.y]);
    }

    if points.len() < 2 {
        return Err(ScanError::Other(
            "curves must contain at least two x:y points".into(),
        ));
    }
    Ok(Some(points))
}

fn parse_curve_point(raw_point: &str, position: usize) -> Result<CurvePoint> {
    let point = raw_point.trim();
    let (raw_x, raw_y) = point
        .split_once(':')
        .filter(|(_, raw_y)| !raw_y.contains(':'))
        .ok_or_else(|| ScanError::Other(format!("curve point {position} must be x:y")))?;
    let x = parse_curve_component(raw_x, position, "x")?;
    let y = parse_curve_component(raw_y, position, "y")?;
    if !(0..=255).contains(&x) || !(0..=255).contains(&y) {
        return Err(ScanError::Other(format!(
            "curve point {position} values must be in 0..=255"
        )));
    }
    Ok(CurvePoint { x, y })
}

fn parse_curve_component(raw: &str, position: usize, axis: &str) -> Result<i32> {
    raw.trim().parse::<i32>().map_err(|_| {
        ScanError::Other(format!(
            "curve point {position} has an invalid {axis} value"
        ))
    })
}

fn validate_curve_x(previous_x: Option<i32>, x: i32) -> Result<()> {
    if previous_x.is_some_and(|previous_x| x <= previous_x) {
        return Err(ScanError::Other(
            "curve x values must be strictly increasing".into(),
        ));
    }
    Ok(())
}

/// Format persisted tone-curve control points for the GUI text field.
pub fn format_curve_points(points: Option<&[[i32; 2]]>) -> String {
    points
        .unwrap_or_default()
        .iter()
        .map(|[x, y]| format!("{x}:{y}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Default user-local config path via platform (`config_dir()/config.json`).
pub fn default_config_path() -> PathBuf {
    platform::config_dir().join("config.json")
}

/// Return true if the key is banned (activation / license / serial / DRM).
pub fn is_banned_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    BANNED_CONFIG_KEYS.contains(&lower.as_str())
}

/// Drop activation / license / serial / DRM keys (case-insensitive name match).
pub fn strip_banned(data: &Value) -> Value {
    match data {
        Value::Object(map) => Value::Object(strip_banned_object(map)),
        other => other.clone(),
    }
}

fn strip_banned_object(map: &serde_json::Map<String, Value>) -> serde_json::Map<String, Value> {
    map.iter()
        .filter(|(key, _)| !is_banned_key(key))
        .map(|(key, value)| (key.clone(), strip_banned(value)))
        .collect()
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
