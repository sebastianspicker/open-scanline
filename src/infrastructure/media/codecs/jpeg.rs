//! JPEG encoding that keeps grey pages single-component and records the density.
use super::image_color_type;
use crate::domain::image::{ImageBuffer, PixelFormat};
use crate::error::{Result, ScanError};
use image::codecs::jpeg::{JpegEncoder, PixelDensity};
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;

pub(super) fn write_jpeg(
    image: &ImageBuffer,
    output: &Path,
    quality: Option<u8>,
    dpi: Option<u32>,
) -> Result<()> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(output)?;
    let mut writer = BufWriter::new(file);
    let mut encoder = JpegEncoder::new_with_quality(&mut writer, quality.unwrap_or(90));
    // JFIF stores density as u16; larger values are left out rather than truncated.
    if let Some(density) = dpi
        .and_then(|dpi| u16::try_from(dpi).ok())
        .filter(|&d| d > 0)
    {
        encoder.set_pixel_density(PixelDensity::dpi(density));
    }
    // The JPEG encoder takes only L8 and Rgb8; alpha is dropped, as before.
    let rgb;
    let (data, color) = if image.pixel_format == PixelFormat::Rgba8 {
        rgb = super::to_rgb_bytes(image)?;
        (rgb.as_slice(), image::ExtendedColorType::Rgb8)
    } else {
        let len = image.width as usize * image.height as usize * image.bpp();
        (
            &image.data[..len],
            image_color_type(image.pixel_format).into(),
        )
    };
    encoder
        .encode(data, image.width, image.height, color)
        .map_err(|error| ScanError::Image(error.to_string()))?;
    writer.flush()?;
    Ok(())
}
