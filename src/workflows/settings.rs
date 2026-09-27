//! Pure resolution from durable settings into typed workflow defaults.

use crate::domain::acquisition::ScanMode;
use crate::domain::image::{Rect, Rotate};
use crate::domain::processing::PipelinePrefs;
use crate::infrastructure::config::AppConfig;

impl AppConfig {
    /// Map durable processing fields onto typed pipeline preferences.
    pub fn to_pipeline_prefs(&self) -> PipelinePrefs {
        resolve_defaults(self).processing
    }
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
    ResolvedSettings {
        acquisition: resolve_acquisition_defaults(config),
        processing: resolve_processing_defaults(config),
    }
}

fn resolve_acquisition_defaults(config: &AppConfig) -> AcquisitionDefaults {
    AcquisitionDefaults {
        device_id: config.last_device_id.clone(),
        mode: config.scan_mode,
        duplex: config.duplex,
        dpi_x: config.default_dpi,
        dpi_y: config.default_dpi,
        width: config.default_width,
        height: config.default_height,
    }
}

fn resolve_processing_defaults(config: &AppConfig) -> PipelinePrefs {
    let crop = resolve_crop(config.crop);
    PipelinePrefs {
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
    }
}

fn resolve_crop(crop: Option<[i32; 4]>) -> Option<Rect> {
    crop.map(|value| {
        Rect::new(
            value[0],
            value[1],
            value[2].max(0) as u32,
            value[3].max(0) as u32,
        )
    })
}
