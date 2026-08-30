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
    let matrix = profile
        .get("matrix")
        .and_then(Value::as_array)
        .filter(|rows| rows.len() == 3)
        .ok_or_else(|| ScanError::Invalid("scanner profile matrix must be 3x3".into()))?;
    for row in matrix {
        let row = row
            .as_array()
            .filter(|values| values.len() == 3)
            .ok_or_else(|| ScanError::Invalid("scanner profile matrix must be 3x3".into()))?;
        for value in row {
            let value = value.as_f64().ok_or_else(|| {
                ScanError::Invalid("scanner profile matrix entries must be numbers".into())
            })?;
            if !value.is_finite() || value.abs() > 16.0 {
                return Err(ScanError::Invalid(
                    "scanner profile matrix entries must be finite and within +/-16".into(),
                ));
            }
        }
    }
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
    let gain_r = if sum_mr > 1e-9 { sum_rr / sum_mr } else { 1.0 };
    let gain_g = if sum_mg > 1e-9 { sum_rg / sum_mg } else { 1.0 };
    let gain_b = if sum_mb > 1e-9 { sum_rb / sum_mb } else { 1.0 };
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

/// Apply a scanner profile (diagonal gain matrix) to an image.
pub fn apply_scanner_profile(image: &ImageBuffer, profile: &Value) -> Result<ImageBuffer> {
    if image.pixel_format != PixelFormat::Rgb8 {
        return Err(ScanError::Unsupported(
            "apply_scanner_profile requires Rgb8".into(),
        ));
    }
    validate_scanner_profile(profile)?;
    let matrix = profile["matrix"].as_array().expect("validated 3x3 matrix");
    let gamma = profile["gamma"].as_array().expect("validated gamma");
    let mut out = image.data.clone();
    for i in (0..out.len()).step_by(3) {
        let input = [
            out[i] as f64 / 255.0,
            out[i + 1] as f64 / 255.0,
            out[i + 2] as f64 / 255.0,
        ];
        for channel in 0..3 {
            let row = matrix[channel].as_array().expect("validated matrix row");
            let mixed = (0..3)
                .map(|column| row[column].as_f64().expect("validated matrix entry") * input[column])
                .sum::<f64>()
                .clamp(0.0, 1.0);
            let exponent = 1.0 / gamma[channel].as_f64().expect("validated gamma entry");
            out[i + channel] = clamp_byte(mixed.powf(exponent) * 255.0);
        }
    }
    ImageBuffer::new(image.width, image.height, PixelFormat::Rgb8, out)
}

/// Encode float as ICC s15Fixed16Number (big-endian u32 bit pattern).
fn s15_fixed(value: f64) -> u32 {
    ((value * 65536.0).round() as i64 & 0xFFFF_FFFF) as u32
}

fn trc_tag(gamma: f64) -> Vec<u8> {
    let mut tag = Vec::with_capacity(16);
    tag.extend_from_slice(b"curv");
    tag.extend_from_slice(&0u32.to_be_bytes());
    tag.extend_from_slice(&1u32.to_be_bytes());
    tag.extend_from_slice(&s15_fixed(gamma).to_be_bytes());
    tag
}

fn xyz_tag(values: [f64; 3]) -> Vec<u8> {
    let mut tag = Vec::with_capacity(20);
    tag.extend_from_slice(b"XYZ ");
    tag.extend_from_slice(&0u32.to_be_bytes());
    for value in values {
        tag.extend_from_slice(&s15_fixed(value).to_be_bytes());
    }
    tag
}

fn icc_tag_payloads() -> Vec<([u8; 4], Vec<u8>)> {
    let gamma = 2.2_f64;
    let mut tags = vec![
        (*b"rTRC", trc_tag(gamma)),
        (*b"gTRC", trc_tag(gamma)),
        (*b"bTRC", trc_tag(gamma)),
        (*b"rXYZ", xyz_tag([0.4361, 0.2225, 0.0139])),
        (*b"gXYZ", xyz_tag([0.3851, 0.7169, 0.0971])),
        (*b"bXYZ", xyz_tag([0.1431, 0.0606, 0.7141])),
        (*b"wtpt", xyz_tag([0.9642, 1.0, 0.8249])),
    ];
    tags.sort_by_key(|tag| tag.0);
    tags
}

fn positioned_icc_payloads(
    tags: &[([u8; 4], Vec<u8>)],
    start_offset: usize,
) -> Vec<([u8; 4], usize, Vec<u8>)> {
    let mut offset = start_offset;
    tags.iter()
        .map(|(signature, payload)| {
            let positioned = (*signature, offset, payload.clone());
            offset += payload.len();
            positioned
        })
        .collect()
}

fn icc_description_tail(profile_id: &str) -> Vec<u8> {
    let id_bytes = profile_id.as_bytes();
    let length = id_bytes.len().min(31);
    let mut tail = Vec::with_capacity(4 + length);
    tail.extend_from_slice(&(length as u32).to_be_bytes());
    tail.extend_from_slice(&id_bytes[..length]);
    tail
}

fn icc_header(size: usize) -> Vec<u8> {
    let mut header = vec![0u8; 128];
    header[0..4].copy_from_slice(&(size as u32).to_be_bytes());
    header[4..8].copy_from_slice(b"ovue");
    header[8] = 0x02;
    header[9] = 0x10;
    header[12..16].copy_from_slice(b"mntr");
    header[16..20].copy_from_slice(b"RGB ");
    header[20..24].copy_from_slice(b"XYZ ");
    header[36..40].copy_from_slice(b"acsp");
    header[40..44].copy_from_slice(b"ovue");
    for (index, value) in [0.9642_f64, 1.0, 0.8249].iter().enumerate() {
        let offset = 68 + index * 4;
        header[offset..offset + 4].copy_from_slice(&s15_fixed(*value).to_be_bytes());
    }
    header
}

/// Minimal ICC v2 display profile bytes with TRC + matrix XYZ + white point tags.
/// Matches the established profile structure (7 tags: r/g/b TRC, r/g/b XYZ, wtpt).
pub fn build_icc_bytes(profile_id: &str) -> Vec<u8> {
    let tags = icc_tag_payloads();
    let payloads = positioned_icc_payloads(&tags, 128 + 4 + tags.len() * 12);
    let description = icc_description_tail(profile_id);
    let size = payloads
        .last()
        .map_or(128 + 4 + tags.len() * 12, |(_, offset, payload)| {
            offset + payload.len()
        })
        + description.len();

    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(&icc_header(size));
    out.extend_from_slice(&(payloads.len() as u32).to_be_bytes());
    for (signature, offset, payload) in &payloads {
        out.extend_from_slice(signature);
        out.extend_from_slice(&(*offset as u32).to_be_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    }
    for (_, _, payload) in &payloads {
        out.extend_from_slice(payload);
    }
    out.extend_from_slice(&description);
    // Fix size field in case of padding drift
    let final_size = out.len() as u32;
    out[0..4].copy_from_slice(&final_size.to_be_bytes());
    out
}

/// Save IT8 profile JSON to path.
pub fn save_profile_json(path: impl AsRef<Path>, profile: &Value) -> Result<()> {
    validate_scanner_profile(profile)?;
    let s = serde_json::to_string_pretty(profile).map_err(|e| ScanError::Other(e.to_string()))?;
    crate::infrastructure::runtime::atomic_publish::write_file_atomic(path.as_ref(), s.as_bytes())?;
    Ok(())
}
