//! Pure processing plan values.

use crate::domain::image::{Rect, Rotate};
use serde::{Deserialize, Serialize};

/// Scan / pipeline preference bundle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PipelinePrefs {
    pub rotate: Rotate,
    pub flip_h: bool,
    pub flip_v: bool,
    /// Optional post-acquisition crop.
    pub crop: Option<Rect>,
    pub brightness: i32,
    pub contrast: i32,
    pub desaturate: bool,
    pub levels_black: i32,
    pub levels_white: i32,
    pub levels_gamma: f64,
    pub auto_deskew: bool,
    pub deskew_angle: f64,
    pub auto_orient: bool,
    pub auto_crop: bool,
    pub white_balance: bool,
    pub sharpen_amount: f64,
    pub invert: bool,
    pub auto_levels: bool,
    pub saturation: f64,
    pub hue: f64,
    pub curves: Option<Vec<[i32; 2]>>,
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
}

impl Default for PipelinePrefs {
    fn default() -> Self {
        Self {
            rotate: Rotate::None,
            flip_h: false,
            flip_v: false,
            crop: None,
            brightness: 0,
            contrast: 0,
            desaturate: false,
            levels_black: 0,
            levels_white: 255,
            levels_gamma: 1.0,
            auto_deskew: false,
            deskew_angle: 0.0,
            auto_orient: false,
            auto_crop: false,
            white_balance: false,
            sharpen_amount: 0.0,
            invert: false,
            auto_levels: false,
            saturation: 0.0,
            hue: 0.0,
            curves: None,
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
        }
    }
}
