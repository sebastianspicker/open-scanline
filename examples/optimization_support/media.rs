use super::{measure, selected};
use open_scanline::core::{ImageBuffer, PixelFormat};
use open_scanline::icc::apply_scanner_profile;
use open_scanline::imaging::{save_image, to_rgb_bytes};
use serde_json::{json, Value};
use std::hint::black_box;

pub fn measure_media(image: &ImageBuffer) -> Vec<Value> {
    let directory = BenchmarkDirectory::new();
    let mut results = Vec::new();
    if selected("to_rgb") {
        results.push(measure("to_rgb", image, || {
            black_box(to_rgb_bytes(image).unwrap());
        }));
    }
    for extension in ["bmp", "jpg"].into_iter().filter(|name| selected(name)) {
        let path = directory.0.join(format!("output.{extension}"));
        results.push(measure(extension, image, || {
            black_box(save_image(&path, image, None, Some(83)).unwrap());
        }));
    }
    if image.pixel_format == PixelFormat::Rgb8 && selected("scanner_profile") {
        let profile = json!({"format":"open-scanline-icc-profile", "version":1,
            "kind":"scanner_it8", "matrix":[[1.1,0.1,0.0],[0.0,0.9,0.1],[0.1,0.0,1.2]],
            "gamma":[1.2,0.9,1.1]});
        results.push(measure("scanner_profile", image, || {
            black_box(apply_scanner_profile(image, &profile).unwrap());
        }));
    }
    results
}

struct BenchmarkDirectory(std::path::PathBuf);
impl BenchmarkDirectory {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("open-scanline-benchmark-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for BenchmarkDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
