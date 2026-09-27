//! Durable application settings value type (OSL-CONFIG on-disk shape).

use crate::domain::acquisition::ScanMode;
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

macro_rules! default_app_config {
    () => {
        AppConfig {
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
    };
}

impl Default for AppConfig {
    fn default() -> Self {
        default_app_config!()
    }
}
