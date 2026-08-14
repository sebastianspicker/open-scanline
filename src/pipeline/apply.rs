//! Pipeline orchestration: `apply_pipeline`.

use crate::core::{ImageBuffer, PipelinePrefs, Result, Rotate};
use crate::ml::{apply_auto_crop, apply_orientation};
use crate::pipeline::color::{
    adjust_brightness_contrast, adjust_hue, adjust_levels, adjust_saturation, apply_curves,
    auto_levels, desaturate, invert, white_balance,
};
use crate::pipeline::filters::{
    colorize, descreen, flatten, grain_reduction, hole_punch_removal, infrared_clean,
    restore_colors, restore_fading, sharpen,
};
use crate::pipeline::geometry::{
    auto_deskew, crop, deskew, flip_horizontal, flip_vertical, rotate,
};

/// Apply geometry, color, and filter-tab chain from `prefs`.
pub fn apply_pipeline(image: &ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    let out = apply_geometry(image, prefs)?;
    let out = apply_color(&out, prefs)?;
    let out = apply_filters(&out, prefs)?;
    let out = apply_film(&out, prefs)?;
    apply_auto_operations(&out, prefs)
}

/// Apply content-aware operations last, preserving the CLI's former order
/// while keeping them in-memory before the final encoder is selected.
fn apply_auto_operations(image: &ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    let mut out = image.clone();
    if prefs.auto_orient {
        out = apply_orientation(&out)?;
    }
    if prefs.auto_crop {
        out = apply_auto_crop(&out)?;
    }
    Ok(out)
}

fn apply_geometry(image: &ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    let mut out = image.clone();

    if let Some(region) = prefs.crop {
        out = crop(&out, region)?;
    }
    if prefs.rotate != Rotate::None {
        out = rotate(&out, prefs.rotate)?;
    }
    if prefs.flip_h {
        out = flip_horizontal(&out)?;
    }
    if prefs.flip_v {
        out = flip_vertical(&out)?;
    }

    if prefs.deskew_angle.abs() >= 0.05 {
        out = deskew(&out, prefs.deskew_angle)?;
    } else if prefs.auto_deskew {
        out = auto_deskew(&out, 15.0)?;
    }

    Ok(out)
}

fn apply_color(image: &ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    let mut out = image.clone();

    if prefs.brightness != 0 || prefs.contrast != 0 {
        out = adjust_brightness_contrast(&out, prefs.brightness, prefs.contrast)?;
    }
    if prefs.desaturate {
        out = desaturate(&out)?;
    }
    if prefs.levels_black != 0
        || prefs.levels_white != 255
        || (prefs.levels_gamma - 1.0).abs() >= 1e-9
    {
        out = adjust_levels(
            &out,
            prefs.levels_black,
            prefs.levels_white,
            prefs.levels_gamma,
        )?;
    }
    if prefs.white_balance {
        out = white_balance(&out)?;
    }
    if prefs.saturation.abs() >= 1e-9 {
        out = adjust_saturation(&out, prefs.saturation)?;
    }
    if prefs.hue.abs() >= 1e-9 {
        out = adjust_hue(&out, prefs.hue)?;
    }
    if let Some(ref pts) = prefs.curves {
        if pts.len() >= 2 {
            out = apply_curves(&out, Some(pts.as_slice()))?;
        }
    }
    if prefs.auto_levels {
        out = auto_levels(&out, 0.5)?;
    }
    if prefs.invert {
        out = invert(&out)?;
    }
    if prefs.sharpen_amount > 0.0 {
        out = sharpen(&out, prefs.sharpen_amount)?;
    }

    Ok(out)
}

fn apply_filters(image: &ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    let mut out = image.clone();

    if let Some(tier) = enabled_value(prefs.infrared_clean.as_deref(), &["off", "none", "false"]) {
        out = infrared_clean(&out, Some(tier))?;
    }
    if prefs.descreen {
        out = descreen(&out, prefs.descreen_dpi)?;
    }
    if let Some(tier) = enabled_value(prefs.grain_reduction.as_deref(), &["off", "none"]) {
        out = grain_reduction(&out, Some(tier))?;
    }
    if prefs.restore_colors {
        out = restore_colors(&out)?;
    }
    if prefs.restore_fading {
        out = restore_fading(&out)?;
    }
    if prefs.flatten {
        out = flatten(&out)?;
    }
    if prefs.hole_punch {
        out = hole_punch_removal(&out)?;
    }
    if let Some(mode) = enabled_value(prefs.colorize_mode.as_deref(), &["off", "none"]) {
        out = colorize(&out, Some(mode))?;
    }

    Ok(out)
}

fn apply_film(image: &ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if let Some(film) = prefs
        .film_type
        .as_deref()
        .map(str::trim)
        .filter(|film| !film.is_empty())
    {
        return crate::film::convert_film(image, film);
    }

    Ok(image.clone())
}

fn enabled_value<'a>(value: Option<&'a str>, disabled_values: &[&str]) -> Option<&'a str> {
    let value = value?.trim();
    (!value.is_empty()
        && !disabled_values
            .iter()
            .any(|disabled| value.eq_ignore_ascii_case(disabled)))
    .then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::PixelFormat;

    #[test]
    fn filter_tab_ops_change_pixels() {
        let mut data = vec![40u8; 16 * 16 * 3];
        data[0] = 250;
        data[1] = 250;
        data[2] = 250;
        let img = ImageBuffer::new(16, 16, PixelFormat::Rgb8, data.clone()).unwrap();
        let prefs = PipelinePrefs {
            infrared_clean: Some("medium".into()),
            descreen: true,
            descreen_dpi: 75,
            ..PipelinePrefs::default()
        };
        let out = apply_pipeline(&img, &prefs).unwrap();
        assert_eq!(out.width, 16);
        // At least one of the filter ops should alter data for this pattern
        assert_ne!(out.data, data);
    }

    fn textlike_image() -> ImageBuffer {
        let mut data = vec![255u8; 80 * 40 * 3];
        for y in 15..25 {
            for x in 10..70 {
                let pixel = (y * 80 + x) * 3;
                data[pixel..pixel + 3].fill(20);
            }
        }
        ImageBuffer::new(80, 40, PixelFormat::Rgb8, data).unwrap()
    }

    #[test]
    fn explicit_deskew_precedes_auto_deskew() {
        let image = textlike_image();
        let explicit = PipelinePrefs {
            deskew_angle: 5.0,
            ..PipelinePrefs::default()
        };
        let both = PipelinePrefs {
            auto_deskew: true,
            ..explicit.clone()
        };

        assert_eq!(
            apply_pipeline(&image, &both).unwrap().data,
            apply_pipeline(&image, &explicit).unwrap().data
        );
    }

    #[test]
    fn deskew_and_filter_sentinels_preserve_existing_behavior() {
        let image = textlike_image();
        let auto_only = PipelinePrefs {
            auto_deskew: true,
            ..PipelinePrefs::default()
        };
        let below_explicit_threshold = PipelinePrefs {
            deskew_angle: 0.01,
            ..auto_only.clone()
        };
        assert_eq!(
            apply_pipeline(&image, &below_explicit_threshold)
                .unwrap()
                .data,
            apply_pipeline(&image, &auto_only).unwrap().data
        );

        let disabled_filters = PipelinePrefs {
            infrared_clean: Some(" false ".into()),
            grain_reduction: Some(" none ".into()),
            colorize_mode: Some(" off ".into()),
            film_type: Some("   ".into()),
            ..PipelinePrefs::default()
        };
        assert_eq!(
            apply_pipeline(&image, &disabled_filters).unwrap().data,
            image.data
        );
    }

    #[test]
    fn auto_operations_run_in_memory_after_existing_stages() {
        let image = textlike_image();
        let expected = apply_auto_crop(&apply_orientation(&image).unwrap()).unwrap();
        let actual = apply_pipeline(
            &image,
            &PipelinePrefs {
                auto_orient: true,
                auto_crop: true,
                ..PipelinePrefs::default()
            },
        )
        .unwrap();

        assert_eq!(actual, expected);
    }
}
