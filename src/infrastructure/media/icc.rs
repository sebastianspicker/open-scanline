//! ICC / IT8 profiling helpers (OSL-ICC) — pure Rust, no proprietary blobs.

use crate::domain::image::{ImageBuffer, PixelFormat};
use crate::error::{Result, ScanError};
use serde_json::{json, Value};
use std::ops::Range;
use std::path::Path;

pub const IT8_COLS: usize = 6;
pub const IT8_TOTAL_ROWS: usize = 5;
pub const PROFILE_FORMAT: &str = "open-scanline-icc-profile";
pub const PROFILE_VERSION: u32 = 1;
/// Scanner profile JSON contains only calibration metadata. Four MiB leaves
/// ample room for future profile fields while bounding untrusted input.
pub const MAX_SCANNER_PROFILE_JSON_BYTES: u64 = 4 * 1024 * 1024;

/// Read and strictly validate an Open Scanline scanner-profile JSON file.
pub fn load_scanner_profile(path: impl AsRef<Path>) -> Result<Value> {
    let path = path.as_ref();
    let contents = crate::infrastructure::config::json::read_bounded_json_file(
        path,
        MAX_SCANNER_PROFILE_JSON_BYTES,
        "scanner profile JSON",
    )?;
    let profile: Value = serde_json::from_str(&contents).map_err(|error| {
        ScanError::Invalid(format!(
            "invalid scanner profile {}: {error}",
            path.display()
        ))
    })?;
    validate_scanner_profile(&profile)?;
    Ok(profile)
}

/// Validate the portable scanner-profile fields accepted at export time.
pub fn validate_scanner_profile(profile: &Value) -> Result<()> {
    validate_profile_header(profile)?;
    validate_profile_matrix(profile)?;
    validate_profile_gamma(profile)
}

fn validate_profile_header(profile: &Value) -> Result<()> {
    if profile.get("format").and_then(Value::as_str) != Some(PROFILE_FORMAT) {
        return Err(ScanError::Invalid(
            "scanner profile has an unsupported format".into(),
        ));
    }
    if profile.get("version").and_then(Value::as_u64) != Some(PROFILE_VERSION as u64) {
        return Err(ScanError::Invalid(
            "scanner profile has an unsupported version".into(),
        ));
    }
    if profile.get("kind").and_then(Value::as_str) != Some("scanner_it8") {
        return Err(ScanError::Invalid(
            "scanner profile has an unsupported kind".into(),
        ));
    }
    Ok(())
}

fn validate_profile_matrix(profile: &Value) -> Result<()> {
    let matrix = profile
        .get("matrix")
        .and_then(Value::as_array)
        .filter(|rows| rows.len() == 3)
        .ok_or_else(|| ScanError::Invalid("scanner profile matrix must be 3x3".into()))?;
    for row in matrix {
        validate_profile_matrix_row(row)?;
    }
    Ok(())
}

fn validate_profile_matrix_row(row: &Value) -> Result<()> {
    let row = row
        .as_array()
        .filter(|values| values.len() == 3)
        .ok_or_else(|| ScanError::Invalid("scanner profile matrix must be 3x3".into()))?;
    for value in row {
        validate_profile_matrix_value(value)?;
    }
    Ok(())
}

fn validate_profile_matrix_value(value: &Value) -> Result<()> {
    let value = value.as_f64().ok_or_else(|| {
        ScanError::Invalid("scanner profile matrix entries must be numbers".into())
    })?;
    if !value.is_finite() || value.abs() > 16.0 {
        return Err(ScanError::Invalid(
            "scanner profile matrix entries must be finite and within +/-16".into(),
        ));
    }
    Ok(())
}

fn validate_profile_gamma(profile: &Value) -> Result<()> {
    let gamma = profile
        .get("gamma")
        .and_then(Value::as_array)
        .filter(|values| values.len() == 3)
        .ok_or_else(|| {
            ScanError::Invalid("scanner profile gamma must contain three values".into())
        })?;
    for value in gamma {
        let value = value.as_f64().ok_or_else(|| {
            ScanError::Invalid("scanner profile gamma entries must be numbers".into())
        })?;
        if !value.is_finite() || !(0.05..=8.0).contains(&value) {
            return Err(ScanError::Invalid(
                "scanner profile gamma entries must be finite and within 0.05..=8".into(),
            ));
        }
    }
    Ok(())
}

fn clamp_byte(v: f64) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

type Rgb = (f64, f64, f64);

fn target_cell_range(index: usize, cell_size: u32, last_index: usize, limit: u32) -> Range<u32> {
    let start = index as u32 * cell_size;
    let end = if index == last_index {
        limit
    } else {
        (index as u32 + 1) * cell_size
    };
    start..end
}

fn paint_rgb_rect(data: &mut [u8], width: u32, x_range: Range<u32>, y_range: Range<u32>, rgb: Rgb) {
    let (red, green, blue) = rgb;
    let rgb = (
        clamp_byte(red * 255.0),
        clamp_byte(green * 255.0),
        clamp_byte(blue * 255.0),
    );
    for y in y_range {
        for x in x_range.clone() {
            let offset = ((y * width + x) * 3) as usize;
            data[offset] = rgb.0;
            data[offset + 1] = rgb.1;
            data[offset + 2] = rgb.2;
        }
    }
}

fn fill_it8_target_data(width: u32, height: u32, patches: &[Rgb]) -> Vec<u8> {
    let mut data = vec![0u8; (width * height * 3) as usize];
    let cell_width = width / IT8_COLS as u32;
    let cell_height = height / IT8_TOTAL_ROWS as u32;
    for row in 0..IT8_TOTAL_ROWS {
        let y_range = target_cell_range(row, cell_height, IT8_TOTAL_ROWS - 1, height);
        for col in 0..IT8_COLS {
            let x_range = target_cell_range(col, cell_width, IT8_COLS - 1, width);
            paint_rgb_rect(
                &mut data,
                width,
                x_range,
                y_range.clone(),
                patches[row * IT8_COLS + col],
            );
        }
    }
    data
}

/// Reference linear RGB (0..1) for the synthetic IT8 patch layout.
pub fn it8_reference_patches() -> Vec<(f64, f64, f64)> {
    let mut patches = Vec::with_capacity(IT8_COLS * IT8_TOTAL_ROWS);
    let bases = [
        (1.0, 0.0, 0.0),
        (0.0, 1.0, 0.0),
        (0.0, 0.0, 1.0),
        (1.0, 1.0, 0.0),
        (0.0, 1.0, 1.0),
        (1.0, 0.0, 1.0),
    ];
    let scales = [1.0, 0.75, 0.50, 0.30];
    for s in scales {
        for (r, g, b) in bases {
            patches.push((r * s, g * s, b * s));
        }
    }
    for i in 0..IT8_COLS {
        let y = if IT8_COLS > 1 {
            i as f64 / (IT8_COLS as f64 - 1.0)
        } else {
            0.0
        };
        patches.push((y, y, y));
    }
    patches
}

/// Generate a color patch chart for profiling workflows.
pub fn make_it8_target_image(width: u32, height: u32) -> Result<ImageBuffer> {
    if width < IT8_COLS as u32 || height < IT8_TOTAL_ROWS as u32 {
        return Err(ScanError::Invalid(format!(
            "make_it8_target_image: need at least {IT8_COLS}x{IT8_TOTAL_ROWS}"
        )));
    }
    ImageBuffer::new(
        width,
        height,
        PixelFormat::Rgb8,
        fill_it8_target_data(width, height, &it8_reference_patches()),
    )
}

fn centered_sample_range(index: usize, cell_size: usize, limit: usize) -> Range<usize> {
    let start = index * cell_size + cell_size / 4;
    let end = (index * cell_size + (3 * cell_size) / 4)
        .max(start + 1)
        .min(limit);
    start..end
}

fn normalized_mean(image: &ImageBuffer, x_range: Range<usize>, y_range: Range<usize>) -> Rgb {
    let mut sums = [0.0; 3];
    let mut count = 0u32;
    for y in y_range {
        for x in x_range.clone() {
            let offset = (y * image.width as usize + x) * 3;
            sums[0] += image.data[offset] as f64;
            sums[1] += image.data[offset + 1] as f64;
            sums[2] += image.data[offset + 2] as f64;
            count += 1;
        }
    }
    if count == 0 {
        return (0.0, 0.0, 0.0);
    }
    let scale = 1.0 / (count as f64 * 255.0);
    (sums[0] * scale, sums[1] * scale, sums[2] * scale)
}

fn sample_patch_means(image: &ImageBuffer) -> Result<Vec<Rgb>> {
    if image.pixel_format != PixelFormat::Rgb8 {
        return Err(ScanError::Unsupported(format!(
            "IT8 sample: {}",
            image.pixel_format.as_str()
        )));
    }
    let w = image.width as usize;
    let h = image.height as usize;
    let cell_width = (w / IT8_COLS).max(1);
    let cell_height = (h / IT8_TOTAL_ROWS).max(1);
    let mut means = Vec::with_capacity(IT8_COLS * IT8_TOTAL_ROWS);
    for row in 0..IT8_TOTAL_ROWS {
        let y_range = centered_sample_range(row, cell_height, h);
        for col in 0..IT8_COLS {
            let x_range = centered_sample_range(col, cell_width, w);
            means.push(normalized_mean(image, x_range, y_range.clone()));
        }
    }
    Ok(means)
}

/// Gain for one channel; a dead channel or a gain `validate_scanner_profile` rejects is a bad capture.
fn it8_gain(measured: f64, reference: f64) -> Result<f64> {
    let gain = reference / measured;
    if measured <= 1e-9 || !gain.is_finite() || gain > 16.0 {
        return Err(ScanError::Invalid(
            "IT8 capture is too dark or a channel is dead; check the lid and lamp".into(),
        ));
    }
    Ok(gain)
}

/// Fit a simple per-channel gain + gamma profile from scanned IT8 chart.
pub fn profile_scanner_it8(image: &ImageBuffer) -> Result<Value> {
    let measured = sample_patch_means(image)?;
    let refs = it8_reference_patches();
    let n = measured.len().min(refs.len());
    let mut sum_mr = 0.0;
    let mut sum_mg = 0.0;
    let mut sum_mb = 0.0;
    let mut sum_rr = 0.0;
    let mut sum_rg = 0.0;
    let mut sum_rb = 0.0;
    for i in 0..n {
        sum_mr += measured[i].0;
        sum_mg += measured[i].1;
        sum_mb += measured[i].2;
        sum_rr += refs[i].0;
        sum_rg += refs[i].1;
        sum_rb += refs[i].2;
    }
    let gain_r = it8_gain(sum_mr, sum_rr)?;
    let gain_g = it8_gain(sum_mg, sum_rg)?;
    let gain_b = it8_gain(sum_mb, sum_rb)?;
    Ok(json!({
        "format": PROFILE_FORMAT,
        "version": PROFILE_VERSION,
        "kind": "scanner_it8",
        "matrix": [
            [gain_r, 0.0, 0.0],
            [0.0, gain_g, 0.0],
            [0.0, 0.0, gain_b],
        ],
        "gamma": [1.0, 1.0, 1.0],
        "patches": n,
        "ok": true,
    }))
}

pub(crate) struct PreparedScannerProfile {
    matrix: [[f64; 3]; 3],
    inverse_gamma: [f64; 3],
}

pub(crate) fn prepare_scanner_profile(profile: &Value) -> Result<PreparedScannerProfile> {
    validate_scanner_profile(profile)?;
    Ok(PreparedScannerProfile {
        matrix: parsed_profile_matrix(profile)?,
        inverse_gamma: parsed_inverse_gamma(profile)?,
    })
}

fn parsed_profile_matrix(profile: &Value) -> Result<[[f64; 3]; 3]> {
    let mut matrix = [[0.0; 3]; 3];
    for (row, values) in matrix.iter_mut().enumerate() {
        for (column, value) in values.iter_mut().enumerate() {
            *value = profile["matrix"][row][column]
                .as_f64()
                .ok_or_else(|| ScanError::Invalid("scanner profile matrix entry missing".into()))?;
        }
    }
    Ok(matrix)
}

fn parsed_inverse_gamma(profile: &Value) -> Result<[f64; 3]> {
    let mut inverse_gamma = [1.0; 3];
    for (channel, value) in inverse_gamma.iter_mut().enumerate() {
        let gamma = profile["gamma"][channel]
            .as_f64()
            .ok_or_else(|| ScanError::Invalid("scanner profile gamma entry missing".into()))?;
        *value = 1.0 / gamma;
    }
    Ok(inverse_gamma)
}

impl PreparedScannerProfile {
    /// Apply this prepared profile to an owned image, consuming its buffer.
    pub(crate) fn apply_owned(&self, mut image: ImageBuffer) -> Result<ImageBuffer> {
        match image.pixel_format {
            PixelFormat::Rgb8 => {}
            PixelFormat::Gray8 => {
                crate::debug_log::debug_log!(
                    "icc",
                    "scanner profile skipped: it applies to colour pages, not Gray8"
                );
                return Ok(image);
            }
            _ => {
                return Err(ScanError::Unsupported(
                    "apply_scanner_profile requires Rgb8".into(),
                ))
            }
        }
        for pixel in image.data.as_chunks_mut::<3>().0 {
            apply_profile_pixel(pixel, &self.matrix, &self.inverse_gamma);
        }
        ImageBuffer::new(image.width, image.height, image.pixel_format, image.data)
    }
}

fn apply_profile_pixel(pixel: &mut [u8], matrix: &[[f64; 3]; 3], inverse_gamma: &[f64; 3]) {
    let red = pixel[0] as f64 / 255.0;
    let green = pixel[1] as f64 / 255.0;
    let blue = pixel[2] as f64 / 255.0;
    let transformed = [
        mixed_channel(&matrix[0], red, green, blue, inverse_gamma[0]),
        mixed_channel(&matrix[1], red, green, blue, inverse_gamma[1]),
        mixed_channel(&matrix[2], red, green, blue, inverse_gamma[2]),
    ];
    pixel.copy_from_slice(&transformed);
}

fn mixed_channel(matrix: &[f64; 3], red: f64, green: f64, blue: f64, inverse_gamma: f64) -> u8 {
    let mixed = (matrix[0] * red + matrix[1] * green + matrix[2] * blue).clamp(0.0, 1.0);
    // Exact for 1.0; the IT8 profiler always writes gamma 1.0, so skip the per-sample powf.
    let curved = if inverse_gamma == 1.0 {
        mixed
    } else {
        mixed.powf(inverse_gamma)
    };
    clamp_byte(curved * 255.0)
}

/// Save IT8 profile JSON to path.
pub fn save_profile_json(path: impl AsRef<Path>, profile: &Value) -> Result<()> {
    validate_scanner_profile(profile)?;
    let s = serde_json::to_string_pretty(profile).map_err(|e| ScanError::Other(e.to_string()))?;
    crate::infrastructure::runtime::atomic_publish::write_file_atomic(path.as_ref(), s.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(gamma: f64) -> PreparedScannerProfile {
        prepare_scanner_profile(&json!({
            "format": PROFILE_FORMAT,
            "version": PROFILE_VERSION,
            "kind": "scanner_it8",
            "matrix": [[1.1, 0.0, 0.0], [0.0, 0.9, 0.0], [0.0, 0.0, 1.0]],
            "gamma": [gamma, gamma, gamma],
        }))
        .unwrap()
    }

    #[test]
    fn black_it8_capture_is_rejected() {
        let black = ImageBuffer::new(24, 16, PixelFormat::Rgb8, vec![0; 24 * 16 * 3]).unwrap();
        assert!(matches!(
            profile_scanner_it8(&black),
            Err(ScanError::Invalid(_))
        ));
    }

    #[test]
    fn unit_gamma_matches_powf_for_every_byte() {
        let prepared = profile(1.0);
        for value in 0..=255u8 {
            let mut fast = [value, value, value];
            apply_profile_pixel(&mut fast, &prepared.matrix, &prepared.inverse_gamma);
            let red = value as f64 / 255.0;
            let expected =
                clamp_byte((prepared.matrix[0][0] * red).clamp(0.0, 1.0).powf(1.0) * 255.0);
            assert_eq!(fast[0], expected);
        }
        // A non-unit gamma still takes the `powf` path.
        let curved = profile(2.2);
        for value in 0..=255u8 {
            let mut pixel = [value, value, value];
            apply_profile_pixel(&mut pixel, &curved.matrix, &curved.inverse_gamma);
            let red = value as f64 / 255.0;
            let expected = clamp_byte(
                (curved.matrix[0][0] * red)
                    .clamp(0.0, 1.0)
                    .powf(curved.inverse_gamma[0])
                    * 255.0,
            );
            assert_eq!(pixel[0], expected);
        }
    }

    #[test]
    fn gray_pages_skip_the_scanner_profile() {
        let gray = ImageBuffer::new(2, 1, PixelFormat::Gray8, vec![10, 200]).unwrap();
        let out = profile(1.0).apply_owned(gray.clone()).unwrap();
        assert_eq!(out, gray);
    }
}
