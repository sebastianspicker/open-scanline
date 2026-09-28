use super::{checked_tiff_frame_aggregate, write_multipage_tiff_rgb};
use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use crate::infrastructure::media::codecs::buffer_to_dynamic;
use crate::infrastructure::media::{save_image, save_pdf_with_options, PdfOptions};
use std::path::{Path, PathBuf};

/// Append a TIFF page, or replace a PDF destination with a single-page document.
pub fn append_page_to_multipage(
    path: impl AsRef<Path>,
    image: &ImageBuffer,
    format: Option<&str>,
    dpi: Option<u32>,
) -> Result<PathBuf> {
    let mut out = path.as_ref().to_path_buf();
    let ext = append_format(format, &out);
    match ext.as_str() {
        "tif" | "tiff" => append_tiff_page(&mut out, image, dpi),
        "pdf" => append_pdf_page(&mut out, image, dpi),
        _ => Err(ScanError::Invalid(format!(
            "append_page_to_multipage supports TIFF or PDF, not '{ext}'"
        ))),
    }
}

fn append_format(format: Option<&str>, out: &Path) -> String {
    format
        .map(|value| value.trim().trim_start_matches('.').to_ascii_lowercase())
        .or_else(|| {
            out.extension()
                .and_then(|value| value.to_str())
                .map(str::to_ascii_lowercase)
        })
        .unwrap_or_else(|| "tif".into())
}

fn append_tiff_page(out: &mut PathBuf, image: &ImageBuffer, dpi: Option<u32>) -> Result<PathBuf> {
    if out.extension().is_none() {
        out.set_extension("tif");
    }
    if !out.is_file() {
        return save_image(out, image, dpi, None);
    }
    let mut frames = load_tiff_frames(out)?;
    let bytes = aggregate_frame_bytes(&frames)?;
    checked_tiff_frame_aggregate(bytes, image.width, image.height)?;
    frames.push(buffer_to_dynamic(image)?.to_rgb8());
    write_multipage_tiff_rgb(&frames, out, dpi)?;
    Ok(out.clone())
}

fn aggregate_frame_bytes(frames: &[image::RgbImage]) -> Result<usize> {
    frames.iter().try_fold(0usize, |total, frame| {
        total.checked_add(frame.as_raw().len()).ok_or_else(|| {
            ScanError::Image("TIFF decoded frame aggregate exceeds the image safety limit".into())
        })
    })
}

fn append_pdf_page(out: &mut PathBuf, image: &ImageBuffer, dpi: Option<u32>) -> Result<PathBuf> {
    if out.extension().is_none() {
        out.set_extension("pdf");
    }
    save_pdf_with_options(
        out,
        std::slice::from_ref(image),
        &PdfOptions {
            dpi: dpi.unwrap_or(150),
            ..PdfOptions::default()
        },
    )
}

pub(super) fn load_tiff_frames(path: &Path) -> Result<Vec<image::RgbImage>> {
    use tiff::decoder::Decoder;

    let file = std::fs::File::open(path)?;
    let mut decoder = Decoder::new(std::io::BufReader::new(file))
        .map_err(|e| ScanError::Image(format!("TIFF decode failed: {e}")))?;
    let mut frames = Vec::new();
    let mut aggregate_rgb_bytes = 0usize;
    loop {
        let (rgb, _bytes) = read_current_tiff_frame(&mut decoder)?;
        aggregate_rgb_bytes =
            checked_tiff_frame_aggregate(aggregate_rgb_bytes, rgb.width(), rgb.height())?;
        frames.push(rgb);
        if !decoder.more_images() {
            break;
        }
        decoder
            .next_image()
            .map_err(|e| ScanError::Image(format!("TIFF next frame failed: {e}")))?;
    }
    Ok(frames)
}

fn read_current_tiff_frame(
    decoder: &mut tiff::decoder::Decoder<std::io::BufReader<std::fs::File>>,
) -> Result<(image::RgbImage, usize)> {
    use tiff::decoder::DecodingResult;
    let (width, height) = decoder
        .dimensions()
        .map_err(|error| ScanError::Image(format!("TIFF dimensions failed: {error}")))?;
    let color = decoder
        .colortype()
        .map_err(|error| ScanError::Image(format!("TIFF color type failed: {error}")))?;
    validate_append_tiff_color(color)?;
    let bytes = decoder
        .read_image()
        .map_err(|error| ScanError::Image(format!("TIFF frame decode failed: {error}")))?;
    let bytes = match bytes {
        DecodingResult::U8(bytes) => bytes,
        _ => return append_tiff_color_error(),
    };
    let byte_len = bytes.len();
    Ok((
        decode_append_tiff_frame(color, width, height, bytes)?,
        byte_len,
    ))
}

fn validate_append_tiff_color(color: tiff::ColorType) -> Result<()> {
    if matches!(color, tiff::ColorType::RGB(8) | tiff::ColorType::Gray(8)) {
        Ok(())
    } else {
        append_tiff_color_error()
    }
}

fn append_tiff_color_error<T>() -> Result<T> {
    Err(ScanError::Image(
        "TIFF append supports 8-bit grayscale or RGB pages".into(),
    ))
}

fn decode_append_tiff_frame(
    color: tiff::ColorType,
    width: u32,
    height: u32,
    bytes: Vec<u8>,
) -> Result<image::RgbImage> {
    let data = if matches!(color, tiff::ColorType::Gray(8)) {
        bytes
            .into_iter()
            .flat_map(|value| [value, value, value])
            .collect()
    } else {
        bytes
    };
    image::RgbImage::from_raw(width, height, data)
        .ok_or_else(|| ScanError::Image("invalid TIFF frame buffer".into()))
}
