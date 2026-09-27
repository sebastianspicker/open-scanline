use super::*;

#[test]
fn prepared_profile_matches_public_api_and_reuses_owned_allocation() {
    let profile = json!({
        "format": PROFILE_FORMAT,
        "version": PROFILE_VERSION,
        "kind": "scanner_it8",
        "matrix": [[1.1, 0.1, 0.0], [0.0, 0.9, 0.1], [0.1, 0.0, 1.2]],
        "gamma": [1.2, 0.9, 1.1]
    });
    let image = ImageBuffer::new(
        3,
        2,
        PixelFormat::Rgb8,
        vec![
            3, 51, 99, 127, 128, 129, 220, 17, 81, 6, 250, 33, 49, 87, 210, 255, 0, 144,
        ],
    )
    .unwrap();
    let expected = ImageBuffer::new(
        3,
        2,
        PixelFormat::Rgb8,
        vec![
            15, 47, 128, 166, 119, 174, 246, 18, 128, 45, 226, 48, 79, 89, 255, 255, 10, 203,
        ],
    )
    .unwrap();
    assert_eq!(apply_scanner_profile(&image, &profile).unwrap(), expected);
    let pointer = image.data.as_ptr();
    let actual = prepare_scanner_profile(&profile)
        .unwrap()
        .apply_owned(image)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual.data.as_ptr(), pointer);
}

#[test]
fn prepared_profile_matches_seeded_json_reference() {
    let mut state = 0x1cc0_51a7;
    for _ in 0..8 {
        let profile = seeded_profile(&mut state);
        let image = seeded_rgb_image(257, 31, &mut state);
        let expected = reference_profile_application(&image, &profile);
        let actual = prepare_scanner_profile(&profile)
            .unwrap()
            .apply_owned(image)
            .unwrap();
        assert_eq!(actual, expected);
    }
}

fn reference_profile_application(image: &ImageBuffer, profile: &Value) -> ImageBuffer {
    let matrix = profile["matrix"].as_array().unwrap();
    let gamma = profile["gamma"].as_array().unwrap();
    let mut data = image.data.clone();
    for pixel in data.chunks_exact_mut(3) {
        let input = [
            pixel[0] as f64 / 255.0,
            pixel[1] as f64 / 255.0,
            pixel[2] as f64 / 255.0,
        ];
        for channel in 0..3 {
            let row = matrix[channel].as_array().unwrap();
            let mixed = (0..3)
                .map(|column| row[column].as_f64().unwrap() * input[column])
                .sum::<f64>()
                .clamp(0.0, 1.0);
            let exponent = 1.0 / gamma[channel].as_f64().unwrap();
            pixel[channel] = clamp_byte(mixed.powf(exponent) * 255.0);
        }
    }
    ImageBuffer::new(image.width, image.height, PixelFormat::Rgb8, data).unwrap()
}

fn seeded_profile(state: &mut u32) -> Value {
    let matrix: Vec<Vec<f64>> = (0..3)
        .map(|row| {
            (0..3)
                .map(|column| {
                    let scale = if row == column { 1.4 } else { 0.3 };
                    (seeded_byte(state) as f64 / 255.0 - 0.15) * scale
                })
                .collect()
        })
        .collect();
    let gamma: Vec<f64> = (0..3)
        .map(|_| 0.5 + seeded_byte(state) as f64 / 170.0)
        .collect();
    json!({
        "format": PROFILE_FORMAT,
        "version": PROFILE_VERSION,
        "kind": "scanner_it8",
        "matrix": matrix,
        "gamma": gamma
    })
}

fn seeded_rgb_image(width: u32, height: u32, state: &mut u32) -> ImageBuffer {
    let data = (0..width as usize * height as usize * 3)
        .map(|_| seeded_byte(state))
        .collect();
    ImageBuffer::new(width, height, PixelFormat::Rgb8, data).unwrap()
}

fn seeded_byte(state: &mut u32) -> u8 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*state >> 24) as u8
}

#[test]
fn public_profile_rejects_wrong_sized_rgb_buffers() {
    let mut seed = 17;
    let profile = seeded_profile(&mut seed);
    for length in [3, 9] {
        let image = ImageBuffer {
            width: 2,
            height: 1,
            pixel_format: PixelFormat::Rgb8,
            data: vec![1; length],
        };
        let expected = ImageBuffer::new(2, 1, PixelFormat::Rgb8, image.data.clone())
            .unwrap_err()
            .to_string();
        assert_eq!(
            apply_scanner_profile(&image, &profile)
                .unwrap_err()
                .to_string(),
            expected
        );
    }
}
