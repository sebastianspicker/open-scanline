//! PNG encoding through the `png` crate so the `pHYs` density chunk can be written.
use super::validate_packed_buffer;
use crate::domain::image::{ImageBuffer, PixelFormat};
use crate::error::{Result, ScanError};
use std::io::{BufWriter, Write};
use std::path::Path;

pub(super) fn write_png(image: &ImageBuffer, output: &Path, dpi: Option<u32>) -> Result<()> {
    validate_packed_buffer(image)?;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(output)?;
    let mut writer = BufWriter::with_capacity(64 * 1024, file);
    let mut encoder = ::png::Encoder::new(&mut writer, image.width, image.height);
    encoder.set_color(match image.pixel_format {
        PixelFormat::Gray8 => ::png::ColorType::Grayscale,
        PixelFormat::Rgb8 => ::png::ColorType::Rgb,
        PixelFormat::Rgba8 => ::png::ColorType::Rgba,
    });
    encoder.set_depth(::png::BitDepth::Eight);
    if let Some(dpi) = dpi.filter(|&dpi| dpi > 0) {
        // pHYs counts pixels per metre; 1 inch = 0.0254 m.
        let ppm = (f64::from(dpi) / 0.0254).round() as u32;
        encoder.set_pixel_dims(Some(::png::PixelDimensions {
            xppu: ppm,
            yppu: ppm,
            unit: ::png::Unit::Meter,
        }));
    }
    let len = image.width as usize * image.height as usize * image.bpp();
    let mut png = encoder
        .write_header()
        .map_err(|error| ScanError::Image(error.to_string()))?;
    png.write_image_data(&image.data[..len])
        .map_err(|error| ScanError::Image(error.to_string()))?;
    png.finish()
        .map_err(|error| ScanError::Image(error.to_string()))?;
    writer.flush()?;
    Ok(())
}
