//! Pure acquisition values, policies, and image calibration operations.

use crate::domain::image::{ImageBuffer, PixelFormat, Rect};
use crate::domain::processing::PipelinePrefs;
use crate::error::{Result, ScanError};
use serde::{Deserialize, Serialize};

/// Lowest physical acquisition resolution accepted by shared scan paths.
pub const MIN_SCAN_DPI: u32 = 50;

pub(crate) fn validate_scan_dpi(dpi_x: u32, dpi_y: u32) -> Result<()> {
    if dpi_x < MIN_SCAN_DPI || dpi_y < MIN_SCAN_DPI {
        return Err(ScanError::Invalid(format!(
            "scan resolution must be at least {MIN_SCAN_DPI} dpi on each axis"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ScanMode {
    #[default]
    Reflective,
    Film,
    Document,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanRequest {
    pub device_id: String,
    pub mode: ScanMode,
    pub duplex: bool,
    pub dpi_x: u32,
    pub dpi_y: u32,
    pub width: u32,
    pub height: u32,
    pub pixel_format: PixelFormat,
    /// Optional device-space acquisition region.
    pub region: Option<Rect>,
    pub seed: u32,
    pub pipeline: PipelinePrefs,
}

impl Default for ScanRequest {
    fn default() -> Self {
        Self {
            device_id: "mock".into(),
            mode: ScanMode::Reflective,
            duplex: false,
            dpi_x: 150,
            dpi_y: 150,
            width: 320,
            height: 240,
            pixel_format: PixelFormat::Rgb8,
            region: None,
            seed: 1,
            pipeline: PipelinePrefs::default(),
        }
    }
}

/// Progress callback payload for scan/batch.
#[derive(Debug, Clone)]
pub struct ScanProgress {
    pub phase: String,
    pub percent: f64,
    pub message: String,
}

impl ScanProgress {
    pub fn new(phase: impl Into<String>, percent: f64, message: impl Into<String>) -> Self {
        Self {
            phase: phase.into(),
            percent,
            message: message.into(),
        }
    }
}

/// Controls opt-in device-opening exceptions. Defaults remain conservative.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeviceOpenPolicy {
    pub allow_unlisted_escl: bool,
}

/// Column-wise dark subtract + flat-field multiply (`apply_flat_dark_cal`).
pub fn apply_flat_dark_cal(img: &ImageBuffer, dark: &[i32], flat: &[f64]) -> Result<ImageBuffer> {
    if flat.is_empty() {
        return Ok(img.clone());
    }
    let ntab = flat.len();
    let w = img.width as usize;
    let h = img.height as usize;
    let bpp = img.bpp();
    let mut out = img.data.clone();
    let mut i = 0usize;
    for _y in 0..h {
        for x in 0..w {
            let idx = if w > 1 { (x * (ntab - 1)) / (w - 1) } else { 0 };
            let d = dark.get(idx).copied().unwrap_or(0);
            let f = flat.get(idx).copied().unwrap_or(1.0);
            let chans = if img.pixel_format == PixelFormat::Gray8 {
                1
            } else {
                3.min(bpp)
            };
            for channel in 0..chans {
                let value = out[i + channel] as i32;
                let corrected = (((value - d).max(0) as f64) * f + d as f64 * 0.25).round() as i32;
                out[i + channel] = corrected.clamp(0, 255) as u8;
            }
            i += bpp;
        }
    }
    ImageBuffer::new(img.width, img.height, img.pixel_format, out)
}

pub fn synthetic_cal_tables() -> (Vec<i32>, Vec<f64>) {
    let n = 64;
    let dark = vec![2_i32; n];
    let flat = (0..n)
        .map(|index| {
            let t = index as f64 / (n - 1) as f64;
            ((1.0 + 0.04 * (t - 0.5).abs() * 2.0) * 10_000.0).round() / 10_000.0
        })
        .collect();
    (dark, flat)
}
