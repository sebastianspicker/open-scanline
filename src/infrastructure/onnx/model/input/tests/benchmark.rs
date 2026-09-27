use super::{fixture, measure_allocations, oracle_image_tensor};
use crate::domain::image::PixelFormat;
use crate::infrastructure::onnx::model::input::image_tensor;
use crate::infrastructure::onnx::model::types::{OnnxInputLayout, OnnxNormalization};
use serde_json::{json, Value};
use std::hint::black_box;
use std::time::Instant;

const SEED: u64 = 1_592_594_996;
const WARMUPS: usize = 3;
const REPETITIONS: usize = 10;

#[test]
#[ignore = "release-only timing and resource benchmark"]
fn release_benchmark_emits_json_for_one_and_twenty_four_megapixels() {
    assert!(
        !cfg!(debug_assertions),
        "run this ignored benchmark with --release"
    );
    let mut results = Vec::new();
    for (width, height) in [(1_000, 1_000), (6_000, 4_000)] {
        let image = fixture(width, height, PixelFormat::Rgb8, SEED);
        let shape = [1, 3, 224, 224];
        results.push(measure_case("baseline", &image, || {
            oracle_image_tensor(
                &image,
                &shape,
                OnnxInputLayout::Nchw,
                OnnxNormalization::ZeroToOne,
            )
            .unwrap()
        }));
        results.push(measure_case("direct", &image, || {
            image_tensor(
                &image,
                &shape,
                OnnxInputLayout::Nchw,
                OnnxNormalization::ZeroToOne,
            )
            .unwrap()
        }));
    }
    let report = json!({
        "seed": SEED,
        "warmups": WARMUPS,
        "repetitions": REPETITIONS,
        "profile": "release",
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "results": results,
    });
    println!("ONNX_INPUT_BENCHMARK_JSON={report}");
}

fn measure_case<T>(
    operation: &str,
    image: &crate::domain::image::ImageBuffer,
    mut run: impl FnMut() -> T,
) -> Value {
    for _ in 0..WARMUPS {
        black_box(run());
    }
    let mut samples = Vec::with_capacity(REPETITIONS);
    for _ in 0..REPETITIONS {
        let start = Instant::now();
        let (_, allocation) = measure_allocations(|| black_box(run()));
        samples.push(json!({
            "ns": start.elapsed().as_nanos() as u64,
            "allocations": allocation.allocations,
            "allocated_bytes": allocation.bytes,
        }));
    }
    json!({
        "operation": operation,
        "width": image.width,
        "height": image.height,
        "format": image.pixel_format.as_str(),
        "target": [1, 3, 224, 224],
        "samples": samples,
    })
}
