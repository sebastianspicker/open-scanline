//! On-disk application config (OSL-CONFIG). No DRM / activation keys.

use crate::core::{PipelinePrefs, Rect, Result, Rotate, ScanError, ScanMode};
use crate::platform;
use serde::{Deserialize, Serialize};
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

/// Read a JSON input through an already-open regular-file handle with a hard
/// byte ceiling. The second metadata check rejects files that change size while
/// being read, so callers never parse a partial or newly oversized file.
pub(crate) fn read_bounded_json_file(path: &Path, max_bytes: u64, label: &str) -> Result<String> {
    let mut options = File::options();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Opening a FIFO normally blocks before metadata can reject it.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let mut file = options.open(path)?;
    let initial = file.metadata()?;
    ensure_bounded_regular_json_file(path, &initial, max_bytes, label)?;

    let mut bytes = Vec::new();
    file.by_ref()
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(json_file_limit_error(path, max_bytes, label));
    }

    let final_metadata = file.metadata()?;
    ensure_bounded_regular_json_file(path, &final_metadata, max_bytes, label)?;
    if final_metadata.len() != initial.len() || final_metadata.len() != bytes.len() as u64 {
        return Err(ScanError::Invalid(format!(
            "{label} changed while it was being read: {}",
            path.display()
        )));
    }
    String::from_utf8(bytes).map_err(|error| {
        ScanError::Invalid(format!(
            "{label} is not valid UTF-8: {} ({error})",
            path.display()
        ))
    })
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

/// User prefs for scan/pipeline. JSON round-trips known fields only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub last_device_id: String,
    pub default_dpi: u32,
    pub default_width: u32,
    pub default_height: u32,
    pub scan_mode: ScanMode,
    pub duplex: bool,
    pub output_dir: String,
    pub rotate: i32,
    pub flip_h: bool,
    pub flip_v: bool,
    pub brightness: i32,
    pub contrast: i32,
    pub desaturate: bool,
    pub levels_black: i32,
    pub levels_white: i32,
    pub levels_gamma: f64,
    /// Saturation delta in [-100, 100].
    pub saturation: f64,
    /// Hue shift in degrees in [-180, 180].
    pub hue: f64,
    /// Tone curve control points `[x, y]`, with both values in 0..=255.
    pub curves: Option<Vec<[i32; 2]>>,
    /// Optional crop as `[x, y, w, h]`.
    pub crop: Option<[i32; 4]>,
    // Extended fields used by scan/process/CLI (safe defaults).
    pub invert_colors: bool,
    pub auto_deskew: bool,
    pub deskew_angle: f64,
    pub auto_orient: bool,
    pub auto_crop: bool,
    pub white_balance: bool,
    pub auto_levels: bool,
    pub sharpen_amount: f64,
    pub infrared_clean: Option<String>,
    pub descreen: bool,
    pub descreen_dpi: i32,
    pub restore_colors: bool,
    pub restore_fading: bool,
    pub grain_reduction: Option<String>,
    pub flatten: bool,
    pub hole_punch: bool,
    pub colorize_mode: Option<String>,
    pub film_type: Option<String>,
    // GUI workflow preferences. Runtime-only state such as zoom and the
    // current preview path is intentionally not persisted.
    pub batch_pages: u32,
    pub output_name: String,
    pub output_format: String,
    pub multipage: bool,
    pub multipage_format: String,
    pub save_raw: bool,
    pub contact_sheet: bool,
    pub ocr_engine: String,
    pub ocr_language: String,
    pub language: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            last_device_id: "mock".into(),
            default_dpi: 150,
            default_width: 320,
            default_height: 240,
            scan_mode: ScanMode::Reflective,
            duplex: false,
            output_dir: String::new(),
            rotate: 0,
            flip_h: false,
            flip_v: false,
            brightness: 0,
            contrast: 0,
            desaturate: false,
            levels_black: 0,
            levels_white: 255,
            levels_gamma: 1.0,
            saturation: 0.0,
            hue: 0.0,
            curves: None,
            crop: None,
            invert_colors: false,
            auto_deskew: false,
            deskew_angle: 0.0,
            auto_orient: false,
            auto_crop: false,
            white_balance: false,
            auto_levels: false,
            sharpen_amount: 0.0,
            infrared_clean: None,
            descreen: false,
            descreen_dpi: 75,
            restore_colors: false,
            restore_fading: false,
            grain_reduction: None,
            flatten: false,
            hole_punch: false,
            colorize_mode: None,
            film_type: None,
            batch_pages: 1,
            output_name: "scan".into(),
            output_format: "png".into(),
            multipage: false,
            multipage_format: "pdf".into(),
            save_raw: false,
            contact_sheet: false,
            ocr_engine: "offline".into(),
            ocr_language: "eng".into(),
            language: "en".into(),
        }
    }
}

impl AppConfig {
    /// Map config pipeline fields onto [`PipelinePrefs`].
    pub fn to_pipeline_prefs(&self) -> PipelinePrefs {
        let crop = self
            .crop
            .map(|c| Rect::new(c[0], c[1], c[2].max(0) as u32, c[3].max(0) as u32));
        PipelinePrefs {
            rotate: Rotate::from_degrees(self.rotate),
            flip_h: self.flip_h,
            flip_v: self.flip_v,
            crop,
            brightness: self.brightness,
            contrast: self.contrast,
            desaturate: self.desaturate,
            levels_black: self.levels_black,
            levels_white: self.levels_white,
            levels_gamma: self.levels_gamma,
            saturation: self.saturation,
            hue: self.hue,
            curves: self.curves.clone(),
            auto_deskew: self.auto_deskew,
            deskew_angle: self.deskew_angle,
            auto_orient: self.auto_orient,
            auto_crop: self.auto_crop,
            white_balance: self.white_balance,
            sharpen_amount: self.sharpen_amount,
            auto_levels: self.auto_levels,
            infrared_clean: self.infrared_clean.clone(),
            descreen: self.descreen,
            descreen_dpi: self.descreen_dpi,
            restore_colors: self.restore_colors,
            restore_fading: self.restore_fading,
            grain_reduction: self.grain_reduction.clone(),
            flatten: self.flatten,
            hole_punch: self.hole_punch,
            colorize_mode: self.colorize_mode.clone(),
            film_type: self.film_type.clone(),
            ..PipelinePrefs::default()
        }
    }
}

fn validate_app_config(config: &AppConfig) -> Result<()> {
    validate_output_name(&config.output_name)?;
    validate_config_choice(
        "output format",
        &config.output_format,
        crate::imaging::supported_extensions(),
    )?;
    validate_config_choice(
        "multipage format",
        &config.multipage_format,
        &["pdf", "tif", "tiff"],
    )?;
    validate_config_choice("OCR engine", &config.ocr_engine, &["offline", "tesseract"])?;
    validate_ocr_language(&config.ocr_language)?;
    crate::core::validate_scan_dpi(config.default_dpi, config.default_dpi)?;
    crate::core::checked_image_len(
        config.default_width,
        config.default_height,
        crate::core::PixelFormat::Rgb8.bpp(),
    )?;
    if !(1..=crate::device::MAX_SCAN_PAGES).contains(&config.batch_pages) {
        return Err(ScanError::Invalid(format!(
            "batch pages must be in 1..={}",
            crate::device::MAX_SCAN_PAGES
        )));
    }
    validate_saturation(config.saturation)?;
    validate_hue(config.hue)?;
    if let Some(points) = config.curves.as_deref() {
        parse_curve_points(&format_curve_points(Some(points)))?;
    }
    if let Some([x, y, width, height]) = config.crop {
        if x < 0 || y < 0 || width < 1 || height < 1 {
            return Err(ScanError::Invalid(
                "config crop must be x,y,w,h with nonnegative origin and positive size".into(),
            ));
        }
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

pub(crate) fn validate_ocr_language(language: &str) -> Result<()> {
    const MAX_OCR_LANGUAGE_BYTES: usize = 64;
    let language = language.trim();
    if language.is_empty() || language.len() > MAX_OCR_LANGUAGE_BYTES {
        return Err(ScanError::Invalid(format!(
            "config OCR language must contain 1..={MAX_OCR_LANGUAGE_BYTES} UTF-8 bytes"
        )));
    }
    if language.chars().any(char::is_control) {
        return Err(ScanError::Invalid(
            "config OCR language contains control characters".into(),
        ));
    }
    Ok(())
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
    if value.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            )
    }) {
        return Err(ScanError::Invalid(
            "output name contains a path separator, control character, or Windows-reserved character"
                .into(),
        ));
    }
    let device_name = value.split('.').next().unwrap_or_default();
    let device_name = device_name.to_ascii_uppercase();
    let reserved = matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (device_name.len() == 4
            && (device_name.starts_with("COM") || device_name.starts_with("LPT"))
            && matches!(device_name.as_bytes()[3], b'1'..=b'9'));
    if reserved {
        return Err(ScanError::Invalid(
            "output name uses a Windows-reserved device name".into(),
        ));
    }
    Ok(value)
}

/// Parse tone-curve control points from `x:y,x:y` text.
///
/// A supplied curve must contain at least two points. Both components are
/// integers in 0..=255, and x values must be strictly increasing.
pub fn parse_curve_points(input: &str) -> Result<Option<Vec<[i32; 2]>>> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }

    let mut points = Vec::new();
    let mut previous_x = None;
    for (index, raw_point) in input.split(',').enumerate() {
        let point = raw_point.trim();
        let Some((raw_x, raw_y)) = point.split_once(':') else {
            return Err(ScanError::Other(format!(
                "curve point {} must be x:y",
                index + 1
            )));
        };
        if raw_y.contains(':') {
            return Err(ScanError::Other(format!(
                "curve point {} must be x:y",
                index + 1
            )));
        }
        let x = raw_x.trim().parse::<i32>().map_err(|_| {
            ScanError::Other(format!("curve point {} has an invalid x value", index + 1))
        })?;
        let y = raw_y.trim().parse::<i32>().map_err(|_| {
            ScanError::Other(format!("curve point {} has an invalid y value", index + 1))
        })?;
        if !(0..=255).contains(&x) || !(0..=255).contains(&y) {
            return Err(ScanError::Other(format!(
                "curve point {} values must be in 0..=255",
                index + 1
            )));
        }
        if previous_x.is_some_and(|previous_x| x <= previous_x) {
            return Err(ScanError::Other(
                "curve x values must be strictly increasing".into(),
            ));
        }
        previous_x = Some(x);
        points.push([x, y]);
    }

    if points.len() < 2 {
        return Err(ScanError::Other(
            "curves must contain at least two x:y points".into(),
        ));
    }
    Ok(Some(points))
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
    if let Some(parent) = cfg_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut value = serde_json::to_value(cfg)
        .map_err(|e| ScanError::Other(format!("config serialize: {e}")))?;
    value = strip_banned(&value);
    if let Value::Object(ref mut map) = value {
        map.retain(|k, _| !is_banned_key(k));
    }
    let text = serde_json::to_string_pretty(&value)? + "\n";
    crate::atomic_write::write_file_atomic(&cfg_path, text.as_bytes())?;
    Ok(cfg_path)
}
