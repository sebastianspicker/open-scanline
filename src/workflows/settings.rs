//! Pure resolution from durable settings into typed workflow defaults.

use crate::domain::acquisition::{PaperSize, ScanMode};
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

/// Resolve the requested scan extent in pixels; 0 on an axis means the device
/// maximum. Per axis: explicit pixels, then explicit paper, then configured
/// paper, then configured pixels. An explicit `Auto` paper skips the
/// configured paper.
pub fn resolve_scan_extent(
    config: &AppConfig,
    paper: Option<PaperSize>,
    width: Option<u32>,
    height: Option<u32>,
    dpi: u32,
) -> (u32, u32) {
    let paper_pixels = match paper {
        Some(paper) => paper.to_pixels(dpi, dpi),
        None => PaperSize::parse(&config.paper_size)
            .ok()
            .and_then(|paper| paper.to_pixels(dpi, dpi)),
    };
    (
        width
            .or(paper_pixels.map(|extent| extent.0))
            .unwrap_or_else(|| configured_pixels(config.default_width, config, dpi)),
        height
            .or(paper_pixels.map(|extent| extent.1))
            .unwrap_or_else(|| configured_pixels(config.default_height, config, dpi)),
    )
}

/// Rescale a configured pixel length from `default_dpi` to `dpi`, rounding. On
/// overflow the value is kept so the usual extent validation rejects the request.
fn configured_pixels(pixels: u32, config: &AppConfig, dpi: u32) -> u32 {
    if pixels == 0 || config.default_dpi == 0 || dpi == config.default_dpi {
        return pixels;
    }
    let from = u64::from(config.default_dpi);
    let scaled = (u64::from(pixels) * u64::from(dpi) + from / 2) / from;
    u32::try_from(scaled).unwrap_or(pixels)
}

fn resolve_acquisition_defaults(config: &AppConfig) -> AcquisitionDefaults {
    let (width, height) = resolve_scan_extent(config, None, None, None, config.default_dpi);
    AcquisitionDefaults {
        device_id: config.last_device_id.clone(),
        mode: config.scan_mode,
        duplex: config.duplex,
        dpi_x: config.default_dpi,
        dpi_y: config.default_dpi,
        width,
        height,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_pixels_follow_dpi() {
        assert_eq!(PaperSize::A4.to_pixels(300, 300), Some((2480, 3508)));
        assert_eq!(PaperSize::Auto.to_pixels(300, 300), None);
        assert_eq!(PaperSize::parse("Letter").unwrap(), PaperSize::Letter);
        assert!(PaperSize::parse("tabloid").is_err());
    }

    #[test]
    fn extent_precedence_is_per_axis() {
        let mut config = AppConfig::default();
        config.default_dpi = 300;
        assert_eq!(resolve_scan_extent(&config, None, None, None, 300), (0, 0));
        config.default_width = 100;
        config.default_height = 200;
        assert_eq!(
            resolve_scan_extent(&config, None, None, None, 300),
            (100, 200)
        );
        config.paper_size = "a5".into();
        assert_eq!(
            resolve_scan_extent(&config, None, None, None, 300),
            (1748, 2480)
        );
        assert_eq!(
            resolve_scan_extent(&config, Some(PaperSize::A4), None, None, 300),
            (2480, 3508)
        );
        assert_eq!(
            resolve_scan_extent(&config, Some(PaperSize::A4), Some(50), None, 300),
            (50, 3508)
        );
        assert_eq!(
            resolve_scan_extent(&config, Some(PaperSize::Auto), None, None, 300),
            (100, 200)
        );
        config.default_dpi = 150;
        config.default_width = 1240;
        config.default_height = 1754;
        config.paper_size = "auto".into();
        assert_eq!(
            resolve_scan_extent(&config, None, None, None, 600),
            (4960, 7016)
        );
        assert_eq!(
            resolve_scan_extent(&config, None, Some(50), None, 600),
            (50, 7016)
        );
        config.default_width = u32::MAX;
        assert_eq!(
            resolve_scan_extent(&config, None, None, None, 600).0,
            u32::MAX
        );
    }
}
