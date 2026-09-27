use crate::domain::image::{ImageBuffer, PixelFormat};
use crate::error::{Result, ScanError};

/// Pack ImageBuffer as tightly interleaved RGBA8 for GUI preview textures.
pub fn image_buffer_to_rgba(image: &ImageBuffer) -> Result<(u32, u32, Vec<u8>)> {
    super::codecs::validate_packed_buffer(image)?;
    let w = image.width;
    let h = image.height;
    let n = (w as usize) * (h as usize);
    let mut rgba = vec![0u8; n * 4];
    match image.pixel_format {
        PixelFormat::Gray8 => {
            for i in 0..n {
                let y = image.data[i];
                let o = i * 4;
                rgba[o] = y;
                rgba[o + 1] = y;
                rgba[o + 2] = y;
                rgba[o + 3] = 255;
            }
        }
        PixelFormat::Rgb8 => {
            for i in 0..n {
                let s = i * 3;
                let o = i * 4;
                rgba[o] = image.data[s];
                rgba[o + 1] = image.data[s + 1];
                rgba[o + 2] = image.data[s + 2];
                rgba[o + 3] = 255;
            }
        }
        PixelFormat::Rgba8 => {
            if image.data.len() == n * 4 {
                rgba.copy_from_slice(&image.data);
            } else {
                return Err(ScanError::Invalid("rgba buffer size".into()));
            }
        }
    }
    Ok((w, h, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_short_gray8_buffer_instead_of_panicking() {
        let image = ImageBuffer {
            width: 4,
            height: 4,
            pixel_format: PixelFormat::Gray8,
            data: vec![0u8; 2],
        };
        assert!(image_buffer_to_rgba(&image).is_err());
    }

    #[test]
    fn rejects_short_rgb8_buffer_instead_of_panicking() {
        let image = ImageBuffer {
            width: 4,
            height: 4,
            pixel_format: PixelFormat::Rgb8,
            data: vec![0u8; 3],
        };
        assert!(image_buffer_to_rgba(&image).is_err());
    }

    #[test]
    fn rejects_dimensions_that_overflow_u32_multiplication() {
        let image = ImageBuffer {
            width: u32::MAX,
            height: 2,
            pixel_format: PixelFormat::Gray8,
            data: vec![0u8; 8],
        };
        assert!(image_buffer_to_rgba(&image).is_err());
    }

    #[test]
    fn packs_a_valid_rgb8_buffer() {
        let image =
            ImageBuffer::new(2, 1, PixelFormat::Rgb8, vec![10, 20, 30, 40, 50, 60]).unwrap();
        let (w, h, rgba) = image_buffer_to_rgba(&image).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(rgba, vec![10, 20, 30, 255, 40, 50, 60, 255]);
    }
}
