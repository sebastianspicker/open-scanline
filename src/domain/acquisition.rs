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

/// Physical scan area. `Auto` means the device's maximum area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PaperSize {
    #[default]
    Auto,
    A4,
    A5,
    A6,
    Letter,
    Legal,
}

impl PaperSize {
    pub const NAMES: [&'static str; 6] = ["auto", "a4", "a5", "a6", "letter", "legal"];
    pub const ALL: [PaperSize; 6] = [
        Self::Auto,
        Self::A4,
        Self::A5,
        Self::A6,
        Self::Letter,
        Self::Legal,
    ];

    pub fn parse(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|paper| value.trim().eq_ignore_ascii_case(paper.name()))
            .ok_or_else(|| {
                ScanError::Invalid(format!(
                    "paper size must be one of: {}",
                    Self::NAMES.join(", ")
                ))
            })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::A4 => "a4",
            Self::A5 => "a5",
            Self::A6 => "a6",
            Self::Letter => "letter",
            Self::Legal => "legal",
        }
    }

    /// Portrait width and height in millimetres; `None` for `Auto`.
    pub fn dimensions_mm(self) -> Option<(f64, f64)> {
        match self {
            Self::Auto => None,
            Self::A4 => Some((210.0, 297.0)),
            Self::A5 => Some((148.0, 210.0)),
            Self::A6 => Some((105.0, 148.0)),
            Self::Letter => Some((215.9, 279.4)),
            Self::Legal => Some((215.9, 355.6)),
        }
    }

    /// Pixel extent at the given resolutions; `None` for `Auto`.
    pub fn to_pixels(self, dpi_x: u32, dpi_y: u32) -> Option<(u32, u32)> {
        let (width_mm, height_mm) = self.dimensions_mm()?;
        let pixels = |mm: f64, dpi: u32| (mm / 25.4 * f64::from(dpi)).round().max(1.0) as u32;
        Some((pixels(width_mm, dpi_x), pixels(height_mm, dpi_y)))
    }
}

/// Acquisition request. `width`/`height` are pixels at `dpi_x`/`dpi_y`; zero
/// means the device's default (maximum) extent on that axis.
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
            // Zero is the device maximum; the mock renders it as 320x240.
            width: 0,
            height: 0,
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

/// Upper bound for one acquisition job.
pub const MAX_SCAN_PAGES: u32 = 1_000;

pub(crate) fn reject_single_page_duplex(request: &ScanRequest) -> Result<()> {
    if request.duplex {
        return Err(ScanError::Unsupported(
            "duplex acquisition produces multiple sides; use the batch API with an even page limit"
                .into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_page_limit(max_pages: u32) -> Result<()> {
    if max_pages == 0 {
        return Err(ScanError::Invalid("page limit must be positive".into()));
    }
    if max_pages > MAX_SCAN_PAGES {
        return Err(ScanError::Invalid(format!(
            "page limit {max_pages} exceeds the supported maximum of {MAX_SCAN_PAGES} logical sides"
        )));
    }
    Ok(())
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
