//! Geometry pipeline ops: crop, rotate, flip, deskew.

use std::collections::HashMap;

use crate::core::{ImageBuffer, PixelFormat, Rect, Result, Rotate, ScanError};

pub fn crop(image: &ImageBuffer, region: Rect) -> Result<ImageBuffer> {
    validate_crop(image, region)?;

    let layout = ImageLayout::from(image);
    let crop_width = region.width as usize;
    let mut output = vec![0; crop_width * region.height as usize * layout.bytes_per_pixel];
    let row_bytes = crop_width * layout.bytes_per_pixel;
    for row in 0..region.height as usize {
        let source_offset = layout.offset(region.x as usize, region.y as usize + row);
        copy_bytes(
            &image.data,
            source_offset,
            &mut output,
            row * row_bytes,
            row_bytes,
        );
    }
    ImageBuffer::new(region.width, region.height, image.pixel_format, output)
}

pub fn flip_horizontal(image: &ImageBuffer) -> Result<ImageBuffer> {
    let layout = ImageLayout::from(image);
    let mut output = vec![0; image.data.len()];
    for y in 0..layout.height {
        for x in 0..layout.width {
            copy_pixel(
                &image.data,
                layout.offset(x, y),
                &mut output,
                layout.offset(layout.width - 1 - x, y),
                layout.bytes_per_pixel,
            );
        }
    }
    ImageBuffer::new(image.width, image.height, image.pixel_format, output)
}

pub fn flip_vertical(image: &ImageBuffer) -> Result<ImageBuffer> {
    let layout = ImageLayout::from(image);
    let mut output = vec![0; image.data.len()];
    for y in 0..layout.height {
        copy_bytes(
            &image.data,
            layout.offset(0, y),
            &mut output,
            layout.offset(0, layout.height - 1 - y),
            layout.row_bytes(),
        );
    }
    ImageBuffer::new(image.width, image.height, image.pixel_format, output)
}

pub fn rotate(image: &ImageBuffer, how: Rotate) -> Result<ImageBuffer> {
    match how {
        Rotate::None => Ok(image.clone()),
        Rotate::R180 => flip_horizontal(&flip_vertical(image)?),
        Rotate::R90 => rotate90_cw(image),
        Rotate::R270 => rotate90_cw(&rotate90_cw(&rotate90_cw(image)?)?),
    }
}

fn rotate90_cw(image: &ImageBuffer) -> Result<ImageBuffer> {
    let layout = ImageLayout::from(image);
    let output_layout = ImageLayout::new(layout.height, layout.width, layout.bytes_per_pixel);
    let mut output = vec![0; output_layout.len()];
    for y in 0..layout.height {
        for x in 0..layout.width {
            copy_pixel(
                &image.data,
                layout.offset(x, y),
                &mut output,
                output_layout.offset(layout.height - 1 - y, x),
                layout.bytes_per_pixel,
            );
        }
    }
    ImageBuffer::new(
        output_layout.width as u32,
        output_layout.height as u32,
        image.pixel_format,
        output,
    )
}

/// Estimate skew via horizontal projection variance over candidate angles.
/// Matches the `estimate_skew_degrees` product contract (pure algorithm).
pub fn estimate_skew_degrees(image: &ImageBuffer, max_angle: f64) -> f64 {
    let layout = ImageLayout::from(image);
    if layout.width < 8 || layout.height < 8 {
        return 0.0;
    }

    let samples = dark_samples(image, layout);
    if samples.len() < 20 {
        return 0.0;
    }
    best_projection_angle(&samples, max_angle)
}

/// Rotate by `angle_degrees` (counter-clockwise) with expand canvas, white fill.
/// Nearest-neighbor inverse mapping (pure-fallback path).
pub fn deskew(image: &ImageBuffer, angle_degrees: f64) -> Result<ImageBuffer> {
    if angle_degrees.abs() < 0.05 {
        return Ok(image.clone());
    }

    let source_layout = ImageLayout::from(image);
    let mapping = DeskewMapping::new(source_layout, angle_degrees);
    let mut output = vec![255; mapping.output_layout.len()];
    copy_deskewed_pixels(&image.data, &mut output, mapping);
    ImageBuffer::new(
        mapping.output_layout.width as u32,
        mapping.output_layout.height as u32,
        image.pixel_format,
        output,
    )
}

/// Detect skew and deskew in one call.
pub fn auto_deskew(image: &ImageBuffer, max_angle: f64) -> Result<ImageBuffer> {
    let angle = estimate_skew_degrees(image, max_angle);
    if angle.abs() < 0.05 {
        return Ok(image.clone());
    }
    deskew(image, angle)
}

#[derive(Clone, Copy)]
struct ImageLayout {
    width: usize,
    height: usize,
    bytes_per_pixel: usize,
}

impl ImageLayout {
    fn from(image: &ImageBuffer) -> Self {
        Self::new(image.width as usize, image.height as usize, image.bpp())
    }

    fn new(width: usize, height: usize, bytes_per_pixel: usize) -> Self {
        Self {
            width,
            height,
            bytes_per_pixel,
        }
    }

    fn offset(self, x: usize, y: usize) -> usize {
        (y * self.width + x) * self.bytes_per_pixel
    }

    fn row_bytes(self) -> usize {
        self.width * self.bytes_per_pixel
    }

    fn len(self) -> usize {
        self.width * self.height * self.bytes_per_pixel
    }
}

fn validate_crop(image: &ImageBuffer, region: Rect) -> Result<()> {
    if !is_crop_format_supported(image.pixel_format) {
        return Err(ScanError::Unsupported(
            "crop: unsupported pixel format".into(),
        ));
    }
    if !has_valid_crop_shape(region) {
        return Err(ScanError::Invalid("invalid crop region".into()));
    }
    if !crop_fits_image(image, region) {
        return Err(ScanError::Invalid("crop region out of bounds".into()));
    }
    Ok(())
}

fn is_crop_format_supported(pixel_format: PixelFormat) -> bool {
    matches!(
        pixel_format,
        PixelFormat::Rgb8 | PixelFormat::Gray8 | PixelFormat::Rgba8
    )
}

fn has_valid_crop_shape(region: Rect) -> bool {
    region.x >= 0 && region.y >= 0 && region.width >= 1 && region.height >= 1
}

fn crop_fits_image(image: &ImageBuffer, region: Rect) -> bool {
    region.x as u32 + region.width <= image.width && region.y as u32 + region.height <= image.height
}

fn copy_bytes(
    source: &[u8],
    source_offset: usize,
    output: &mut [u8],
    output_offset: usize,
    len: usize,
) {
    output[output_offset..output_offset + len]
        .copy_from_slice(&source[source_offset..source_offset + len]);
}

fn copy_pixel(
    source: &[u8],
    source_offset: usize,
    output: &mut [u8],
    output_offset: usize,
    bpp: usize,
) {
    copy_bytes(source, source_offset, output, output_offset, bpp);
}

fn dark_samples(image: &ImageBuffer, layout: ImageLayout) -> Vec<(i32, i32)> {
    let step = (layout.width.min(layout.height) as i32 / 128).max(1) as usize;
    let mut samples = Vec::new();
    for y in (0..layout.height).step_by(step) {
        for x in (0..layout.width).step_by(step) {
            if luma_at(image, layout.offset(x, y)) < 200 {
                samples.push((x as i32, y as i32));
            }
        }
    }
    samples
}

fn luma_at(image: &ImageBuffer, offset: usize) -> u8 {
    if image.bpp() == 1 {
        image.data[offset]
    } else {
        let red = image.data[offset] as u32;
        let green = image.data[offset + 1] as u32;
        let blue = image.data[offset + 2] as u32;
        ((77 * red + 150 * green + 29 * blue) >> 8) as u8
    }
}

fn best_projection_angle(samples: &[(i32, i32)], max_angle: f64) -> f64 {
    let mut best_angle = 0.0;
    let mut best_score = -1.0;
    let max_steps = (max_angle * 2.0) as i32;
    for candidate in -max_steps..=max_steps {
        let angle = candidate as f64 * 0.5;
        if let Some(score) = projection_variance(samples, angle) {
            if score > best_score {
                best_score = score;
                best_angle = angle;
            }
        }
    }
    best_angle
}

fn projection_variance(samples: &[(i32, i32)], angle: f64) -> Option<f64> {
    let radians = angle.to_radians();
    let cosine = radians.cos();
    let sine = radians.sin();
    let mut bins = HashMap::new();
    for &(x, y) in samples {
        let projection = (-(x as f64) * sine + (y as f64) * cosine).round() as i32;
        *bins.entry(projection).or_insert(0_i32) += 1;
    }
    if bins.is_empty() {
        return None;
    }

    let count = bins.len() as f64;
    let mean = bins.values().map(|&value| value as f64).sum::<f64>() / count;
    Some(
        bins.values()
            .map(|&value| {
                let difference = value as f64 - mean;
                difference * difference
            })
            .sum::<f64>()
            / count,
    )
}

#[derive(Clone, Copy)]
struct DeskewMapping {
    source_layout: ImageLayout,
    output_layout: ImageLayout,
    source_center: (f64, f64),
    output_center: (f64, f64),
    cosine: f64,
    inverse_sine: f64,
}

impl DeskewMapping {
    fn new(source_layout: ImageLayout, angle_degrees: f64) -> Self {
        let radians = angle_degrees.to_radians();
        let cosine = radians.cos();
        let sine = radians.sin();
        let source_center = center_of(source_layout);
        let output_layout = expanded_layout(source_layout, source_center, cosine, sine);
        Self {
            source_layout,
            output_layout,
            source_center,
            output_center: center_of(output_layout),
            cosine,
            inverse_sine: -sine,
        }
    }

    fn source_pixel(self, output_x: usize, output_y: usize) -> Option<(usize, usize)> {
        let source_x = self.cosine * (output_x as f64 - self.output_center.0)
            - self.inverse_sine * (output_y as f64 - self.output_center.1)
            + self.source_center.0;
        let source_y = self.inverse_sine * (output_x as f64 - self.output_center.0)
            + self.cosine * (output_y as f64 - self.output_center.1)
            + self.source_center.1;
        let x = source_x.round() as i32;
        let y = source_y.round() as i32;
        if x >= 0
            && x < self.source_layout.width as i32
            && y >= 0
            && y < self.source_layout.height as i32
        {
            Some((x as usize, y as usize))
        } else {
            None
        }
    }
}

fn center_of(layout: ImageLayout) -> (f64, f64) {
    (
        (layout.width as f64 - 1.0) / 2.0,
        (layout.height as f64 - 1.0) / 2.0,
    )
}

fn expanded_layout(
    source_layout: ImageLayout,
    source_center: (f64, f64),
    cosine: f64,
    sine: f64,
) -> ImageLayout {
    let (min_x, max_x, min_y, max_y) = rotated_bounds(source_layout, source_center, cosine, sine);
    let width = ((max_x - min_x + 1.0).ceil() as usize).max(1);
    let height = ((max_y - min_y + 1.0).ceil() as usize).max(1);
    ImageLayout::new(width, height, source_layout.bytes_per_pixel)
}

fn rotated_bounds(
    layout: ImageLayout,
    center: (f64, f64),
    cosine: f64,
    sine: f64,
) -> (f64, f64, f64, f64) {
    let corners = [
        (0.0, 0.0),
        (layout.width as f64 - 1.0, 0.0),
        (0.0, layout.height as f64 - 1.0),
        (layout.width as f64 - 1.0, layout.height as f64 - 1.0),
    ];
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (x, y) in corners {
        let rotated_x = cosine * (x - center.0) - sine * (y - center.1) + center.0;
        let rotated_y = sine * (x - center.0) + cosine * (y - center.1) + center.1;
        min_x = min_x.min(rotated_x);
        max_x = max_x.max(rotated_x);
        min_y = min_y.min(rotated_y);
        max_y = max_y.max(rotated_y);
    }
    (min_x, max_x, min_y, max_y)
}

fn copy_deskewed_pixels(source: &[u8], output: &mut [u8], mapping: DeskewMapping) {
    for output_y in 0..mapping.output_layout.height {
        for output_x in 0..mapping.output_layout.width {
            if let Some((source_x, source_y)) = mapping.source_pixel(output_x, output_y) {
                copy_pixel(
                    source,
                    mapping.source_layout.offset(source_x, source_y),
                    output,
                    mapping.output_layout.offset(output_x, output_y),
                    mapping.source_layout.bytes_per_pixel,
                );
            }
        }
    }
}
