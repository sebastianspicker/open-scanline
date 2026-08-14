//! Filter pipeline ops: sharpen, IR clean, descreen, grain, restore, flatten.

use crate::core::{ImageBuffer, PixelFormat, Result, ScanError};

mod color;
mod fading;
mod infrared;

fn clamp_f(v: f64) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

fn norm_amount(amount: Option<&str>, default: &str) -> Option<&'static str> {
    let value = amount?.trim().to_ascii_lowercase();
    (!matches!(value.as_str(), "" | "off" | "none" | "false" | "0" | "no")).then(|| {
        match value.as_str() {
            "light" | "l" | "low" => "light",
            "heavy" | "h" | "high" | "strong" => "heavy",
            "true" | "on" | "yes" | "1" => bool_tier(default),
            _ => "medium",
        }
    })
}

fn bool_tier(default: &str) -> &'static str {
    match default {
        "light" => "light",
        "heavy" => "heavy",
        _ => "medium",
    }
}

/// Unsharp-style 3×3 kernel sharpen. `amount` 0 = identity.
pub fn sharpen(image: &ImageBuffer, amount: f64) -> Result<ImageBuffer> {
    if amount.abs() < 1e-9 {
        return Ok(image.clone());
    }
    if image.width < 3 || image.height < 3 {
        return Ok(image.clone());
    }
    let a = amount.clamp(0.0, 5.0);
    let bpp = image.bpp();
    let w = image.width as i32;
    let h = image.height as i32;
    let src = &image.data;
    let mut out = vec![0u8; src.len()];
    let c = 1.0 + a;
    let n = -a / 4.0;

    let sample = |xx: i32, yy: i32, c_i: usize| -> f64 {
        let xx = xx.clamp(0, w - 1) as usize;
        let yy = yy.clamp(0, h - 1) as usize;
        src[(yy * w as usize + xx) * bpp + c_i] as f64
    };

    for y in 0..h {
        for x in 0..w {
            for ch in 0..bpp {
                let v = c * sample(x, y, ch)
                    + n * (sample(x - 1, y, ch)
                        + sample(x + 1, y, ch)
                        + sample(x, y - 1, ch)
                        + sample(x, y + 1, ch));
                let idx = (y as usize * w as usize + x as usize) * bpp + ch;
                out[idx] = clamp_f(v);
            }
        }
    }
    ImageBuffer::new(image.width, image.height, image.pixel_format, out)
}

/// Median filter over radius (box neighborhood).
pub fn median_filter(image: &ImageBuffer, radius: i32) -> Result<ImageBuffer> {
    let r = radius.max(1);
    let bpp = image.bpp();
    let w = image.width as i32;
    let h = image.height as i32;
    let src = &image.data;
    let context = MedianContext {
        data: src,
        width: w,
        height: h,
        bpp,
        radius: r,
    };
    let mut out = vec![0u8; src.len()];
    let mut vals = Vec::with_capacity(((2 * r + 1) * (2 * r + 1)) as usize);

    for y in 0..h {
        for x in 0..w {
            for ch in 0..bpp {
                out[(y as usize * w as usize + x as usize) * bpp + ch] =
                    context.at(x, y, ch, &mut vals);
            }
        }
    }
    ImageBuffer::new(image.width, image.height, image.pixel_format, out)
}

struct MedianContext<'a> {
    data: &'a [u8],
    width: i32,
    height: i32,
    bpp: usize,
    radius: i32,
}

impl MedianContext<'_> {
    fn at(&self, x: i32, y: i32, channel: usize, vals: &mut Vec<u8>) -> u8 {
        vals.clear();
        for dy in -self.radius..=self.radius {
            for dx in -self.radius..=self.radius {
                let xx = (x + dx).clamp(0, self.width - 1) as usize;
                let yy = (y + dy).clamp(0, self.height - 1) as usize;
                vals.push(self.data[(yy * self.width as usize + xx) * self.bpp + channel]);
            }
        }
        vals.sort_unstable();
        vals[vals.len() / 2]
    }
}

/// Separable box blur.
pub fn box_blur(image: &ImageBuffer, radius: i32) -> Result<ImageBuffer> {
    let r = radius.max(1) as usize;
    let bpp = image.bpp();
    let w = image.width as usize;
    let h = image.height as usize;
    let context = BlurContext {
        width: w,
        height: h,
        bpp,
        radius: r,
    };
    let mut tmp = vec![0u8; image.data.len()];
    let mut out = vec![0u8; image.data.len()];
    context.pass(&image.data, &mut tmp, true);
    context.pass(&tmp, &mut out, false);
    ImageBuffer::new(image.width, image.height, image.pixel_format, out)
}

struct BlurContext {
    width: usize,
    height: usize,
    bpp: usize,
    radius: usize,
}

impl BlurContext {
    fn pass(&self, src: &[u8], out: &mut [u8], horizontal: bool) {
        let (line_count, line_len) = if horizontal {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        };
        for line in 0..line_count {
            for channel in 0..self.bpp {
                let mut prefix = vec![0u32; line_len + 1];
                for pos in 0..line_len {
                    prefix[pos + 1] =
                        prefix[pos] + src[self.index(line, pos, channel, horizontal)] as u32;
                }
                for pos in 0..line_len {
                    let lo = pos.saturating_sub(self.radius);
                    let hi = (pos + self.radius).min(line_len - 1);
                    let value = (prefix[hi + 1] - prefix[lo]) / (hi - lo + 1) as u32;
                    out[self.index(line, pos, channel, horizontal)] = value as u8;
                }
            }
        }
    }

    fn index(&self, line: usize, pos: usize, channel: usize, horizontal: bool) -> usize {
        let pixel = if horizontal {
            line * self.width + pos
        } else {
            pos * self.width + line
        };
        pixel * self.bpp + channel
    }
}

/// Denoise by blending with median.
pub fn denoise(image: &ImageBuffer, strength: f64) -> Result<ImageBuffer> {
    let a = strength.clamp(0.0, 1.0);
    if a < 1e-9 {
        return Ok(image.clone());
    }
    let med = median_filter(image, 1)?;
    let mut out = image.data.clone();
    for (out_item, &median) in out.iter_mut().zip(&med.data) {
        *out_item = clamp_f(*out_item as f64 * (1.0 - a) + median as f64 * a);
    }
    ImageBuffer::new(image.width, image.height, image.pixel_format, out)
}

/// Remove dust/scratches via IR-style residual median inpaint.
pub fn infrared_clean(image: &ImageBuffer, amount: Option<&str>) -> Result<ImageBuffer> {
    let tier = match norm_amount(amount, "medium") {
        None => return Ok(image.clone()),
        Some(t) => t,
    };
    infrared::clean(image, tier)
}

/// Mild low-pass tuned by print-screen dpi to reduce moiré.
pub fn descreen(image: &ImageBuffer, dpi: i32) -> Result<ImageBuffer> {
    if !matches!(
        image.pixel_format,
        PixelFormat::Gray8 | PixelFormat::Rgb8 | PixelFormat::Rgba8
    ) {
        return Err(ScanError::Unsupported(format!(
            "descreen: {:?}",
            image.pixel_format
        )));
    }
    let d = dpi.clamp(25, 300);
    let radius = (150.0 / d as f64).round().clamp(1.0, 4.0) as i32;
    box_blur(image, radius)
}

/// Grain reduction via denoise/median blend.
pub fn grain_reduction(image: &ImageBuffer, amount: Option<&str>) -> Result<ImageBuffer> {
    let tier = match norm_amount(amount, "medium") {
        None => return Ok(image.clone()),
        Some(t) => t,
    };
    match tier {
        "light" => denoise(image, 0.35),
        "heavy" => {
            let med = median_filter(image, 2)?;
            let a = 0.9;
            let mut out = image.data.clone();
            for (out_item, &median) in out.iter_mut().zip(&med.data) {
                *out_item = clamp_f(*out_item as f64 * (1.0 - a) + median as f64 * a);
            }
            ImageBuffer::new(image.width, image.height, image.pixel_format, out)
        }
        _ => denoise(image, 0.7),
    }
}

/// Memory-color restore: modest G/B boost.
pub fn restore_colors(image: &ImageBuffer) -> Result<ImageBuffer> {
    if image.pixel_format == PixelFormat::Gray8 {
        return Ok(image.clone());
    }
    if image.pixel_format != PixelFormat::Rgb8 && image.pixel_format != PixelFormat::Rgba8 {
        return Err(ScanError::Unsupported(format!(
            "restore_colors: {:?}",
            image.pixel_format
        )));
    }
    ImageBuffer::new(
        image.width,
        image.height,
        image.pixel_format,
        color::restored(&image.data, image.bpp()),
    )
}

/// Correct dye fade via per-channel stretch + soft gray-world.
pub fn restore_fading(image: &ImageBuffer) -> Result<ImageBuffer> {
    if image.pixel_format == PixelFormat::Gray8 {
        return Ok(image.clone());
    }
    if image.pixel_format != PixelFormat::Rgb8 && image.pixel_format != PixelFormat::Rgba8 {
        return Err(ScanError::Unsupported(format!(
            "restore_fading: {:?}",
            image.pixel_format
        )));
    }
    let n = (image.width * image.height) as usize;
    if n < 1 {
        return Ok(image.clone());
    }
    let bpp = image.bpp();
    let out = fading::restore_rgb(&image.data, bpp, n);
    ImageBuffer::new(image.width, image.height, image.pixel_format, out)
}

/// Flatten page: lift dark background toward white.
pub fn flatten(image: &ImageBuffer) -> Result<ImageBuffer> {
    let bpp = image.bpp();
    let mut out = image.data.clone();
    for i in (0..out.len()).step_by(bpp) {
        for c in 0..bpp.min(3) {
            let v = out[i + c] as f64;
            // Soft lift of midtones toward paper white
            let t = v / 255.0;
            let lifted = t + (1.0 - t) * 0.12 * t;
            out[i + c] = clamp_f(lifted * 255.0);
        }
    }
    ImageBuffer::new(image.width, image.height, image.pixel_format, out)
}

/// Hole-punch removal: brighten dark circular-ish corners (heuristic).
pub fn hole_punch_removal(image: &ImageBuffer) -> Result<ImageBuffer> {
    let w = image.width as i32;
    let h = image.height as i32;
    if w == 0 || h == 0 {
        return Ok(image.clone());
    }
    let bpp = image.bpp();
    let mut out = image.data.clone();
    let r = (w.min(h) as f64 * 0.04).max(4.0) as i32;
    let corners = [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)];
    for (cx, cy) in corners {
        brighten_corner(&mut out, w, h, bpp, cx, cy, r);
    }
    ImageBuffer::new(image.width, image.height, image.pixel_format, out)
}

fn brighten_corner(out: &mut [u8], width: i32, height: i32, bpp: usize, cx: i32, cy: i32, r: i32) {
    for y in (cy - r).max(0)..=(cy + r).min(height - 1) {
        for x in (cx - r).max(0)..=(cx + r).min(width - 1) {
            let dx = x - cx;
            let dy = y - cy;
            if dx * dx + dy * dy <= r * r {
                brighten_pixel(out, (y as usize * width as usize + x as usize) * bpp, bpp);
            }
        }
    }
}

fn brighten_pixel(out: &mut [u8], index: usize, bpp: usize) {
    for channel in 0..bpp.min(3) {
        if out[index + channel] < 40 {
            out[index + channel] = 245;
        }
    }
}

/// Colorize grayscale / push monochrome toward a tint mode.
pub fn colorize(image: &ImageBuffer, mode: Option<&str>) -> Result<ImageBuffer> {
    let mode = mode.unwrap_or("sepia").to_ascii_lowercase();
    if mode == "off" || mode == "none" {
        return Ok(image.clone());
    }
    ImageBuffer::new(
        image.width,
        image.height,
        PixelFormat::Rgb8,
        color::colorized(image, &mode),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(format: PixelFormat, width: u32, height: u32, data: Vec<u8>) -> ImageBuffer {
        ImageBuffer::new(width, height, format, data).unwrap()
    }

    #[test]
    fn median_and_blur_keep_supported_layouts_and_rgba_alpha() {
        let blur_input = image(
            PixelFormat::Rgb8,
            3,
            3,
            (0u8..9).flat_map(|value| [value; 3]).collect(),
        );
        assert_eq!(
            box_blur(&blur_input, 1).unwrap().data,
            vec![1, 1, 1, 2, 2, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5]
        );
        let inputs = [
            image(PixelFormat::Gray8, 3, 3, (0..9).collect()),
            image(PixelFormat::Rgb8, 3, 3, (0..27).collect()),
            image(PixelFormat::Rgba8, 3, 3, (0..36).collect()),
        ];
        for input in &inputs {
            for output in [
                median_filter(input, 1).unwrap(),
                box_blur(input, 1).unwrap(),
            ] {
                assert_eq!(
                    (output.width, output.height, output.pixel_format),
                    (input.width, input.height, input.pixel_format)
                );
            }
        }
        let rgba = &inputs[2];
        let alpha: Vec<_> = rgba.data.chunks_exact(4).map(|pixel| pixel[3]).collect();
        for output in [
            restore_colors(rgba).unwrap(),
            restore_fading(rgba).unwrap(),
            flatten(rgba).unwrap(),
            hole_punch_removal(rgba).unwrap(),
        ] {
            assert_eq!(
                output
                    .data
                    .chunks_exact(4)
                    .map(|pixel| pixel[3])
                    .collect::<Vec<_>>(),
                alpha
            );
        }
    }

    #[test]
    fn off_tiers_and_aliases_are_stable() {
        let input = image(PixelFormat::Rgb8, 3, 3, (0..27).collect());
        assert_eq!(infrared_clean(&input, None).unwrap(), input);
        assert_eq!(infrared_clean(&input, Some("off")).unwrap(), input);
        assert_eq!(grain_reduction(&input, Some("none")).unwrap(), input);
        assert_eq!(colorize(&input, Some("off")).unwrap(), input);
        assert_eq!(sharpen(&input, 0.0).unwrap(), input);
        assert_eq!(
            infrared_clean(&input, Some("light")).unwrap(),
            infrared_clean(&input, Some("l")).unwrap()
        );
        assert_eq!(
            grain_reduction(&input, Some("heavy")).unwrap(),
            grain_reduction(&input, Some("strong")).unwrap()
        );
    }

    #[test]
    fn colorize_uses_all_visible_rgba_channels() {
        let rgb = image(PixelFormat::Rgb8, 2, 1, vec![0, 255, 0, 20, 40, 200]);
        let rgba = image(
            PixelFormat::Rgba8,
            2,
            1,
            vec![0, 255, 0, 17, 20, 40, 200, 231],
        );

        for mode in ["sepia", "blue"] {
            assert_eq!(
                colorize(&rgba, Some(mode)).unwrap(),
                colorize(&rgb, Some(mode)).unwrap(),
                "RGBA and RGB visible channels diverged in {mode} mode"
            );
        }
    }

    #[test]
    fn hole_punch_removal_changes_only_dark_corner_disks() {
        let mut data = vec![128; 20 * 20 * 4];
        for pixel in data.chunks_exact_mut(4) {
            pixel[3] = 77;
        }
        data[0..3].fill(10);
        data[(4 * 20 + 4) * 4..(4 * 20 + 4) * 4 + 3].fill(10);
        data[(10 * 20 + 10) * 4..(10 * 20 + 10) * 4 + 3].fill(10);
        let output = hole_punch_removal(&image(PixelFormat::Rgba8, 20, 20, data)).unwrap();
        assert_eq!(&output.data[0..4], &[245, 245, 245, 77]);
        assert_eq!(
            &output.data[(4 * 20 + 4) * 4..(4 * 20 + 4) * 4 + 4],
            &[10, 10, 10, 77]
        );
        assert_eq!(
            &output.data[(10 * 20 + 10) * 4..(10 * 20 + 10) * 4 + 4],
            &[10, 10, 10, 77]
        );
    }
}
