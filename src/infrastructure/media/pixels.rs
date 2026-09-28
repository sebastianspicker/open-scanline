//! Owned packed-pixel conversions for streaming document writers.

use crate::domain::image::{ImageBuffer, PixelFormat};
use crate::error::{Result, ScanError};

pub(super) fn into_rgb_bytes(image: ImageBuffer) -> Result<Vec<u8>> {
    super::codecs::validate_packed_buffer(&image)?;
    if image.pixel_format == PixelFormat::Rgb8 {
        Ok(image.data)
    } else {
        super::to_rgb_bytes(&image)
    }
}

pub(super) fn into_rgb_image(image: ImageBuffer, label: &str) -> Result<image::RgbImage> {
    let (width, height) = (image.width, image.height);
    let data = if image.pixel_format == PixelFormat::Rgb8 {
        image.data
    } else {
        into_rgb_bytes(image)?
    };
    image::RgbImage::from_raw(width, height, data)
        .ok_or_else(|| ScanError::Image(format!("invalid {label} RGB image buffer")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gray_and_rgba_conversion_ignore_trailing_packed_bytes() {
        let gray = ImageBuffer {
            width: 1,
            height: 1,
            pixel_format: PixelFormat::Gray8,
            data: vec![17, 99],
        };
        assert_eq!(into_rgb_bytes(gray).unwrap(), vec![17, 17, 17]);

        let rgba = ImageBuffer {
            width: 1,
            height: 1,
            pixel_format: PixelFormat::Rgba8,
            data: vec![1, 2, 3, 4, 9, 9, 9, 9],
        };
        assert_eq!(into_rgb_bytes(rgba).unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn short_tiff_rgb_buffer_preserves_legacy_error() {
        let image = ImageBuffer {
            width: 2,
            height: 1,
            pixel_format: PixelFormat::Rgb8,
            data: vec![1, 2, 3],
        };
        let error = into_rgb_image(image, "TIFF").unwrap_err();
        assert!(
            matches!(error, ScanError::Image(message) if message == "invalid TIFF RGB image buffer")
        );
    }
}
