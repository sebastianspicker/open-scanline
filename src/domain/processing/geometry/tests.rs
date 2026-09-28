use super::*;

#[test]
fn cardinal_rotations_match_composed_reference_for_all_formats() {
    for format in [PixelFormat::Gray8, PixelFormat::Rgb8, PixelFormat::Rgba8] {
        let image = seeded_image(7, 5, format, 0x5eed_cafe);
        for (rotation, turns) in [(Rotate::R90, 1), (Rotate::R180, 2), (Rotate::R270, 3)] {
            let expected = repeated_reference_rotation(&image, turns);
            assert_eq!(rotate(&image, rotation).unwrap(), expected);
        }
    }
}

#[test]
fn rotate_270_matches_reference_across_tile_boundaries() {
    for format in [PixelFormat::Gray8, PixelFormat::Rgb8, PixelFormat::Rgba8] {
        let image = seeded_image(37, 35, format, 0x2700_b10c);
        assert_eq!(
            rotate(&image, Rotate::R270).unwrap(),
            repeated_reference_rotation(&image, 3)
        );
    }
}

#[test]
fn rotate_270_preserves_zero_dimension_validation() {
    for (width, height) in [(0, 1), (1, 0)] {
        let image = ImageBuffer {
            width,
            height,
            pixel_format: PixelFormat::Gray8,
            data: Vec::new(),
        };
        let expected = ImageBuffer::new(height, width, PixelFormat::Gray8, Vec::new())
            .unwrap_err()
            .to_string();
        assert_eq!(
            rotate(&image, Rotate::R270).unwrap_err().to_string(),
            expected
        );
    }
}

fn repeated_reference_rotation(image: &ImageBuffer, turns: usize) -> ImageBuffer {
    let mut output = image.clone();
    for _ in 0..turns {
        output = reference_rotate_90(&output);
    }
    output
}

fn reference_rotate_90(image: &ImageBuffer) -> ImageBuffer {
    let bpp = image.bpp();
    let mut output = vec![0; image.data.len()];
    for y in 0..image.height as usize {
        for x in 0..image.width as usize {
            let source = (y * image.width as usize + x) * bpp;
            let destination = (x * image.height as usize + image.height as usize - 1 - y) * bpp;
            output[destination..destination + bpp]
                .copy_from_slice(&image.data[source..source + bpp]);
        }
    }
    ImageBuffer::new(image.height, image.width, image.pixel_format, output).unwrap()
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
