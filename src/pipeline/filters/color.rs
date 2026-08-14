use crate::core::{ImageBuffer, PixelFormat};

use super::clamp_f;

pub(super) fn restored(data: &[u8], bpp: usize) -> Vec<u8> {
    let luts = restore_luts();
    let mut output = data.to_vec();
    for pixel in output.chunks_exact_mut(bpp) {
        let (r, g, b) = (pixel[0] as usize, pixel[1] as usize, pixel[2] as usize);
        let mut green = luts[1][g];
        let mut blue = luts[2][b];
        if g >= r && g >= b && g > 40 {
            green = clamp_f(green as f64 * 1.03);
        }
        if b >= r && b > g && b > 40 {
            blue = clamp_f(blue as f64 * 1.03);
        }
        pixel[0] = luts[0][r];
        pixel[1] = green;
        pixel[2] = blue;
    }
    output
}

fn restore_luts() -> [[u8; 256]; 3] {
    let mut luts = [[0u8; 256]; 3];
    let [red_lut, green_lut, blue_lut] = &mut luts;
    for (value, ((red, green), blue)) in red_lut
        .iter_mut()
        .zip(green_lut.iter_mut())
        .zip(blue_lut.iter_mut())
        .enumerate()
    {
        let t = value as f64 / 255.0;
        *red = clamp_f(t.powf(1.05) * 255.0 * 0.98);
        *green = clamp_f(t.powf(0.92) * 255.0 * 1.06);
        *blue = clamp_f(t.powf(0.94) * 255.0 * 1.04);
    }
    luts
}

pub(super) fn colorized(image: &ImageBuffer, mode: &str) -> Vec<u8> {
    let mut rgb = rgb_workspace(image);
    for pixel in rgb.chunks_exact_mut(3) {
        tint(pixel, mode);
    }
    rgb
}

fn rgb_workspace(image: &ImageBuffer) -> Vec<u8> {
    match image.pixel_format {
        PixelFormat::Rgb8 => image.data.clone(),
        PixelFormat::Gray8 => image.data.iter().flat_map(|value| [*value; 3]).collect(),
        PixelFormat::Rgba8 => image
            .data
            .chunks_exact(4)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
            .collect(),
    }
}

fn tint(pixel: &mut [u8], mode: &str) {
    let luma = ((77u32 * pixel[0] as u32 + 150 * pixel[1] as u32 + 29 * pixel[2] as u32) >> 8)
        as f64
        / 255.0;
    let factors = if mode.contains("blue") {
        [0.7, 0.85, 1.15]
    } else {
        [1.15, 1.0, 0.75]
    };
    for (channel, factor) in pixel.iter_mut().zip(factors) {
        *channel = clamp_f(luma * factor * 255.0);
    }
}
