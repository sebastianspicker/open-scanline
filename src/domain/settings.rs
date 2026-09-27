//! Pure validation and parsing rules for durable application settings.

use crate::error::{Result, ScanError};
use serde_json::Value;

/// Maximum UTF-8 length for a GUI output file stem. This leaves room for the
/// fixed suffixes/extensions on filesystems whose component limit is 255 bytes.
pub const MAX_OUTPUT_NAME_BYTES: usize = 128;

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

type ParsedCurvePoints = Option<Vec<[i32; 2]>>;

struct CurvePoint {
    x: i32,
    y: i32,
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

/// Validate an OCR language tag accepted by the CLI, GUI, and durable config.
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
