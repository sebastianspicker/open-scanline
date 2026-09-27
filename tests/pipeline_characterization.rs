//! Golden outputs captured from the pre-optimization working-tree baseline.
use open_scanline::core::{ImageBuffer, PipelinePrefs, PixelFormat};
use open_scanline::pipeline::apply_pipeline;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn cases() -> Vec<Value> {
    vec![
        json!({}),
        json!({"rotate":"R180"}),
        json!({"rotate":"R270"}),
        json!({"flip_h":true}),
        json!({"flip_v":true}),
        json!({"crop":{"x":1,"y":2,"width":13,"height":11}}),
        json!({"deskew_angle":2.0}),
        json!({"auto_deskew":true}),
        json!({"brightness":17,"contrast":-11}),
        json!({"desaturate":true}),
        json!({"levels_black":8,"levels_white":230,"levels_gamma":1.3}),
        json!({"white_balance":true}),
        json!({"saturation":0.2,"hue":23.0}),
        json!({"curves":[[0,5],[100,120],[255,250]]}),
        json!({"auto_levels":true}),
        json!({"invert":true}),
        json!({"sharpen_amount":0.7}),
        json!({"infrared_clean":"medium"}),
        json!({"descreen":true}),
        json!({"grain_reduction":"heavy"}),
        json!({"restore_colors":true}),
        json!({"restore_fading":true}),
        json!({"flatten":true}),
        json!({"hole_punch":true}),
        json!({"colorize_mode":"sepia"}),
        json!({"film_type":"color-negative"}),
        json!({"auto_orient":true,"auto_crop":true}),
        json!({"rotate":"R270","flip_h":true,"brightness":17,"contrast":-11,
            "levels_black":8,"levels_white":230,"levels_gamma":1.3,"invert":true}),
        json!({"desaturate":true,"sharpen_amount":0.7,"descreen":true,
            "grain_reduction":"light","flatten":true,"auto_crop":true}),
    ]
}

fn fixture(format: PixelFormat) -> ImageBuffer {
    let mut state = 0x5eed_1234_u32;
    let data = (0..19 * 17 * format.bpp())
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect();
    ImageBuffer::new(19, 17, format, data).unwrap()
}

fn outcome(image: &ImageBuffer, value: Value) -> Value {
    let prefs: PipelinePrefs = serde_json::from_value(value.clone()).unwrap();
    let result = match apply_pipeline(image, &prefs) {
        Ok(output) => json!({"width":output.width,"height":output.height,
            "format":output.pixel_format.as_str(),
            "sha256":format!("{:x}", Sha256::digest(&output.data))}),
        Err(error) => json!({"error":error.to_string()}),
    };
    json!({"input":image.pixel_format.as_str(),"prefs":value,"result":result})
}

fn snapshot() -> Vec<Value> {
    [PixelFormat::Gray8, PixelFormat::Rgb8, PixelFormat::Rgba8]
        .into_iter()
        .flat_map(|format| {
            let image = fixture(format);
            cases().into_iter().map(move |case| outcome(&image, case))
        })
        .collect()
}

#[test]
fn seeded_operations_and_combinations_match_baseline() {
    let expected: Vec<Value> =
        serde_json::from_str(include_str!("pipeline_characterization/baseline.json")).unwrap();
    assert_eq!(snapshot(), expected);
}
