//! Borrowed BMP encoding with a larger sequential output buffer.
use super::image_color_type;
use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use std::io::{BufWriter, Write};
use std::path::Path;

pub(super) fn write_bmp(image: &ImageBuffer, output: &Path) -> Result<()> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(output)?;
    let mut writer = BufWriter::with_capacity(64 * 1024, file);
    image::codecs::bmp::BmpEncoder::new(&mut writer)
        .encode(
            &image.data,
            image.width,
            image.height,
            image_color_type(image.pixel_format).into(),
        )
        .map_err(|error| ScanError::Image(error.to_string()))?;
    writer.flush()?;
    Ok(())
}
