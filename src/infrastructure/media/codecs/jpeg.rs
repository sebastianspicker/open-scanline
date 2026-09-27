//! Borrowed view preserves DynamicImage's RGBA pixel contract for JPEG encoding.
use crate::domain::image::{ImageBuffer, PixelFormat};
use crate::error::{Result, ScanError};
use image::{GenericImageView, Rgba};
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;

struct RgbaView<'a>(&'a ImageBuffer);

impl GenericImageView for RgbaView<'_> {
    type Pixel = Rgba<u8>;

    fn dimensions(&self) -> (u32, u32) {
        (self.0.width, self.0.height)
    }

    fn get_pixel(&self, x: u32, y: u32) -> Self::Pixel {
        let image = self.0;
        let base = (y as usize * image.width as usize + x as usize) * image.bpp();
        let data = &image.data[base..];
        match image.pixel_format {
            PixelFormat::Gray8 => Rgba([data[0], data[0], data[0], 255]),
            PixelFormat::Rgb8 => Rgba([data[0], data[1], data[2], 255]),
            PixelFormat::Rgba8 => Rgba([data[0], data[1], data[2], data[3]]),
        }
    }
}

pub(super) fn write_jpeg(image: &ImageBuffer, output: &Path, quality: Option<u8>) -> Result<()> {
    let file = OpenOptions::new().write(true).truncate(true).open(output)?;
    let mut writer = BufWriter::new(file);
    let mut encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, quality.unwrap_or(90));
    encoder
        .encode_image(&RgbaView(image))
        .map_err(|error| ScanError::Image(error.to_string()))?;
    writer.flush()?;
    Ok(())
}
