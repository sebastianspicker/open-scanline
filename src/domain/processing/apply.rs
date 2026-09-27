//! Pipeline orchestration: `apply_pipeline`.

use super::auto::{apply_auto_crop, apply_orientation};
use super::color::{
    adjust_brightness_contrast, adjust_hue, adjust_levels, adjust_saturation, apply_curves,
    auto_levels, desaturate, invert, white_balance,
};
use super::filters::{
    colorize, descreen, flatten, grain_reduction, hole_punch_removal, infrared_clean,
    restore_colors, restore_fading, sharpen,
};
use super::geometry::{auto_deskew, crop, deskew, flip_horizontal, flip_vertical, rotate};
use crate::domain::image::{ImageBuffer, Rotate};
use crate::domain::processing::PipelinePrefs;
use crate::error::Result;

/// Apply geometry, color, and filter-tab chain from `prefs`.
pub fn apply_pipeline(image: &ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    apply_pipeline_owned(image.clone(), prefs)
}

pub(crate) fn apply_pipeline_owned(
    image: ImageBuffer,
    prefs: &PipelinePrefs,
) -> Result<ImageBuffer> {
    let image = apply_geometry(image, prefs)?;
    let image = apply_color(image, prefs)?;
    let image = apply_filters(image, prefs)?;
    let image = apply_film(image, prefs)?;
    apply_auto_operations(image, prefs)
}

/// Apply content-aware operations last, preserving the CLI's former order
/// while keeping them in-memory before the final encoder is selected.
fn apply_auto_operations(mut image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if prefs.auto_orient {
        image = apply_orientation(&image)?;
    }
    if prefs.auto_crop {
        image = apply_auto_crop(&image)?;
    }
    Ok(image)
}

fn apply_geometry(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    let out = apply_crop(image, prefs)?;
    let out = apply_rotation(out, prefs)?;
    let out = apply_horizontal_flip(out, prefs)?;
    let out = apply_vertical_flip(out, prefs)?;
    apply_deskewing(out, prefs)
}

fn apply_crop(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    match prefs.crop {
        Some(region) => crop(&image, region),
        None => Ok(image),
    }
}

fn apply_rotation(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    match prefs.rotate {
        Rotate::None => Ok(image),
        rotation => rotate(&image, rotation),
    }
}

fn apply_horizontal_flip(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if prefs.flip_h {
        return flip_horizontal(&image);
    }
    Ok(image)
}

fn apply_vertical_flip(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if prefs.flip_v {
        return flip_vertical(&image);
    }
    Ok(image)
}

fn apply_deskewing(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if prefs.deskew_angle.abs() >= 0.05 {
        return deskew(&image, prefs.deskew_angle);
    }
    if prefs.auto_deskew {
        return auto_deskew(&image, 15.0);
    }
    Ok(image)
}

fn apply_color(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    let out = apply_tone(image, prefs)?;
    let out = apply_color_channels(out, prefs)?;
    apply_color_finishing(out, prefs)
}

fn apply_tone(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    let out = apply_brightness_contrast(image, prefs)?;
    let out = apply_desaturation(out, prefs)?;
    apply_level_adjustment(out, prefs)
}

fn apply_brightness_contrast(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if prefs.brightness == 0 && prefs.contrast == 0 {
        return Ok(image);
    }
    adjust_brightness_contrast(&image, prefs.brightness, prefs.contrast)
}

fn apply_desaturation(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if prefs.desaturate {
        return desaturate(&image);
    }
    Ok(image)
}

fn apply_level_adjustment(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    let unchanged = prefs.levels_black == 0
        && prefs.levels_white == 255
        && (prefs.levels_gamma - 1.0).abs() < 1e-9;
    if unchanged {
        return Ok(image);
    }
    adjust_levels(
        &image,
        prefs.levels_black,
        prefs.levels_white,
        prefs.levels_gamma,
    )
}

fn apply_color_channels(mut image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if prefs.white_balance {
        image = white_balance(&image)?;
    }
    if prefs.saturation.abs() >= 1e-9 {
        image = adjust_saturation(&image, prefs.saturation)?;
    }
    if prefs.hue.abs() >= 1e-9 {
        image = adjust_hue(&image, prefs.hue)?;
    }
    Ok(image)
}

fn apply_color_finishing(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    let out = apply_curve_adjustment(image, prefs)?;
    let out = apply_automatic_levels(out, prefs)?;
    let out = apply_inversion(out, prefs)?;
    apply_sharpening(out, prefs)
}

fn apply_curve_adjustment(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if let Some(ref pts) = prefs.curves {
        if pts.len() >= 2 {
            return apply_curves(&image, Some(pts.as_slice()));
        }
    }
    Ok(image)
}

fn apply_automatic_levels(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if prefs.auto_levels {
        return auto_levels(&image, 0.5);
    }
    Ok(image)
}

fn apply_inversion(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if prefs.invert {
        return invert(&image);
    }
    Ok(image)
}

fn apply_sharpening(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if prefs.sharpen_amount > 0.0 {
        return sharpen(&image, prefs.sharpen_amount);
    }
    Ok(image)
}

fn apply_filters(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    let out = apply_cleanup_filters(image, prefs)?;
    apply_restoration_filters(out, prefs)
}

fn apply_cleanup_filters(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    let out = apply_infrared_cleanup(image, prefs)?;
    let out = apply_descreening(out, prefs)?;
    apply_grain_reduction(out, prefs)
}

fn apply_infrared_cleanup(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if let Some(tier) = enabled_value(prefs.infrared_clean.as_deref(), &["off", "none", "false"]) {
        return infrared_clean(&image, Some(tier));
    }
    Ok(image)
}

fn apply_descreening(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if prefs.descreen {
        return descreen(&image, prefs.descreen_dpi);
    }
    Ok(image)
}

fn apply_grain_reduction(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if let Some(tier) = enabled_value(prefs.grain_reduction.as_deref(), &["off", "none"]) {
        return grain_reduction(&image, Some(tier));
    }
    Ok(image)
}

fn apply_restoration_filters(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    let out = apply_color_restoration(image, prefs)?;
    let out = apply_fading_restoration(out, prefs)?;
    let out = apply_flattening(out, prefs)?;
    let out = apply_hole_punch_removal(out, prefs)?;
    apply_colorization(out, prefs)
}

fn apply_color_restoration(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if prefs.restore_colors {
        return restore_colors(&image);
    }
    Ok(image)
}

fn apply_fading_restoration(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if prefs.restore_fading {
        return restore_fading(&image);
    }
    Ok(image)
}

fn apply_flattening(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if prefs.flatten {
        return flatten(&image);
    }
    Ok(image)
}

fn apply_hole_punch_removal(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if prefs.hole_punch {
        return hole_punch_removal(&image);
    }
    Ok(image)
}

fn apply_colorization(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if let Some(mode) = enabled_value(prefs.colorize_mode.as_deref(), &["off", "none"]) {
        return colorize(&image, Some(mode));
    }
    Ok(image)
}

fn apply_film(image: ImageBuffer, prefs: &PipelinePrefs) -> Result<ImageBuffer> {
    if let Some(film) = prefs
        .film_type
        .as_deref()
        .map(str::trim)
        .filter(|film| !film.is_empty())
    {
        return super::film::convert_film(&image, film);
    }

    Ok(image)
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
    use crate::domain::image::PixelFormat;

    #[test]
    fn owned_disabled_pipeline_reuses_the_input_allocation() {
        let image = ImageBuffer::new(2, 2, PixelFormat::Rgb8, (0..12).collect()).unwrap();
        let pointer = image.data.as_ptr();
        let output = apply_pipeline_owned(image, &PipelinePrefs::default()).unwrap();
        assert_eq!(output.data.as_ptr(), pointer);
    }

    #[test]
    fn borrowed_pipeline_preserves_input_and_returns_one_clone() {
        let image = ImageBuffer::new(2, 2, PixelFormat::Gray8, vec![1, 2, 3, 4]).unwrap();
        let output = apply_pipeline(&image, &PipelinePrefs::default()).unwrap();
        assert_eq!(output, image);
        assert_ne!(output.data.as_ptr(), image.data.as_ptr());
    }
}
