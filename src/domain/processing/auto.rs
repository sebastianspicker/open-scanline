//! Pure auto-crop and cardinal-orientation algorithms.

use crate::domain::image::{image_to_luma8, ImageBuffer, Rect, Rotate};
use crate::error::Result;

#[derive(Debug, Clone)]
pub struct AutoCropResult {
    pub rect: Rect,
    pub confidence: f64,
    pub method: String,
}

#[derive(Debug, Clone)]
pub struct OrientationResult {
    pub degrees: i32,
    pub confidence: f64,
    pub method: String,
}

/// Content-aware bounding box (threshold default 240).
pub fn auto_crop_bounds(
    image: &ImageBuffer,
    threshold: i32,
    margin: i32,
) -> Result<AutoCropResult> {
    let (width, height, gray) = image_to_luma8(image)?;
    let threshold = threshold.clamp(0, 255) as u8;
    let mut min_x = width as i32;
    let mut min_y = height as i32;
    let mut max_x = -1_i32;
    let mut max_y = -1_i32;
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            if gray[(y as u32 * width + x as u32) as usize] < threshold {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if max_x < 0 {
        return Ok(AutoCropResult {
            rect: Rect::new(0, 0, width, height),
            confidence: 0.0,
            method: "full-frame-empty".into(),
        });
    }
    min_x = (min_x - margin).max(0);
    min_y = (min_y - margin).max(0);
    max_x = (max_x + margin).min(width as i32 - 1);
    max_y = (max_y + margin).min(height as i32 - 1);
    let crop_width = (max_x - min_x + 1).max(1) as u32;
    let crop_height = (max_y - min_y + 1).max(1) as u32;
    let area_ratio = (crop_width * crop_height) as f64 / (width * height).max(1) as f64;
    Ok(AutoCropResult {
        rect: Rect::new(min_x, min_y, crop_width, crop_height),
        confidence: (1.0 - area_ratio + 0.2).clamp(0.0, 1.0),
        method: "luma-threshold-bbox".into(),
    })
}

/// Apply auto-crop at threshold 240 (product default).
pub fn apply_auto_crop(image: &ImageBuffer) -> Result<ImageBuffer> {
    apply_auto_crop_with_params(image, 240, 2)
}

pub fn apply_auto_crop_with_params(
    image: &ImageBuffer,
    threshold: i32,
    margin: i32,
) -> Result<ImageBuffer> {
    let result = auto_crop_bounds(image, threshold, margin)?;
    if result.rect.width == image.width && result.rect.height == image.height {
        return Ok(image.clone());
    }
    super::geometry::crop(image, result.rect)
}

pub fn detect_orientation(image: &ImageBuffer) -> Result<OrientationResult> {
    let (width, height, gray) = image_to_luma8(image)?;
    if width < 3 || height < 3 {
        return Ok(OrientationResult {
            degrees: 0,
            confidence: 0.0,
            method: "too-small".into(),
        });
    }
    let (gx_sum, gy_sum) = gradient_energy(width, height, &gray);
    let (degrees, confidence) = primary_orientation(gx_sum, gy_sum);
    let (degrees, confidence) =
        refine_horizontal_orientation(width, height, &gray, degrees, confidence);
    Ok(OrientationResult {
        degrees,
        confidence: confidence.min(1.0),
        method: "gradient-energy".into(),
    })
}

fn gradient_energy(width: u32, height: u32, gray: &[u8]) -> (u64, u64) {
    let mut gx_sum = 0;
    let mut gy_sum = 0;
    for y in 1..height as usize - 1 {
        for x in 1..width as usize - 1 {
            let index = y * width as usize + x;
            gx_sum += (gray[index + 1] as i32 - gray[index - 1] as i32).unsigned_abs() as u64;
            gy_sum += (gray[index + width as usize] as i32 - gray[index - width as usize] as i32)
                .unsigned_abs() as u64;
        }
    }
    (gx_sum, gy_sum)
}

fn primary_orientation(gx_sum: u64, gy_sum: u64) -> (i32, f64) {
    let total = gx_sum + gy_sum + 1;
    let horizontal = gx_sum as f64 / total as f64;
    let vertical = gy_sum as f64 / total as f64;
    if horizontal >= vertical {
        (0, horizontal)
    } else {
        (90, vertical)
    }
}

fn refine_horizontal_orientation(
    width: u32,
    height: u32,
    gray: &[u8],
    degrees: i32,
    confidence: f64,
) -> (i32, f64) {
    if degrees != 0 {
        return (degrees, confidence);
    }
    let band = (height / 20).max(1);
    let top = band_average(gray, 0, band, width);
    let bottom = band_average(gray, height - band, band, width);
    if top < bottom - 8.0 {
        return (180, (confidence + 0.1).min(1.0));
    }
    (degrees, confidence)
}

/// Mean of `rows` rows starting at `first_row` (about 5% of the height, so one noisy row can't decide).
fn band_average(gray: &[u8], first_row: u32, rows: u32, width: u32) -> f64 {
    let start = first_row as usize * width as usize;
    let len = rows as usize * width as usize;
    gray[start..start + len]
        .iter()
        .map(|&value| value as f64)
        .sum::<f64>()
        / len as f64
}

pub fn apply_orientation(image: &ImageBuffer) -> Result<ImageBuffer> {
    apply_orientation_with_degrees(image, None)
}

pub fn apply_orientation_with_degrees(
    image: &ImageBuffer,
    degrees: Option<i32>,
) -> Result<ImageBuffer> {
    let degrees = match degrees {
        Some(value) => value.rem_euclid(360),
        None => detect_orientation(image)?.degrees.rem_euclid(360),
    };
    match degrees {
        0 => Ok(image.clone()),
        90 => super::geometry::rotate(image, Rotate::R90),
        180 => super::geometry::rotate(image, Rotate::R180),
        270 => super::geometry::rotate(image, Rotate::R270),
        _ => Ok(image.clone()),
    }
}

#[cfg(test)]
mod orientation_tests {
    use super::*;
    use crate::domain::image::PixelFormat;

    fn striped(top_rows_dark: bool) -> ImageBuffer {
        let (w, h) = (40u32, 40u32);
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let high = (x / 2) % 2 == 0;
                let dark_band = (top_rows_dark && y < 2) || (!top_rows_dark && y >= h - 2);
                data.push(match (dark_band, high) {
                    (true, true) => 60,
                    (true, false) => 0,
                    (false, true) => 200,
                    (false, false) => 100,
                });
            }
        }
        ImageBuffer::new(w, h, PixelFormat::Gray8, data).unwrap()
    }

    #[test]
    fn dark_top_band_is_detected_as_upside_down() {
        assert_eq!(detect_orientation(&striped(true)).unwrap().degrees, 180);
        assert_eq!(detect_orientation(&striped(false)).unwrap().degrees, 0);
    }
}
