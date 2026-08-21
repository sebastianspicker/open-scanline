//! Core types for open-scanline (OSL-CORE).

use serde::{Deserialize, Serialize};
use std::num::ParseIntError;
use thiserror::Error;

/// Product safety ceiling for one decoded, tightly packed image buffer.
pub const MAX_IMAGE_BYTES: usize = 512 * 1024 * 1024;
/// Per-axis ceiling shared by input decoding and generated image buffers.
pub const MAX_IMAGE_DIMENSION: u32 = 65_535;
/// Lowest physical acquisition resolution accepted by the shared scan paths.
///
/// Hardware adapters historically clamped smaller requests independently,
/// which made the reported request differ from the acquisition. Rejecting the
/// value before a device is opened keeps the contract explicit.
pub const MIN_SCAN_DPI: u32 = 50;

pub(crate) fn validate_scan_dpi(dpi_x: u32, dpi_y: u32) -> Result<()> {
    if dpi_x < MIN_SCAN_DPI || dpi_y < MIN_SCAN_DPI {
        return Err(ScanError::Invalid(format!(
            "scan resolution must be at least {MIN_SCAN_DPI} dpi on each axis"
        )));
    }
    Ok(())
}

pub(crate) fn checked_image_len(width: u32, height: u32, bytes_per_pixel: usize) -> Result<usize> {
    if width == 0 || height == 0 {
        return Err(ScanError::Invalid(
            "image width and height must be positive".into(),
        ));
    }
    if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err(ScanError::Invalid(format!(
            "image dimensions exceed the {MAX_IMAGE_DIMENSION}-pixel per-axis safety limit"
        )));
    }
    let bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
        .ok_or_else(|| ScanError::Invalid("image dimensions overflow".into()))?;
    if bytes > MAX_IMAGE_BYTES {
        return Err(ScanError::Invalid(format!(
            "image buffer exceeds the {MAX_IMAGE_BYTES}-byte safety limit"
        )));
    }
    Ok(bytes)
}

#[derive(Debug)]
pub(crate) enum CropParseError {
    ComponentCount,
    InvalidInteger(ParseIntError),
}

impl std::fmt::Display for CropParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ComponentCount => formatter.write_str("crop must be x,y,w,h"),
            Self::InvalidInteger(error) => write!(formatter, "{error}"),
        }
    }
}

pub(crate) fn parse_crop_components(value: &str) -> std::result::Result<[i32; 4], CropParseError> {
    let parts = value.split(',').collect::<Vec<_>>();
    if parts.len() != 4 {
        return Err(CropParseError::ComponentCount);
    }
    let mut output = [0_i32; 4];
    for (index, part) in parts.iter().enumerate() {
        output[index] = part
            .trim()
            .parse()
            .map_err(CropParseError::InvalidInteger)?;
    }
    Ok(output)
}

/// Packed pixel layout for [`ImageBuffer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PixelFormat {
    Gray8,
    Rgb8,
    Rgba8,
}

impl PixelFormat {
    pub fn bpp(self) -> usize {
        match self {
            PixelFormat::Gray8 => 1,
            PixelFormat::Rgb8 => 3,
            PixelFormat::Rgba8 => 4,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            PixelFormat::Gray8 => "Gray8",
            PixelFormat::Rgb8 => "Rgb8",
            PixelFormat::Rgba8 => "Rgba8",
        }
    }
}

/// Rotation in degrees (clockwise).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Rotate {
    #[default]
    None = 0,
    R90 = 90,
    R180 = 180,
    R270 = 270,
}

impl Rotate {
    pub fn from_degrees(d: i32) -> Self {
        match d.rem_euclid(360) {
            90 => Rotate::R90,
            180 => Rotate::R180,
            270 => Rotate::R270,
            _ => Rotate::None,
        }
    }

    pub fn degrees(self) -> i32 {
        match self {
            Rotate::None => 0,
            Rotate::R90 => 90,
            Rotate::R180 => 180,
            Rotate::R270 => 270,
        }
    }
}

/// Axis-aligned rectangle in pixel space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Tightly packed image buffer (row-major).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageBuffer {
    pub width: u32,
    pub height: u32,
    pub pixel_format: PixelFormat,
    pub data: Vec<u8>,
}

impl ImageBuffer {
    pub fn new(width: u32, height: u32, pixel_format: PixelFormat, data: Vec<u8>) -> Result<Self> {
        let expected = checked_image_len(width, height, pixel_format.bpp())?;
        if data.len() != expected {
            return Err(ScanError::Invalid(format!(
                "ImageBuffer size mismatch: got {}, expected {}",
                data.len(),
                expected
            )));
        }
        Ok(Self {
            width,
            height,
            pixel_format,
            data,
        })
    }

    pub fn bpp(&self) -> usize {
        self.pixel_format.bpp()
    }
}

pub(crate) fn image_to_luma8(image: &ImageBuffer) -> Result<(u32, u32, Vec<u8>)> {
    let width = image.width;
    let height = image.height;
    match image.pixel_format {
        PixelFormat::Gray8 => Ok((width, height, image.data.clone())),
        PixelFormat::Rgb8 | PixelFormat::Rgba8 => {
            let bytes_per_pixel = image.bpp();
            let mut gray = vec![0_u8; image.data.len() / bytes_per_pixel];
            for (index, output) in gray.iter_mut().enumerate() {
                let base = index * bytes_per_pixel;
                let red = image.data[base] as u32;
                let green = image.data[base + 1] as u32;
                let blue = image.data[base + 2] as u32;
                *output = ((77 * red + 150 * green + 29 * blue) >> 8) as u8;
            }
            Ok((width, height, gray))
        }
    }
}

/// Scan / pipeline preference bundle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PipelinePrefs {
    pub rotate: Rotate,
    pub flip_h: bool,
    pub flip_v: bool,
    pub crop: Option<Rect>,
    pub brightness: i32,
    pub contrast: i32,
    pub desaturate: bool,
    pub levels_black: i32,
    pub levels_white: i32,
    pub levels_gamma: f64,
    pub auto_deskew: bool,
    pub deskew_angle: f64,
    /// Detect and apply a cardinal orientation after the normal pipeline stages.
    pub auto_orient: bool,
    /// Detect page bounds after the normal pipeline stages.
    pub auto_crop: bool,
    pub white_balance: bool,
    pub sharpen_amount: f64,
    pub invert: bool,
    pub auto_levels: bool,
    /// Saturation delta [-100..100]; 0 = identity
    pub saturation: f64,
    /// Hue shift in degrees; 0 = identity
    pub hue: f64,
    /// Tone curve control points `[x, y]` in 0..255; None or fewer than 2 points = identity
    pub curves: Option<Vec<[i32; 2]>>,
    /// Filter-tab: "light" | "medium" | "heavy" | None/off
    pub infrared_clean: Option<String>,
    pub descreen: bool,
    pub descreen_dpi: i32,
    pub restore_colors: bool,
    pub restore_fading: bool,
    pub grain_reduction: Option<String>,
    pub flatten: bool,
    pub hole_punch: bool,
    pub colorize_mode: Option<String>,
    /// Film profile id for OSL-FILM convert
    pub film_type: Option<String>,
}

impl Default for PipelinePrefs {
    fn default() -> Self {
        Self {
            rotate: Rotate::None,
            flip_h: false,
            flip_v: false,
            crop: None,
            brightness: 0,
            contrast: 0,
            desaturate: false,
            levels_black: 0,
            levels_white: 255,
            levels_gamma: 1.0,
            auto_deskew: false,
            deskew_angle: 0.0,
            auto_orient: false,
            auto_crop: false,
            white_balance: false,
            sharpen_amount: 0.0,
            invert: false,
            auto_levels: false,
            saturation: 0.0,
            hue: 0.0,
            curves: None,
            infrared_clean: None,
            descreen: false,
            descreen_dpi: 75,
            restore_colors: false,
            restore_fading: false,
            grain_reduction: None,
            flatten: false,
            hole_punch: false,
            colorize_mode: None,
            film_type: None,
        }
    }
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
    /// Request both sides when `mode` selects a document feeder.
    /// Backends that cannot provide duplex acquisition return an explicit
    /// unsupported error instead of silently scanning only the front side.
    pub duplex: bool,
    pub dpi_x: u32,
    pub dpi_y: u32,
    pub width: u32,
    pub height: u32,
    pub pixel_format: PixelFormat,
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

/// Unified application error type.
#[derive(Debug, Error)]
pub enum ScanError {
    #[error("device not found: {0}")]
    DeviceNotFound(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("cancelled: {0}")]
    Cancelled(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("image: {0}")]
    Image(String),
    #[error("{0}")]
    Other(String),
}

impl From<image::ImageError> for ScanError {
    fn from(e: image::ImageError) -> Self {
        ScanError::Image(e.to_string())
    }
}

impl From<serde_json::Error> for ScanError {
    fn from(e: serde_json::Error) -> Self {
        ScanError::Other(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ScanError>;
