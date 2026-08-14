use crate::core::{ImageBuffer, PixelFormat, Result, ScanError};

use super::{clamp_f, median_filter};

pub(super) fn clean(image: &ImageBuffer, tier: &str) -> Result<ImageBuffer> {
    if !matches!(
        image.pixel_format,
        PixelFormat::Gray8 | PixelFormat::Rgb8 | PixelFormat::Rgba8
    ) {
        return Err(ScanError::Unsupported(format!(
            "infrared_clean: {:?}",
            image.pixel_format
        )));
    }
    let settings = Settings::for_tier(tier);
    let median = median_filter(image, settings.radius)?;
    let mask = residual_mask(
        &image.data,
        &median.data,
        image.bpp(),
        settings.residual_threshold,
    );
    let mut output = image.data.clone();
    replace_masked_colors(
        &mut output,
        &median.data,
        &mask,
        image.bpp(),
        settings.mask_threshold,
    );
    if tier == "heavy" {
        blend_heavy(
            &mut output,
            &median.data,
            &mask,
            image.bpp(),
            settings.mask_threshold,
        );
    }
    ImageBuffer::new(image.width, image.height, image.pixel_format, output)
}

struct Settings {
    radius: i32,
    residual_threshold: u32,
    mask_threshold: u8,
}

impl Settings {
    fn for_tier(tier: &str) -> Self {
        let (radius, residual_threshold, mask_threshold) = match tier {
            "light" => (1, 45, 40),
            "heavy" => (3, 18, 18),
            _ => (2, 30, 28),
        };
        Self {
            radius,
            residual_threshold,
            mask_threshold,
        }
    }
}

fn residual_mask(source: &[u8], median: &[u8], bpp: usize, threshold: u32) -> Vec<u8> {
    source
        .chunks_exact(bpp)
        .zip(median.chunks_exact(bpp))
        .map(|(source, median)| {
            if residual(source, median) > threshold {
                255
            } else {
                0
            }
        })
        .collect()
}

fn residual(source: &[u8], median: &[u8]) -> u32 {
    if source.len() == 1 {
        return (source[0] as i32 - median[0] as i32).unsigned_abs();
    }
    (0..3)
        .map(|channel| (source[channel] as i32 - median[channel] as i32).unsigned_abs())
        .max()
        .unwrap_or(0)
}

fn replace_masked_colors(output: &mut [u8], median: &[u8], mask: &[u8], bpp: usize, threshold: u8) {
    for ((output, median), &mask) in output
        .chunks_exact_mut(bpp)
        .zip(median.chunks_exact(bpp))
        .zip(mask)
    {
        if mask >= threshold {
            output[..bpp.min(3)].copy_from_slice(&median[..bpp.min(3)]);
        }
    }
}

fn blend_heavy(output: &mut [u8], median: &[u8], mask: &[u8], bpp: usize, threshold: u8) {
    for ((output, median), &mask) in output
        .chunks_exact_mut(bpp)
        .zip(median.chunks_exact(bpp))
        .zip(mask)
    {
        if mask >= threshold {
            for channel in 0..bpp.min(3) {
                output[channel] =
                    clamp_f(output[channel] as f64 * 0.65 + median[channel] as f64 * 0.35);
            }
        }
    }
}
