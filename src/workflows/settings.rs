//! Pure resolution from durable settings into typed workflow defaults.

use crate::domain::acquisition::ScanMode;
use crate::domain::image::{Rect, Rotate};
use crate::domain::processing::PipelinePrefs;
use crate::error::{Result, ScanError};
use serde::{Deserialize, Serialize};

/// Durable application settings. JSON adapters serialize this exact value type.
///
/// Unknown JSON fields are ignored through Serde's default behavior, while
/// omitted fields are supplied by [`Default`].
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
    /// Map durable processing fields onto typed pipeline preferences.
    pub fn to_pipeline_prefs(&self) -> PipelinePrefs {
        resolve_defaults(self).processing
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

#[derive(Debug, Clone, PartialEq)]
pub struct AcquisitionDefaults {
    pub device_id: String,
    pub mode: ScanMode,
    pub duplex: bool,
    pub dpi_x: u32,
    pub dpi_y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSettings {
    pub acquisition: AcquisitionDefaults,
    pub processing: PipelinePrefs,
}

/// Resolve durable JSON fields without I/O or validation side effects.
pub fn resolve_defaults(config: &AppConfig) -> ResolvedSettings {
    let crop = config.crop.map(|value| {
        Rect::new(
            value[0],
            value[1],
            value[2].max(0) as u32,
            value[3].max(0) as u32,
        )
    });
    let processing = PipelinePrefs {
        rotate: Rotate::from_degrees(config.rotate),
        flip_h: config.flip_h,
        flip_v: config.flip_v,
        crop,
        brightness: config.brightness,
        contrast: config.contrast,
        desaturate: config.desaturate,
        levels_black: config.levels_black,
        levels_white: config.levels_white,
        levels_gamma: config.levels_gamma,
        saturation: config.saturation,
        hue: config.hue,
        curves: config.curves.clone(),
        auto_deskew: config.auto_deskew,
        deskew_angle: config.deskew_angle,
        auto_orient: config.auto_orient,
        auto_crop: config.auto_crop,
        white_balance: config.white_balance,
        sharpen_amount: config.sharpen_amount,
        auto_levels: config.auto_levels,
        infrared_clean: config.infrared_clean.clone(),
        descreen: config.descreen,
        descreen_dpi: config.descreen_dpi,
        restore_colors: config.restore_colors,
        restore_fading: config.restore_fading,
        grain_reduction: config.grain_reduction.clone(),
        flatten: config.flatten,
        hole_punch: config.hole_punch,
        colorize_mode: config.colorize_mode.clone(),
        film_type: config.film_type.clone(),
        ..PipelinePrefs::default()
    };
    ResolvedSettings {
        acquisition: AcquisitionDefaults {
            device_id: config.last_device_id.clone(),
            mode: config.scan_mode,
            duplex: config.duplex,
            dpi_x: config.default_dpi,
            dpi_y: config.default_dpi,
            width: config.default_width,
            height: config.default_height,
        },
        processing,
    }
}
