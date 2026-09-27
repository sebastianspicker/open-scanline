use super::*;

#[test]
fn neighborhood_filters_match_seeded_references_for_all_formats() {
    for format in [PixelFormat::Gray8, PixelFormat::Rgb8, PixelFormat::Rgba8] {
        let image = seeded_image(9, 7, format, 0x51a7_0ff5);
        for radius in [1, 2, 3] {
            assert_eq!(
                median_filter(&image, radius).unwrap(),
                reference_median(&image, radius)
            );
            assert_eq!(
                box_blur(&image, radius).unwrap(),
                reference_box_blur(&image, radius)
            );
        }
    }
}

#[test]
fn box_blur_matches_reference_when_radius_exceeds_dimensions() {
    for format in [PixelFormat::Gray8, PixelFormat::Rgb8, PixelFormat::Rgba8] {
        let image = seeded_image(2, 1, format, 0xa11c_e551);
        assert_eq!(box_blur(&image, 5).unwrap(), reference_box_blur(&image, 5));
    }
}

#[test]
fn box_blur_preserves_zero_dimension_validation() {
    let image = ImageBuffer {
        width: 0,
        height: 1,
        pixel_format: PixelFormat::Gray8,
        data: Vec::new(),
    };
    let expected = ImageBuffer::new(0, 1, PixelFormat::Gray8, Vec::new())
        .unwrap_err()
        .to_string();
    assert_eq!(box_blur(&image, 1).unwrap_err().to_string(), expected);
}

fn reference_median(image: &ImageBuffer, radius: i32) -> ImageBuffer {
    let context = ReferenceContext::new(image, radius);
    let data = (0..image.data.len())
        .map(|index| {
            let (x, y, channel) = context.coordinates(index);
            context.median(x, y, channel)
        })
        .collect();
    ImageBuffer::new(image.width, image.height, image.pixel_format, data).unwrap()
}

fn reference_box_blur(image: &ImageBuffer, radius: i32) -> ImageBuffer {
    let context = ReferenceContext::new(image, radius);
    let horizontal = context.blur_pass(&image.data, true);
    let output = context.blur_pass(&horizontal, false);
    ImageBuffer::new(image.width, image.height, image.pixel_format, output).unwrap()
}

struct ReferenceContext<'a> {
    image: &'a ImageBuffer,
    radius: i32,
}

impl<'a> ReferenceContext<'a> {
    fn new(image: &'a ImageBuffer, radius: i32) -> Self {
        Self {
            image,
            radius: radius.max(1),
        }
    }

    fn coordinates(&self, index: usize) -> (i32, i32, usize) {
        let pixel = index / self.image.bpp();
        let x = (pixel % self.image.width as usize) as i32;
        let y = (pixel / self.image.width as usize) as i32;
        (x, y, index % self.image.bpp())
    }

    fn index(&self, x: i32, y: i32, channel: usize) -> usize {
        (y as usize * self.image.width as usize + x as usize) * self.image.bpp() + channel
    }

    fn median(&self, x: i32, y: i32, channel: usize) -> u8 {
        let mut values = Vec::new();
        for dy in -self.radius..=self.radius {
            for dx in -self.radius..=self.radius {
                let xx = (x + dx).clamp(0, self.image.width as i32 - 1);
                let yy = (y + dy).clamp(0, self.image.height as i32 - 1);
                values.push(self.image.data[self.index(xx, yy, channel)]);
            }
        }
        values.sort_unstable();
        values[values.len() / 2]
    }

    fn blur_pass(&self, source: &[u8], horizontal: bool) -> Vec<u8> {
        (0..source.len())
            .map(|index| {
                let (x, y, channel) = self.coordinates(index);
                self.blur_at(source, x, y, channel, horizontal)
            })
            .collect()
    }

    fn blur_at(&self, source: &[u8], x: i32, y: i32, channel: usize, horizontal: bool) -> u8 {
        let (axis, limit) = if horizontal {
            (x, self.image.width as i32)
        } else {
            (y, self.image.height as i32)
        };
        let start = (axis - self.radius).max(0);
        let end = (axis + self.radius).min(limit - 1);
        let sum = (start..=end)
            .map(|position| self.blur_value(source, x, y, channel, horizontal, position) as u32)
            .sum::<u32>();
        (sum / (end - start + 1) as u32) as u8
    }

    fn blur_value(
        &self,
        source: &[u8],
        x: i32,
        y: i32,
        channel: usize,
        horizontal: bool,
        position: i32,
    ) -> u8 {
        let (x, y) = if horizontal {
            (position, y)
        } else {
            (x, position)
        };
        source[self.index(x, y, channel)]
    }
}

fn seeded_image(width: u32, height: u32, format: PixelFormat, seed: u32) -> ImageBuffer {
    let mut state = seed;
    let data = (0..width as usize * height as usize * format.bpp())
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect();
    ImageBuffer::new(width, height, format, data).unwrap()
}
