//! Pure image values and invariants.

use crate::error::{Result, ScanError};
use serde::{Deserialize, Serialize};
use std::num::ParseIntError;

/// Product safety ceiling for one decoded, tightly packed image buffer.
pub const MAX_IMAGE_BYTES: usize = 512 * 1024 * 1024;
/// Per-axis ceiling shared by input decoding and generated image buffers.
pub const MAX_IMAGE_DIMENSION: u32 = 65_535;

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
            Self::Gray8 => 1,
            Self::Rgb8 => 3,
            Self::Rgba8 => 4,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gray8 => "Gray8",
            Self::Rgb8 => "Rgb8",
            Self::Rgba8 => "Rgba8",
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
    pub fn from_degrees(degrees: i32) -> Self {
        match degrees.rem_euclid(360) {
            90 => Self::R90,
            180 => Self::R180,
            270 => Self::R270,
            _ => Self::None,
        }
    }

    pub fn degrees(self) -> i32 {
        self as i32
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
