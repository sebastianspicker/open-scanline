//! Deterministic release benchmark; output is JSON and timing is never a CI gate.
mod optimization_support;
use open_scanline::core::{ImageBuffer, PipelinePrefs, PixelFormat, Rotate};
use open_scanline::pipeline::{apply_pipeline, box_blur, median_filter, rotate};
use optimization_support as optimization;
use serde_json::json;
use std::hint::black_box;

#[global_allocator]
static ALLOCATOR: optimization::CountingAllocator = optimization::CountingAllocator;

fn main() {
    let mut results = Vec::new();
    for (width, height) in [(1000, 1000), (6000, 4000)] {
        for format in [PixelFormat::Gray8, PixelFormat::Rgb8, PixelFormat::Rgba8] {
            let image = fixture(width, height, format);
            results.extend(measure_image(&image));
        }
    }
    println!(
        "{}",
        json!({"seed": 1592594996_u64, "warmups": 3, "repetitions": 10,
        "profile": "release", "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
        "peak_rss_bytes": optimization::peak_rss(), "results": results})
    );
}

fn measure_image(image: &ImageBuffer) -> Vec<serde_json::Value> {
    let mut results = Vec::new();
    for operation in ["pipeline_default", "rotate180", "rotate270", "box_blur"]
        .into_iter()
        .filter(|operation| optimization::selected(operation))
    {
        results.push(optimization::measure(operation, image, || {
            let output = match operation {
                "pipeline_default" => apply_pipeline(image, &PipelinePrefs::default()),
                "rotate180" => rotate(image, Rotate::R180),
                "rotate270" => rotate(image, Rotate::R270),
                _ => box_blur(image, 2),
            }
            .unwrap();
            black_box(output);
        }));
    }
    if std::env::var_os("OPEN_SCANLINE_BENCH_MEDIAN").is_some()
        && optimization::selected("median_r1")
    {
        results.push(optimization::measure("median_r1", image, || {
            black_box(median_filter(image, 1).unwrap());
        }));
    }
    if std::env::var_os("OPEN_SCANLINE_BENCH_MEDIA").is_some() {
        results.extend(optimization::media::measure_media(image));
    }
    results
}

fn fixture(width: u32, height: u32, format: PixelFormat) -> ImageBuffer {
    let mut state = 1592594996_u64;
    let data = (0..width as usize * height as usize * format.bpp())
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect();
    ImageBuffer::new(width, height, format, data).unwrap()
}
