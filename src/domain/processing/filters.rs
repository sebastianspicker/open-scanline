//! Filter pipeline ops: sharpen, IR clean, descreen, grain, restore, flatten.

use crate::domain::image::{ImageBuffer, PixelFormat};
use crate::error::{Result, ScanError};

mod color;
mod fading;
mod infrared;

fn clamp_f(v: f64) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

/// Accepted tiers for `infrared_clean` and `grain_reduction`.
pub const FILTER_TIERS: &[&str] = &["light", "medium", "heavy"];
/// Accepted tint modes for `colorize`.
/// `warm` and `cool` are the GUI names for `sepia` and `blue`.
pub const COLORIZE_MODES: &[&str] = &["sepia", "blue", "warm", "cool"];

/// True for the shared "feature off" spellings: empty, `off`, `none`, `false`
/// (trimmed, case-insensitive).
pub fn is_disabled(value: &str) -> bool {
    let value = value.trim();
    value.is_empty()
        || ["off", "none", "false"]
            .iter()
            .any(|off| value.eq_ignore_ascii_case(off))
}

/// Accept a disabled spelling or one of `allowed` (case-insensitive).
pub fn validate_disableable_choice(label: &str, value: &str, allowed: &[&str]) -> Result<()> {
    let value = value.trim();
    if is_disabled(value) || allowed.iter().any(|a| value.eq_ignore_ascii_case(a)) {
        return Ok(());
    }
    Err(ScanError::Invalid(format!(
        "{label} must be off or one of: {}",
        allowed.join(", ")
    )))
}

/// Map a tier spelling to its canonical tier; `Ok(None)` means disabled.
/// The short and boolean aliases are kept for configs saved by older builds.
fn norm_amount(label: &str, amount: Option<&str>) -> Result<Option<&'static str>> {
    let Some(value) = amount.map(str::trim) else {
        return Ok(None);
    };
    if is_disabled(value) || matches!(value, "0") || value.eq_ignore_ascii_case("no") {
        return Ok(None);
    }
    let tier = match value.to_ascii_lowercase().as_str() {
        "light" | "l" | "low" => "light",
        "medium" | "true" | "on" | "yes" | "1" => "medium",
        "heavy" | "h" | "high" | "strong" => "heavy",
        _ => {
            return Err(ScanError::Invalid(format!(
                "{label} must be off or one of: {}",
                FILTER_TIERS.join(", ")
            )))
        }
    };
    Ok(Some(tier))
}

/// Accept a disabled spelling, a tier, or one of its legacy aliases.
pub fn validate_filter_tier(label: &str, value: &str) -> Result<()> {
    norm_amount(label, Some(value)).map(|_| ())
}

/// Sharpen amounts are accepted from 0 to 5 (not silently clamped or ignored).
pub fn validate_sharpen(amount: f64) -> Result<f64> {
    if amount.is_finite() && (0.0..=5.0).contains(&amount) {
        Ok(amount)
    } else {
        Err(ScanError::Invalid(
            "sharpen amount must be a finite number from 0 to 5".into(),
        ))
    }
}

/// Descreen print-screen dpi range that `descreen` actually uses.
pub fn validate_descreen_dpi(dpi: i32) -> Result<i32> {
    if (25..=300).contains(&dpi) {
        Ok(dpi)
    } else {
        Err(ScanError::Invalid(
            "descreen dpi must be from 25 to 300".into(),
        ))
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

/// Median filter over radius (box neighborhood, edges replicated).
///
/// Slides a 256-bin histogram along each row, so the cost per sample no
/// longer depends on sorting the whole (2r+1)^2 window.
pub fn median_filter(image: &ImageBuffer, radius: i32) -> Result<ImageBuffer> {
    let r = radius.max(1);
    let bpp = image.bpp();
    let w = image.width as i32;
    let h = image.height as i32;
    let src = &image.data;
    let sample = |x: i32, y: i32, ch: usize| -> usize {
        let xx = x.clamp(0, w - 1) as usize;
        let yy = y.clamp(0, h - 1) as usize;
        src[(yy * w as usize + xx) * bpp + ch] as usize
    };
    // Window holds n samples; the median is the sample at sorted index n / 2.
    let target = ((2 * r + 1) * (2 * r + 1)) as u32 / 2;
    let mut out = vec![0u8; src.len()];
    let mut hist = [0u32; 256];
    for y in 0..h {
        for ch in 0..bpp {
            hist.fill(0);
            for dy in -r..=r {
                for dx in -r..=r {
                    hist[sample(dx, y + dy, ch)] += 1;
                }
            }
            for x in 0..w {
                if x > 0 {
                    for dy in -r..=r {
                        hist[sample(x - 1 - r, y + dy, ch)] -= 1;
                        hist[sample(x + r, y + dy, ch)] += 1;
                    }
                }
                let mut seen = 0;
                let median = hist.iter().position(|&count| {
                    seen += count;
                    seen > target
                });
                out[(y as usize * w as usize + x as usize) * bpp + ch] = median.unwrap_or(0) as u8;
            }
        }
    }
    ImageBuffer::new(image.width, image.height, image.pixel_format, out)
}

/// Separable box blur.
pub fn box_blur(image: &ImageBuffer, radius: i32) -> Result<ImageBuffer> {
    if image.width == 0 || image.height == 0 {
        return ImageBuffer::new(image.width, image.height, image.pixel_format, Vec::new());
    }
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
    fn pass(&self, source: &[u8], output: &mut [u8], horizontal: bool) {
        if horizontal {
            self.pass_horizontal(source, output);
        } else {
            self.pass_vertical(source, output);
        }
    }

    fn pass_horizontal(&self, source: &[u8], output: &mut [u8]) {
        for row in 0..self.height {
            for channel in 0..self.bpp {
                let start = row * self.width * self.bpp + channel;
                self.blur_line(source, output, start, self.bpp, self.width);
            }
        }
    }

    fn pass_vertical(&self, source: &[u8], output: &mut [u8]) {
        let row_bytes = self.width * self.bpp;
        let initial_end = self.radius.min(self.height - 1);
        let mut sums = vec![0_u32; row_bytes];
        for row in 0..=initial_end {
            add_row(&mut sums, image_row(source, row, row_bytes));
        }
        for row in 0..self.height {
            let lo = row.saturating_sub(self.radius);
            let hi = (row + self.radius).min(self.height - 1);
            write_average_row(image_row_mut(output, row, row_bytes), &sums, hi - lo + 1);
            if row >= self.radius {
                subtract_row(&mut sums, image_row(source, row - self.radius, row_bytes));
            }
            if row + self.radius + 1 < self.height {
                add_row(
                    &mut sums,
                    image_row(source, row + self.radius + 1, row_bytes),
                );
            }
        }
    }

    fn blur_line(&self, source: &[u8], output: &mut [u8], start: usize, stride: usize, len: usize) {
        let initial_end = self.radius.min(len - 1);
        let mut sum = (0..=initial_end)
            .map(|position| source[start + position * stride] as u32)
            .sum::<u32>();
        for pos in 0..len {
            let lo = pos.saturating_sub(self.radius);
            let hi = (pos + self.radius).min(len - 1);
            output[start + pos * stride] = (sum / (hi - lo + 1) as u32) as u8;
            if pos >= self.radius {
                sum -= source[start + (pos - self.radius) * stride] as u32;
            }
            if pos + self.radius + 1 < len {
                sum += source[start + (pos + self.radius + 1) * stride] as u32;
            }
        }
    }
}

fn image_row(data: &[u8], row: usize, row_bytes: usize) -> &[u8] {
    &data[row * row_bytes..(row + 1) * row_bytes]
}

fn image_row_mut(data: &mut [u8], row: usize, row_bytes: usize) -> &mut [u8] {
    &mut data[row * row_bytes..(row + 1) * row_bytes]
}

fn add_row(sums: &mut [u32], row: &[u8]) {
    for (sum, &value) in sums.iter_mut().zip(row) {
        *sum += value as u32;
    }
}

fn subtract_row(sums: &mut [u32], row: &[u8]) {
    for (sum, &value) in sums.iter_mut().zip(row) {
        *sum -= value as u32;
    }
}

fn write_average_row(output: &mut [u8], sums: &[u32], count: usize) {
    for (value, &sum) in output.iter_mut().zip(sums) {
        *value = (sum / count as u32) as u8;
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
    let tier = match norm_amount("infrared clean tier", amount)? {
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
    let tier = match norm_amount("grain reduction tier", amount)? {
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
    let luma = if bpp >= 3 {
        u32::from(crate::domain::image::luma(
            out[index],
            out[index + 1],
            out[index + 2],
        ))
    } else {
        u32::from(out[index])
    };
    if luma < 40 {
        out[index..index + bpp.min(3)].fill(245);
    }
}

/// Colorize grayscale / push monochrome toward a tint mode.
pub fn colorize(image: &ImageBuffer, mode: Option<&str>) -> Result<ImageBuffer> {
    let mode = mode.unwrap_or("sepia").trim().to_ascii_lowercase();
    if is_disabled(&mode) {
        return Ok(image.clone());
    }
    validate_disableable_choice("colorize mode", &mode, COLORIZE_MODES)?;
    let tint = if mode == "cool" {
        "blue"
    } else {
        mode.as_str()
    };
    ImageBuffer::new(
        image.width,
        image.height,
        PixelFormat::Rgb8,
        color::colorized(image, tint),
    )
}

#[cfg(test)]
mod mode_validation_tests {
    use super::*;

    fn gray() -> ImageBuffer {
        ImageBuffer::new(2, 2, PixelFormat::Gray8, vec![10, 90, 170, 250]).unwrap()
    }

    #[test]
    fn disabled_spellings_are_shared() {
        for off in ["", " ", "off", "None", "FALSE"] {
            assert!(is_disabled(off), "{off:?}");
        }
        assert!(!is_disabled("light"));
    }

    #[test]
    fn unknown_tiers_and_modes_are_rejected_not_defaulted() {
        assert!(grain_reduction(&gray(), Some("hevy")).is_err());
        assert!(infrared_clean(&gray(), Some("heavyy")).is_err());
        assert!(colorize(&gray(), Some("red")).is_err());
        for ok in ["light", "Medium", "heavy", "off", "false"] {
            assert!(grain_reduction(&gray(), Some(ok)).is_ok(), "{ok}");
        }
        for ok in ["sepia", "BLUE", "none", "warm", "cool"] {
            assert!(colorize(&gray(), Some(ok)).is_ok(), "{ok}");
        }
    }

    #[test]
    fn legacy_tier_aliases_and_gui_tints_keep_working() {
        let tier = |value| norm_amount("tier", Some(value)).unwrap();
        assert_eq!(tier("low"), Some("light"));
        assert_eq!(tier("true"), Some("medium"));
        assert_eq!(tier("strong"), Some("heavy"));
        assert_eq!(tier("no"), None);
        assert_eq!(tier("0"), None);
        // The GUI offers warm/cool; cool used to fall back to sepia.
        assert_eq!(
            colorize(&gray(), Some("cool")).unwrap().data,
            colorize(&gray(), Some("blue")).unwrap().data
        );
        assert_eq!(
            colorize(&gray(), Some("warm")).unwrap().data,
            colorize(&gray(), Some("sepia")).unwrap().data
        );
    }
}

#[cfg(test)]
mod median_and_hole_punch_tests {
    use super::*;
    use crate::domain::image::PixelFormat;

    /// Median filter over radius (box neighborhood).
    fn reference_median(image: &ImageBuffer, radius: i32) -> Result<ImageBuffer> {
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
            let middle = vals.len() / 2;
            if vals.len() >= 49 {
                *vals.select_nth_unstable(middle).1
            } else {
                vals.sort_unstable();
                vals[middle]
            }
        }
    }

    fn pseudo_random_image(width: u32, height: u32, format: PixelFormat) -> ImageBuffer {
        let len = width as usize * height as usize * format.bpp();
        let mut state = 12345u32;
        let data = (0..len)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 24) as u8
            })
            .collect();
        ImageBuffer::new(width, height, format, data).unwrap()
    }

    #[test]
    fn histogram_median_matches_the_sorting_median() {
        for format in [PixelFormat::Rgb8, PixelFormat::Gray8] {
            for (w, h) in [(13, 11), (2, 3), (1, 5)] {
                let image = pseudo_random_image(w, h, format);
                for r in 1..=3 {
                    assert_eq!(
                        median_filter(&image, r).unwrap().data,
                        reference_median(&image, r).unwrap().data,
                        "{w}x{h} {format:?} r={r}"
                    );
                }
            }
        }
    }

    #[test]
    fn hole_punch_only_whitens_dark_pixels() {
        let (w, h) = (50u32, 50u32);
        let mut data = vec![128u8; (w * h * 3) as usize];
        data[..3].copy_from_slice(&[200, 10, 10]);
        data[3..6].copy_from_slice(&[10, 10, 10]);
        let image = ImageBuffer::new(w, h, PixelFormat::Rgb8, data).unwrap();
        let out = hole_punch_removal(&image).unwrap();
        assert_eq!(out.data[..3], [200, 10, 10]);
        assert_eq!(out.data[3..6], [245, 245, 245]);
    }
}
