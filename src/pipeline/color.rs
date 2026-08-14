//! Color pipeline ops.

use crate::core::{ImageBuffer, PixelFormat, Result, ScanError};

fn clamp_u8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

pub fn adjust_brightness_contrast(
    image: &ImageBuffer,
    brightness: i32,
    contrast: i32,
) -> Result<ImageBuffer> {
    if brightness == 0 && contrast == 0 {
        return Ok(image.clone());
    }
    let b = brightness.clamp(-100, 100);
    let c = contrast.clamp(-100, 100);
    let b_off = (b as f64 * 2.55).round() as i32;
    let c_factor = (100 + c) as f64 / 100.0;
    let mut lut = [0u8; 256];
    for (i, item) in lut.iter_mut().enumerate() {
        let v = (((i as f64 - 128.0) * c_factor + 128.0).round() as i32) + b_off;
        *item = clamp_u8(v);
    }
    let out: Vec<u8> = image.data.iter().map(|&px| lut[px as usize]).collect();
    ImageBuffer::new(image.width, image.height, image.pixel_format, out)
}

pub fn desaturate(image: &ImageBuffer) -> Result<ImageBuffer> {
    if image.pixel_format == PixelFormat::Gray8 {
        return Ok(image.clone());
    }
    if image.pixel_format != PixelFormat::Rgb8 {
        return Err(ScanError::Unsupported(format!(
            "desaturate: {:?}",
            image.pixel_format
        )));
    }
    let mut out = vec![0u8; image.data.len()];
    for i in (0..image.data.len()).step_by(3) {
        let r = image.data[i] as u32;
        let g = image.data[i + 1] as u32;
        let b = image.data[i + 2] as u32;
        let y = ((77 * r + 150 * g + 29 * b) >> 8) as u8;
        out[i] = y;
        out[i + 1] = y;
        out[i + 2] = y;
    }
    ImageBuffer::new(image.width, image.height, PixelFormat::Rgb8, out)
}

pub fn invert(image: &ImageBuffer) -> Result<ImageBuffer> {
    let out: Vec<u8> = image.data.iter().map(|&px| 255 - px).collect();
    ImageBuffer::new(image.width, image.height, image.pixel_format, out)
}

pub fn white_balance(image: &ImageBuffer) -> Result<ImageBuffer> {
    if image.pixel_format == PixelFormat::Gray8 {
        return Ok(image.clone());
    }
    if image.pixel_format != PixelFormat::Rgb8 {
        return Err(ScanError::Unsupported(format!(
            "white_balance: {:?}",
            image.pixel_format
        )));
    }
    let n = (image.width * image.height) as f64;
    if n < 1.0 {
        return Ok(image.clone());
    }
    let Some(gains) = white_balance_gains(&image.data, n) else {
        return Ok(image.clone());
    };
    let out = apply_white_balance_gains(&image.data, gains);
    ImageBuffer::new(image.width, image.height, PixelFormat::Rgb8, out)
}

fn white_balance_gains(data: &[u8], pixel_count: f64) -> Option<[f64; 3]> {
    let [red_sum, green_sum, blue_sum] = sum_rgb_channels(data);
    let red_mean = red_sum as f64 / pixel_count;
    let green_mean = green_sum as f64 / pixel_count;
    let blue_mean = blue_sum as f64 / pixel_count;
    if red_mean < 1e-6 || green_mean < 1e-6 || blue_mean < 1e-6 {
        return None;
    }
    let mean = (red_mean + green_mean + blue_mean) / 3.0;
    Some([mean / red_mean, mean / green_mean, mean / blue_mean])
}

fn sum_rgb_channels(data: &[u8]) -> [u64; 3] {
    let mut sums = [0u64; 3];
    for index in (0..data.len()).step_by(3) {
        sums[0] += data[index] as u64;
        sums[1] += data[index + 1] as u64;
        sums[2] += data[index + 2] as u64;
    }
    sums
}

fn apply_white_balance_gains(data: &[u8], gains: [f64; 3]) -> Vec<u8> {
    let mut out = vec![0u8; data.len()];
    for index in (0..data.len()).step_by(3) {
        out[index] = clamp_u8((data[index] as f64 * gains[0]).round() as i32);
        out[index + 1] = clamp_u8((data[index + 1] as f64 * gains[1]).round() as i32);
        out[index + 2] = clamp_u8((data[index + 2] as f64 * gains[2]).round() as i32);
    }
    out
}

pub fn adjust_levels(
    image: &ImageBuffer,
    black: i32,
    white: i32,
    gamma: f64,
) -> Result<ImageBuffer> {
    let black = black.clamp(0, 255);
    let white = white.clamp(0, 255);
    if black == 0 && white == 255 && (gamma - 1.0).abs() < 1e-9 {
        return Ok(image.clone());
    }
    let span = (white - black).max(1) as f64;
    let g = if gamma <= 0.0 { 1.0 } else { gamma };
    let mut lut = [0u8; 256];
    for (i, item) in lut.iter_mut().enumerate() {
        let mut t = (i as i32 - black) as f64 / span;
        t = t.clamp(0.0, 1.0);
        t = t.powf(1.0 / g);
        *item = clamp_u8((t * 255.0).round() as i32);
    }
    let out: Vec<u8> = image.data.iter().map(|&px| lut[px as usize]).collect();
    ImageBuffer::new(image.width, image.height, image.pixel_format, out)
}

pub fn auto_levels(image: &ImageBuffer, clip_percent: f64) -> Result<ImageBuffer> {
    if image.data.is_empty() {
        return Ok(image.clone());
    }
    let mut hist = [0u32; 256];
    for &px in &image.data {
        hist[px as usize] += 1;
    }
    let total = image.data.len() as f64;
    let clip = (clip_percent.clamp(0.0, 20.0) / 100.0) * total;
    let mut black = 0i32;
    let mut acc = 0u32;
    for (i, &count) in hist.iter().enumerate() {
        acc += count;
        if acc as f64 > clip {
            black = i as i32;
            break;
        }
    }
    let mut white = 255i32;
    acc = 0;
    for i in (0..256).rev() {
        acc += hist[i];
        if acc as f64 > clip {
            white = i as i32;
            break;
        }
    }
    if white <= black {
        return Ok(image.clone());
    }
    adjust_levels(image, black, white, 1.0)
}

/// Saturation in [-100..100]; RGB scaled around BT.601 luma.
pub fn adjust_saturation(image: &ImageBuffer, saturation: f64) -> Result<ImageBuffer> {
    let sat = saturation.clamp(-100.0, 100.0);
    if sat.abs() < 1e-9 {
        return Ok(image.clone());
    }
    if image.pixel_format == PixelFormat::Gray8 {
        return Ok(image.clone());
    }
    if image.pixel_format != PixelFormat::Rgb8 {
        return Err(ScanError::Unsupported(format!(
            "adjust_saturation: {:?}",
            image.pixel_format
        )));
    }
    let factor = 1.0 + sat / 100.0;
    let mut out = vec![0u8; image.data.len()];
    for i in (0..image.data.len()).step_by(3) {
        let r = image.data[i] as i32;
        let g = image.data[i + 1] as i32;
        let b = image.data[i + 2] as i32;
        let y = ((77 * r + 150 * g + 29 * b) >> 8) as f64;
        out[i] = clamp_u8((y + (r as f64 - y) * factor).round() as i32);
        out[i + 1] = clamp_u8((y + (g as f64 - y) * factor).round() as i32);
        out[i + 2] = clamp_u8((y + (b as f64 - y) * factor).round() as i32);
    }
    ImageBuffer::new(image.width, image.height, image.pixel_format, out)
}

fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let rf = r as f64 / 255.0;
    let gf = g as f64 / 255.0;
    let bf = b as f64 / 255.0;
    let mx = rf.max(gf).max(bf);
    let mn = rf.min(gf).min(bf);
    let delta = mx - mn;
    if delta < 1e-9 {
        return (0.0, 0.0, mx);
    }
    let h = if (mx - rf).abs() < 1e-12 {
        60.0 * (((gf - bf) / delta).rem_euclid(6.0))
    } else if (mx - gf).abs() < 1e-12 {
        60.0 * ((bf - rf) / delta + 2.0)
    } else {
        60.0 * ((rf - gf) / delta + 4.0)
    };
    let s = if mx == 0.0 { 0.0 } else { delta / mx };
    (h, s, mx)
}

fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (u8, u8, u8) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    (
        clamp_u8(((r + m) * 255.0).round() as i32),
        clamp_u8(((g + m) * 255.0).round() as i32),
        clamp_u8(((b + m) * 255.0).round() as i32),
    )
}

/// Rotate hue in HSV space by `hue_degrees` (mod 360).
pub fn adjust_hue(image: &ImageBuffer, hue_degrees: f64) -> Result<ImageBuffer> {
    let shift = hue_degrees.rem_euclid(360.0);
    if shift.abs() < 1e-9 {
        return Ok(image.clone());
    }
    if image.pixel_format == PixelFormat::Gray8 {
        return Ok(image.clone());
    }
    if image.pixel_format != PixelFormat::Rgb8 {
        return Err(ScanError::Unsupported(format!(
            "adjust_hue: {:?}",
            image.pixel_format
        )));
    }
    let mut out = vec![0u8; image.data.len()];
    for i in (0..image.data.len()).step_by(3) {
        let (h, s, v) = rgb_to_hsv(image.data[i], image.data[i + 1], image.data[i + 2]);
        let h = (h + shift).rem_euclid(360.0);
        let (nr, ng, nb) = hsv_to_rgb(h, s, v);
        out[i] = nr;
        out[i + 1] = ng;
        out[i + 2] = nb;
    }
    ImageBuffer::new(image.width, image.height, image.pixel_format, out)
}

/// Apply a 256-entry LUT built from `[x, y]` control points (0..255).
/// Identity when `points` is `None` or has fewer than 2 points.
pub fn apply_curves(image: &ImageBuffer, points: Option<&[[i32; 2]]>) -> Result<ImageBuffer> {
    let Some(pts_in) = points else {
        return Ok(image.clone());
    };
    if pts_in.len() < 2 {
        return Ok(image.clone());
    }
    let mut pts: Vec<(i32, i32)> = pts_in
        .iter()
        .map(|p| (p[0].clamp(0, 255), p[1].clamp(0, 255)))
        .collect();
    pts.sort_by_key(|p| p.0);
    let xs: Vec<i32> = pts.iter().map(|p| p.0).collect();
    let ys: Vec<i32> = pts.iter().map(|p| p.1).collect();
    let first_x = xs[0];
    let first_y = ys[0];
    let last_x = xs[xs.len() - 1];
    let last_y = ys[ys.len() - 1];
    let mut lut = [0u8; 256];
    let mut seg = 0usize;
    for i in 0..256i32 {
        while seg + 1 < xs.len() && i >= xs[seg + 1] {
            seg += 1;
        }
        let v = if i <= first_x {
            first_y as f64
        } else if i >= last_x {
            last_y as f64
        } else {
            let x0 = xs[seg];
            let x1 = xs[seg + 1];
            if x1 == x0 {
                ys[seg] as f64
            } else {
                let t = (i - x0) as f64 / (x1 - x0) as f64;
                ys[seg] as f64 * (1.0 - t) + ys[seg + 1] as f64 * t
            }
        };
        lut[i as usize] = clamp_u8(v.round() as i32);
    }
    let out: Vec<u8> = image.data.iter().map(|&px| lut[px as usize]).collect();
    ImageBuffer::new(image.width, image.height, image.pixel_format, out)
}

/// Per-channel histograms: `gray` for Gray8, `r/g/b/luma` for RGB.
pub fn histogram(image: &ImageBuffer) -> Result<serde_json::Value> {
    use serde_json::json;
    let count = image.data.len() / image.bpp();
    match image.pixel_format {
        PixelFormat::Gray8 => {
            let mut hist = vec![0u32; 256];
            for &px in &image.data {
                hist[px as usize] += 1;
            }
            Ok(json!({ "gray": hist, "count": count }))
        }
        PixelFormat::Rgb8 | PixelFormat::Rgba8 => {
            let bpp = image.bpp();
            let mut rh = vec![0u32; 256];
            let mut gh = vec![0u32; 256];
            let mut bh = vec![0u32; 256];
            let mut lh = vec![0u32; 256];
            for i in (0..image.data.len()).step_by(bpp) {
                let r = image.data[i] as usize;
                let g = image.data[i + 1] as usize;
                let b = image.data[i + 2] as usize;
                rh[r] += 1;
                gh[g] += 1;
                bh[b] += 1;
                let luma = (77 * r + 150 * g + 29 * b) >> 8;
                lh[luma.min(255)] += 1;
            }
            Ok(json!({
                "r": rh,
                "g": gh,
                "b": bh,
                "luma": lh,
                "count": count,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_rgb_counts() {
        let data = vec![255u8, 0, 0, 0, 255, 0, 0, 0, 255];
        let img = ImageBuffer::new(3, 1, PixelFormat::Rgb8, data).unwrap();
        let h = histogram(&img).unwrap();
        assert_eq!(h["count"], 3);
        assert_eq!(h["r"].as_array().unwrap()[255], 1);
        assert_eq!(h["g"].as_array().unwrap()[255], 1);
        assert_eq!(h["b"].as_array().unwrap()[255], 1);
    }

    #[test]
    fn apply_curves_s_curve_changes_midtones() {
        // Mid-gray gradient sample
        let data: Vec<u8> = (0..16u8)
            .flat_map(|i| {
                let v = 64 + i * 8; // midtones ~64..184
                [v, v, v]
            })
            .collect();
        let img = ImageBuffer::new(16, 1, PixelFormat::Rgb8, data.clone()).unwrap();
        // Classic mild S-curve: darks down, lights up
        let points = [[0, 0], [64, 48], [128, 128], [192, 208], [255, 255]];
        let out = apply_curves(&img, Some(&points)).unwrap();
        assert_ne!(out.data, data, "S-curve must change midtone pixels");
        // Identity points
        let id_pts = [[0, 0], [255, 255]];
        let id = apply_curves(&img, Some(&id_pts)).unwrap();
        assert_eq!(id.data, data);
    }

    #[test]
    fn apply_curves_identity_when_none_or_short() {
        let data = vec![10u8, 20, 30, 40, 50, 60];
        let img = ImageBuffer::new(2, 1, PixelFormat::Rgb8, data.clone()).unwrap();
        assert_eq!(apply_curves(&img, None).unwrap().data, data);
        let one = [[0, 0]];
        assert_eq!(apply_curves(&img, Some(&one)).unwrap().data, data);
        let empty: [[i32; 2]; 0] = [];
        assert_eq!(apply_curves(&img, Some(&empty)).unwrap().data, data);
    }

    #[test]
    fn adjust_saturation_changes_pixels() {
        let data = vec![200u8, 40, 40, 40, 200, 40, 40, 40, 200];
        let img = ImageBuffer::new(3, 1, PixelFormat::Rgb8, data.clone()).unwrap();
        let out = adjust_saturation(&img, 50.0).unwrap();
        assert_ne!(out.data, data);
        let zero = adjust_saturation(&img, 0.0).unwrap();
        assert_eq!(zero.data, data);
    }

    #[test]
    fn adjust_hue_changes_pixels() {
        let data = vec![200u8, 40, 40, 40, 200, 40, 40, 40, 200];
        let img = ImageBuffer::new(3, 1, PixelFormat::Rgb8, data.clone()).unwrap();
        let out = adjust_hue(&img, 90.0).unwrap();
        assert_ne!(out.data, data);
        let zero = adjust_hue(&img, 0.0).unwrap();
        assert_eq!(zero.data, data);
    }

    #[test]
    fn white_balance_preserves_rgb_math_and_format_contracts() {
        let rgb =
            ImageBuffer::new(2, 1, PixelFormat::Rgb8, vec![100, 50, 25, 200, 100, 50]).unwrap();
        assert_eq!(
            white_balance(&rgb).unwrap().data,
            vec![58, 58, 58, 117, 117, 117]
        );

        let gray = ImageBuffer::new(1, 1, PixelFormat::Gray8, vec![42]).unwrap();
        assert_eq!(white_balance(&gray).unwrap(), gray);

        let rgba = ImageBuffer::new(1, 1, PixelFormat::Rgba8, vec![1, 2, 3, 4]).unwrap();
        assert!(matches!(
            white_balance(&rgba),
            Err(ScanError::Unsupported(message)) if message == "white_balance: Rgba8"
        ));
    }
}
