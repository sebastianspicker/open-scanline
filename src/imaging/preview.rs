use crate::core::{ImageBuffer, PixelFormat, Result, ScanError};

/// Pack ImageBuffer as tightly interleaved RGBA8 for GUI preview textures.
pub fn image_buffer_to_rgba(image: &ImageBuffer) -> Result<(u32, u32, Vec<u8>)> {
    let w = image.width;
    let h = image.height;
    let n = (w * h) as usize;
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
