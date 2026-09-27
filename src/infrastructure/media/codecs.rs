use super::pdf::{save_pdf_with_options_and_cancellation, PdfOptions};
use crate::domain::image::{ImageBuffer, PixelFormat, MAX_IMAGE_BYTES};
use crate::error::{Result, ScanError};
use crate::infrastructure::runtime::{
    run_contained_command_with_artifact_quota, ArtifactQuota, ArtifactWatch, CommandSpec,
    TemporaryOutput,
};
use crate::operation::CancellationToken;
use image::{ColorType, DynamicImage, ImageBuffer as ImgBuf, ImageFormat, Rgb, Rgba};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

mod bmp;
mod jpeg;
mod temp_output;
use jpeg::write_jpeg;
#[cfg(test)]
mod tests;
pub(super) use temp_output::{create_output_temp, OutputTemp};

pub const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
pub(super) const JPEG_MAGIC: &[u8] = b"\xff\xd8";
pub(super) const TIFF_MAGIC_LE: &[u8] = b"II";
pub(super) const TIFF_MAGIC_BE: &[u8] = b"MM";

const PREFIX_FORMATS: &[(&[u8], &str)] = &[
    (PNG_MAGIC, "png"),
    (JPEG_MAGIC, "jpeg"),
    (TIFF_MAGIC_LE, "tiff"),
    (TIFF_MAGIC_BE, "tiff"),
    (b"GIF87a", "gif"),
    (b"GIF89a", "gif"),
    (b"BM", "bmp"),
    (b"%PDF-", "pdf"),
    (b"\xff\x0a", "jxl"),
];
const WEBP_RIFF_MAGIC: &[u8] = b"RIFF";
const WEBP_FORMAT_MAGIC: &[u8] = b"WEBP";
const JXL_CONTAINER_MAGIC: &[u8] = b"\0\0\0\x0cJXL \r\n\x87\n";

/// Read enough leading bytes to recognize every supported container.
pub fn read_magic(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let mut f = std::fs::File::open(path.as_ref())?;
    let mut buf = [0u8; 12];
    let n = f.read(&mut buf)?;
    Ok(buf[..n].to_vec())
}

/// Return true when `bytes` start with the PNG signature.
pub fn is_png_magic(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && &bytes[..8] == PNG_MAGIC
}

/// Validate that file header is PNG; error otherwise.
pub fn validate_png_magic(path: impl AsRef<Path>) -> Result<()> {
    let head = read_magic(path.as_ref())?;
    if !is_png_magic(&head) {
        return Err(ScanError::Invalid(format!(
            "not a PNG (bad magic) at {}",
            path.as_ref().display()
        )));
    }
    Ok(())
}

/// Extensions accepted by [`save_image`]. JPEG XL additionally requires `cjxl`.
pub fn supported_extensions() -> &'static [&'static str] {
    &[
        "png", "jpg", "jpeg", "tif", "tiff", "webp", "bmp", "gif", "pdf", "jxl",
    ]
}

/// Detect a supported container from its magic bytes.
pub fn detect_format_bytes(head: &[u8]) -> Option<&'static str> {
    detect_webp(head)
        .or_else(|| detect_jxl_container(head))
        .or_else(|| detect_prefix_format(head))
}

fn detect_prefix_format(head: &[u8]) -> Option<&'static str> {
    PREFIX_FORMATS
        .iter()
        .find_map(|(magic, format)| head.starts_with(magic).then_some(*format))
}

fn detect_webp(head: &[u8]) -> Option<&'static str> {
    (head.starts_with(WEBP_RIFF_MAGIC) && head.get(8..12) == Some(WEBP_FORMAT_MAGIC))
        .then_some("webp")
}

fn detect_jxl_container(head: &[u8]) -> Option<&'static str> {
    head.starts_with(JXL_CONTAINER_MAGIC).then_some("jxl")
}

/// Detect image format from file magic bytes.
pub fn detect_format(path: impl AsRef<Path>) -> Result<Option<&'static str>> {
    let head = read_magic(path)?;
    Ok(detect_format_bytes(&head))
}

pub fn load_image(path: impl AsRef<Path>) -> Result<ImageBuffer> {
    load_image_with_limits(
        path,
        crate::domain::image::MAX_IMAGE_DIMENSION,
        crate::domain::image::MAX_IMAGE_DIMENSION,
        crate::domain::image::MAX_IMAGE_BYTES as u64,
    )
}

/// Production decoder that satisfies the runtime [`ImageDecoder`] seam.
///
/// [`ImageDecoder`]: crate::infrastructure::runtime::ImageDecoder
pub struct NativeImageDecoder;

impl crate::infrastructure::runtime::ImageDecoder for NativeImageDecoder {
    fn decode(&self, path: &Path) -> Result<ImageBuffer> {
        load_image(path)
    }
}

/// Decode an image with explicit dimensions and allocation ceilings.
pub fn load_image_with_limits(
    path: impl AsRef<Path>,
    max_width: u32,
    max_height: u32,
    max_allocation: u64,
) -> Result<ImageBuffer> {
    let path = path.as_ref();
    if !path.is_file() {
        return Err(ScanError::Other(format!(
            "file not found: {}",
            path.display()
        )));
    }
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(max_width);
    limits.max_image_height = Some(max_height);
    limits.max_alloc = Some(max_allocation);
    reader.limits(limits);
    dynamic_to_buffer(reader.decode()?)
}

fn dynamic_to_buffer(dyn_img: DynamicImage) -> Result<ImageBuffer> {
    match dyn_img {
        DynamicImage::ImageLuma8(g) => {
            let (w, h) = g.dimensions();
            ImageBuffer::new(w, h, PixelFormat::Gray8, g.into_raw())
        }
        // Prefer packed Rgb8 for all non-luma sources (incl. RGBA).
        other => {
            let rgb = other.to_rgb8();
            let (w, h) = rgb.dimensions();
            ImageBuffer::new(w, h, PixelFormat::Rgb8, rgb.into_raw())
        }
    }
}

pub fn save_image(
    path: impl AsRef<Path>,
    image: &ImageBuffer,
    dpi: Option<u32>,
    quality: Option<u8>,
) -> Result<PathBuf> {
    save_image_with_cancellation(path, image, dpi, quality, None)
}

pub fn save_image_with_cancellation(
    path: impl AsRef<Path>,
    image: &ImageBuffer,
    dpi: Option<u32>,
    quality: Option<u8>,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    let path = path.as_ref();
    create_output_parent(path)?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| {
            ScanError::Invalid(format!(
                "output path has no extension; supported extensions: {}",
                supported_extensions().join(", ")
            ))
        })?
        .to_ascii_lowercase();
    save_by_extension(path, image, dpi, quality, cancellation, &ext)
}

fn save_by_extension(
    path: &Path,
    image: &ImageBuffer,
    dpi: Option<u32>,
    quality: Option<u8>,
    cancellation: Option<&CancellationToken>,
    ext: &str,
) -> Result<PathBuf> {
    if ext == "pdf" {
        return save_pdf_image(path, image, dpi, cancellation);
    }
    if ext == "jxl" {
        return save_jpeg_xl_with_cancellation(path, image, quality.unwrap_or(90), cancellation);
    }
    save_standard_image_output(path, image, quality, cancellation, ext)
}

fn save_pdf_image(
    path: &Path,
    image: &ImageBuffer,
    dpi: Option<u32>,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    save_pdf_with_options_and_cancellation(
        path,
        std::slice::from_ref(image),
        &PdfOptions {
            dpi: dpi.unwrap_or(150),
            ..PdfOptions::default()
        },
        cancellation,
    )
}

fn save_standard_image_output(
    path: &Path,
    image: &ImageBuffer,
    quality: Option<u8>,
    cancellation: Option<&CancellationToken>,
    ext: &str,
) -> Result<PathBuf> {
    validate_packed_buffer(image)?;
    let temp = create_output_temp(path)?;
    write_standard_image(image, ext, temp.path(), quality)?;
    validate_output_container(temp.path(), expected_container(ext))?;
    check_native_publication_cancellation(cancellation)?;
    temp.publish()?;
    Ok(path.to_path_buf())
}

fn write_standard_image(
    image: &ImageBuffer,
    extension: &str,
    output: &Path,
    quality: Option<u8>,
) -> Result<()> {
    validate_packed_buffer(image)?;
    if matches!(extension, "jpg" | "jpeg") {
        return write_jpeg(image, output, quality);
    }
    if extension == "bmp" {
        return bmp::write_bmp(image, output);
    }
    let format = standard_image_format(extension)?;
    image::save_buffer_with_format(
        output,
        &image.data,
        image.width,
        image.height,
        image_color_type(image.pixel_format),
        format,
    )
    .map_err(|error| ScanError::Image(error.to_string()))
}

pub(super) fn validate_packed_buffer(image: &ImageBuffer) -> Result<()> {
    let required = (image.width as usize)
        .checked_mul(image.height as usize)
        .and_then(|pixels| pixels.checked_mul(image.bpp()));
    if required.is_some_and(|required| required <= image.data.len()) {
        return Ok(());
    }
    let name = match image.pixel_format {
        PixelFormat::Gray8 => "gray",
        PixelFormat::Rgb8 => "rgb",
        PixelFormat::Rgba8 => "rgba",
    };
    Err(ScanError::Image(format!("{name} buffer")))
}

fn image_color_type(format: PixelFormat) -> ColorType {
    match format {
        PixelFormat::Gray8 => ColorType::L8,
        PixelFormat::Rgb8 => ColorType::Rgb8,
        PixelFormat::Rgba8 => ColorType::Rgba8,
    }
}

fn standard_image_format(extension: &str) -> Result<ImageFormat> {
    match extension {
        "tif" | "tiff" => Ok(ImageFormat::Tiff),
        "bmp" => Ok(ImageFormat::Bmp),
        "gif" => Ok(ImageFormat::Gif),
        "webp" => Ok(ImageFormat::WebP),
        "png" => Ok(ImageFormat::Png),
        _ => Err(ScanError::Invalid(format!(
            "unsupported extension '.{extension}'; use one of {}",
            supported_extensions().join(", ")
        ))),
    }
}

fn expected_container(extension: &str) -> &str {
    match extension {
        "jpg" | "jpeg" => "jpeg",
        "tif" | "tiff" => "tiff",
        extension => extension,
    }
}

fn save_jpeg_xl_with_cancellation(
    path: &Path,
    image: &ImageBuffer,
    quality: u8,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    save_jpeg_xl_with_program_and_cancellation(
        path,
        image,
        quality,
        Path::new("cjxl"),
        cancellation,
    )
}

pub(super) fn save_jpeg_xl_with_program_and_cancellation(
    path: &Path,
    image: &ImageBuffer,
    quality: u8,
    program: &Path,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    create_output_parent(path)?;
    let artifacts = TemporaryOutput::new("jpeg-xl", "jxl")?;
    let input_path = artifacts.directory().join("input.png");
    let encoded_path = artifacts.path();
    let output = create_output_temp(path)?;
    let artifact_limit = write_jxl_input(image, &input_path)?;
    run_jxl_encoder(
        program,
        &input_path,
        encoded_path,
        quality,
        artifacts.directory(),
        artifact_limit,
        cancellation,
    )?;
    publish_jxl_output(encoded_path, output, cancellation)?;
    Ok(path.to_path_buf())
}

fn write_jxl_input(image: &ImageBuffer, input_path: &Path) -> Result<u64> {
    write_standard_image(image, "png", input_path, None)?;
    std::fs::metadata(input_path)?
        .len()
        .checked_add(MAX_IMAGE_BYTES as u64)
        .ok_or_else(|| ScanError::Image("JPEG XL artifact quota overflow".into()))
}

fn run_jxl_encoder(
    program: &Path,
    input_path: &Path,
    encoded_path: &Path,
    quality: u8,
    artifact_directory: &Path,
    artifact_limit: u64,
    cancellation: Option<&CancellationToken>,
) -> Result<()> {
    let distance = jxl_distance(quality);
    let command_output = run_contained_command_with_artifact_quota(
        &CommandSpec {
            program: program.display().to_string(),
            args: vec![
                input_path.display().to_string(),
                encoded_path.display().to_string(),
                "--distance".into(),
                format!("{distance:.2}"),
            ],
        },
        Duration::from_secs(120),
        &Mutex::new(false),
        cancellation,
        "cjxl",
        "JPEG XL export cancelled",
        ArtifactWatch {
            directory: artifact_directory,
            quota: ArtifactQuota {
                max_files: 2,
                max_bytes: artifact_limit,
            },
        },
    )
    .map_err(|error| match error {
        ScanError::Unsupported(message) if message.contains("failed to start") => {
            ScanError::Other(format!(
                "JPEG XL output requires the optional 'cjxl' executable from libjxl: {message}"
            ))
        }
        other => other,
    })?;
    ensure_jxl_success(&command_output)
}

fn jxl_distance(quality: u8) -> f64 {
    ((100_u16.saturating_sub(u16::from(quality.min(100)))) as f64 / 15.0).clamp(0.0, 15.0)
}

fn ensure_jxl_success(
    command_output: &crate::infrastructure::runtime::CommandOutput,
) -> Result<()> {
    if !command_output.success {
        let detail = String::from_utf8_lossy(&command_output.stderr)
            .trim()
            .to_string();
        return Err(ScanError::Other(format!(
            "cjxl failed{}",
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        )));
    }
    Ok(())
}

fn publish_jxl_output(
    encoded_path: &Path,
    output: OutputTemp,
    cancellation: Option<&CancellationToken>,
) -> Result<()> {
    let output_len = std::fs::metadata(encoded_path)?.len();
    if output_len > MAX_IMAGE_BYTES as u64 {
        return Err(ScanError::Image(format!(
            "JPEG XL output exceeds the {MAX_IMAGE_BYTES} byte limit"
        )));
    }
    validate_output_container(encoded_path, "jxl")?;
    std::fs::copy(encoded_path, output.path())?;
    validate_output_container(output.path(), "jxl")?;
    check_native_publication_cancellation(cancellation)?;
    output.publish()
}

fn check_native_publication_cancellation(cancellation: Option<&CancellationToken>) -> Result<()> {
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err(ScanError::Cancelled("image publication cancelled".into()));
    }
    Ok(())
}

fn create_output_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

/// Confirm an encoder wrote a nonempty file with the requested container header.
pub(super) fn validate_output_container(path: &Path, expected: &str) -> Result<()> {
    let meta = std::fs::metadata(path)?;
    if meta.len() == 0 {
        return Err(ScanError::Image("write produced empty file".into()));
    }
    let actual = detect_format(path)?;
    if actual != Some(expected) {
        return Err(ScanError::Image(format!(
            "writer produced invalid {expected} header"
        )));
    }
    Ok(())
}

pub(super) fn buffer_to_dynamic(image: &ImageBuffer) -> Result<DynamicImage> {
    match image.pixel_format {
        PixelFormat::Gray8 => {
            let buf = ImgBuf::<image::Luma<u8>, _>::from_raw(
                image.width,
                image.height,
                image.data.clone(),
            )
            .ok_or_else(|| ScanError::Image("gray buffer".into()))?;
            Ok(DynamicImage::ImageLuma8(buf))
        }
        PixelFormat::Rgb8 => {
            let buf = ImgBuf::<Rgb<u8>, _>::from_raw(image.width, image.height, image.data.clone())
                .ok_or_else(|| ScanError::Image("rgb buffer".into()))?;
            Ok(DynamicImage::ImageRgb8(buf))
        }
        PixelFormat::Rgba8 => {
            let buf =
                ImgBuf::<Rgba<u8>, _>::from_raw(image.width, image.height, image.data.clone())
                    .ok_or_else(|| ScanError::Image("rgba buffer".into()))?;
            Ok(DynamicImage::ImageRgba8(buf))
        }
    }
}

pub fn convert_image(
    inp: impl AsRef<Path>,
    out: impl AsRef<Path>,
    dpi: Option<u32>,
) -> Result<PathBuf> {
    let img = load_image(inp)?;
    save_image(out, &img, dpi, None)
}

/// Convert buffer to tightly packed RGB bytes without copying the input pixels.
pub fn to_rgb_bytes(image: &ImageBuffer) -> Result<Vec<u8>> {
    use image::buffer::ConvertBuffer;
    validate_packed_buffer(image)?;
    let converted: image::RgbImage = match image.pixel_format {
        PixelFormat::Rgb8 => return Ok(image.data.clone()),
        PixelFormat::Gray8 => {
            ImgBuf::<image::Luma<u8>, _>::from_raw(image.width, image.height, image.data.as_slice())
                .expect("validated gray buffer")
                .convert()
        }
        PixelFormat::Rgba8 => {
            ImgBuf::<Rgba<u8>, _>::from_raw(image.width, image.height, image.data.as_slice())
                .expect("validated RGBA buffer")
                .convert()
        }
    };
    Ok(converted.into_raw())
}
