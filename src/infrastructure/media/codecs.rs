use super::pdf::{save_pdf_with_options_and_cancellation, PdfOptions};
use crate::domain::image::{ImageBuffer, PixelFormat, MAX_IMAGE_BYTES};
use crate::error::{Result, ScanError};
use crate::infrastructure::runtime::{
    run_contained_command_with_artifact_quota, ArtifactQuota, ArtifactWatch, CommandSpec,
    TemporaryOutput,
};
use crate::workflows::operation::CancellationToken;
use image::{DynamicImage, ImageBuffer as ImgBuf, ImageFormat, Rgb, Rgba};
use std::fs::OpenOptions;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

/// PNG file signature (8 bytes).
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
const TEMP_CREATE_ATTEMPTS: u64 = 128;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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

/// Save an image while allowing command-backed encoders to observe cancellation.
///
/// The existing [`save_image`] API remains a source-compatible wrapper for
/// callers that do not own a cancellation token.
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
    // Route PDF through the structured writer.
    if ext == "pdf" {
        return save_pdf_with_options_and_cancellation(
            path,
            std::slice::from_ref(image),
            &PdfOptions {
                dpi: dpi.unwrap_or(150),
                ..PdfOptions::default()
            },
            cancellation,
        );
    }
    if ext == "jxl" {
        return save_jpeg_xl_with_cancellation(path, image, quality.unwrap_or(90), cancellation);
    }
    let dyn_img = buffer_to_dynamic(image)?;
    let temp = create_output_temp(path)?;
    match ext.as_str() {
        "jpg" | "jpeg" => {
            let q = quality.unwrap_or(90);
            let mut f = OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(temp.path())?;
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut f, q);
            enc.encode_image(&dyn_img)
                .map_err(|e| ScanError::Image(e.to_string()))?;
            f.flush()?;
        }
        "tif" | "tiff" => {
            dyn_img
                .save_with_format(temp.path(), ImageFormat::Tiff)
                .map_err(|e| ScanError::Image(e.to_string()))?;
        }
        "bmp" => {
            dyn_img
                .save_with_format(temp.path(), ImageFormat::Bmp)
                .map_err(|e| ScanError::Image(e.to_string()))?;
        }
        "gif" => {
            dyn_img
                .save_with_format(temp.path(), ImageFormat::Gif)
                .map_err(|e| ScanError::Image(e.to_string()))?;
        }
        "webp" => {
            dyn_img
                .save_with_format(temp.path(), ImageFormat::WebP)
                .map_err(|e| ScanError::Image(e.to_string()))?;
        }
        "png" => {
            dyn_img
                .save_with_format(temp.path(), ImageFormat::Png)
                .map_err(|e| ScanError::Image(e.to_string()))?;
        }
        _ => {
            return Err(ScanError::Invalid(format!(
                "unsupported extension '.{ext}'; use one of {}",
                supported_extensions().join(", ")
            )));
        }
    }
    validate_output_container(temp.path(), expected_container(&ext))?;
    check_native_publication_cancellation(cancellation)?;
    temp.publish()?;
    Ok(path.to_path_buf())
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
    buffer_to_dynamic(image)?
        .save_with_format(&input_path, ImageFormat::Png)
        .map_err(|e| ScanError::Image(e.to_string()))?;
    let input_bytes = std::fs::metadata(&input_path)?.len();
    let artifact_limit = input_bytes
        .checked_add(MAX_IMAGE_BYTES as u64)
        .ok_or_else(|| ScanError::Image("JPEG XL artifact quota overflow".into()))?;
    let distance =
        ((100_u16.saturating_sub(u16::from(quality.min(100)))) as f64 / 15.0).clamp(0.0, 15.0);
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
            directory: artifacts.directory(),
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
    output.publish()?;
    Ok(path.to_path_buf())
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

/// A create-new sibling temporary that is removed unless it is published.
pub(super) struct OutputTemp {
    path: PathBuf,
    destination: PathBuf,
    published: bool,
}

impl OutputTemp {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn publish(mut self) -> Result<()> {
        crate::infrastructure::runtime::atomic_publish::replace_file_atomic(
            &self.path,
            &self.destination,
        )?;
        self.published = true;
        Ok(())
    }
}

impl Drop for OutputTemp {
    fn drop(&mut self) {
        if !self.published {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Reserve a unique sibling temporary with the destination's effective extension.
pub(super) fn create_output_temp(destination: &Path) -> Result<OutputTemp> {
    create_output_parent(destination)?;
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let file_name = destination.file_name().ok_or_else(|| {
        ScanError::Invalid(format!(
            "output path has no file name: {}",
            destination.display()
        ))
    })?;
    let extension = destination
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = destination
        .file_stem()
        .unwrap_or(file_name)
        .to_string_lossy();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    #[cfg(unix)]
    let output_mode = existing_output_mode_or_private(destination)?;

    for _ in 0..TEMP_CREATE_ATTEMPTS {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let suffix = if extension.is_empty() {
            String::new()
        } else {
            format!(".{extension}")
        };
        let path = parent.join(format!(
            ".{stem}.open-scanline-{}-{timestamp}-{sequence}{suffix}",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => {
                #[cfg(unix)]
                if let Err(error) =
                    file.set_permissions(std::fs::Permissions::from_mode(output_mode))
                {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    return Err(error.into());
                }
                #[cfg(not(unix))]
                drop(file);
                return Ok(OutputTemp {
                    path,
                    destination: destination.to_path_buf(),
                    published: false,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }

    Err(ScanError::Other(format!(
        "could not reserve a unique temporary output beside {}",
        destination.display()
    )))
}

#[cfg(unix)]
fn existing_output_mode_or_private(destination: &Path) -> Result<u32> {
    use std::os::unix::fs::PermissionsExt;

    match std::fs::metadata(destination) {
        Ok(metadata) => Ok(metadata.permissions().mode() & 0o7777),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0o600),
        Err(error) => Err(error.into()),
    }
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

/// Convert buffer to tightly packed RGB bytes.
pub fn to_rgb_bytes(image: &ImageBuffer) -> Result<Vec<u8>> {
    Ok(buffer_to_dynamic(image)?.to_rgb8().into_raw())
}
