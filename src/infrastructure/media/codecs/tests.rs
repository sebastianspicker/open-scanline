use super::*;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn borrowed_standard_encoders_match_owned_dynamic_reference() {
    let directory = TestDirectory::new();
    for format in [PixelFormat::Gray8, PixelFormat::Rgb8, PixelFormat::Rgba8] {
        let image = seeded_image(11, 7, format, 0xc0de_cafe);
        for extension in ["png", "jpg", "tiff", "webp", "bmp", "gif"] {
            compare_encoding(&directory, &image, extension);
        }
    }
}

#[test]
fn direct_rgb_conversion_matches_dynamic_conversion() {
    for format in [PixelFormat::Gray8, PixelFormat::Rgb8, PixelFormat::Rgba8] {
        let image = seeded_image(5, 3, format, 0x7ab1_e123);
        let expected = buffer_to_dynamic(&image).unwrap().to_rgb8().into_raw();
        assert_eq!(to_rgb_bytes(&image).unwrap(), expected);
    }
}

fn compare_encoding(directory: &TestDirectory, image: &ImageBuffer, extension: &str) {
    let direct = directory.path(format!("direct.{extension}"));
    let reference = directory.path(format!("reference.{extension}"));
    std::fs::write(&direct, []).unwrap();
    let direct_result = write_standard_image(image, extension, &direct, Some(83));
    let reference_result = write_reference(image, extension, &reference, Some(83));
    assert_eq!(
        direct_result.as_ref().map_err(ToString::to_string),
        reference_result.as_ref().map_err(ToString::to_string),
        "result changed for {} {extension}",
        image.pixel_format.as_str()
    );
    if direct_result.is_ok() {
        assert_eq!(
            std::fs::read(direct).unwrap(),
            std::fs::read(reference).unwrap(),
            "encoded bytes changed for {} {extension}",
            image.pixel_format.as_str()
        );
    }
}

fn write_reference(
    image: &ImageBuffer,
    extension: &str,
    output: &Path,
    quality: Option<u8>,
) -> Result<()> {
    let dynamic = buffer_to_dynamic(image)?;
    if matches!(extension, "jpg" | "jpeg") {
        return write_reference_jpeg(&dynamic, output, quality);
    }
    dynamic
        .save_with_format(output, standard_image_format(extension)?)
        .map_err(|error| ScanError::Image(error.to_string()))
}

fn write_reference_jpeg(image: &DynamicImage, output: &Path, quality: Option<u8>) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(output)?;
    let mut encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut file, quality.unwrap_or(90));
    encoder
        .encode_image(image)
        .map_err(|error| ScanError::Image(error.to_string()))?;
    file.flush()?;
    Ok(())
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

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "open-scanline-codecs-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: impl AsRef<Path>) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn malformed_borrowed_buffers_keep_dynamic_conversion_errors() {
    let directory = TestDirectory::new();
    for format in [PixelFormat::Gray8, PixelFormat::Rgb8, PixelFormat::Rgba8] {
        let image = ImageBuffer {
            width: 2,
            height: 2,
            pixel_format: format,
            data: vec![0],
        };
        let expected = buffer_to_dynamic(&image).unwrap_err().to_string();
        assert_eq!(to_rgb_bytes(&image).unwrap_err().to_string(), expected);
        let path = directory.path("invalid.png");
        assert_eq!(
            save_image(&path, &image, None, None)
                .unwrap_err()
                .to_string(),
            expected
        );
        assert!(!path.exists());
    }
}

#[test]
fn direct_rgb_conversion_preserves_trailing_buffer_behavior() {
    for format in [PixelFormat::Gray8, PixelFormat::Rgb8, PixelFormat::Rgba8] {
        let mut image = seeded_image(5, 3, format, 0x7ab1_e123);
        image.data.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        let expected = buffer_to_dynamic(&image).unwrap().to_rgb8().into_raw();
        assert_eq!(to_rgb_bytes(&image).unwrap(), expected);
    }
}
