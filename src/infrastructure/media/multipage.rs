use super::codecs::buffer_to_dynamic;
use super::{load_image, save_image_with_cancellation};
use crate::domain::image::{ImageBuffer, PixelFormat};
use crate::error::{Result, ScanError};
use crate::workflows::operation::CancellationToken;
use image::Rgb;
use std::path::{Path, PathBuf};

/// Build a white-backed thumbnail grid in row-major order.
pub fn save_index_contact_sheet(
    images: &[PathBuf],
    path: impl AsRef<Path>,
    across: u32,
    thumb_width: u32,
    thumb_height: Option<u32>,
    margin: u32,
) -> Result<PathBuf> {
    save_index_contact_sheet_with_cancellation(
        images,
        path,
        across,
        thumb_width,
        thumb_height,
        margin,
        None,
    )
}

/// Build a contact sheet while checking cancellation before every source page
/// and immediately before its atomic image publication.
pub fn save_index_contact_sheet_with_cancellation(
    images: &[PathBuf],
    path: impl AsRef<Path>,
    across: u32,
    thumb_width: u32,
    thumb_height: Option<u32>,
    margin: u32,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    if images.is_empty() {
        return Err(ScanError::Invalid(
            "save_index_contact_sheet requires at least one image".into(),
        ));
    }
    check_contact_sheet_cancellation(cancellation)?;
    let cols = across.max(1);
    let tw = thumb_width.max(8);
    let th = thumb_height.unwrap_or_else(|| (tw * 3 / 4).max(8)).max(8);
    let image_count = u32::try_from(images.len())
        .map_err(|_| ScanError::Invalid("contact sheet has too many images".into()))?;
    let rows = image_count.div_ceil(cols);
    let width = cols
        .checked_mul(tw)
        .and_then(|value| value.checked_add(cols.checked_add(1)?.checked_mul(margin)?))
        .ok_or_else(|| ScanError::Invalid("contact sheet dimensions overflow".into()))?;
    let height = rows
        .checked_mul(th)
        .and_then(|value| value.checked_add(rows.checked_add(1)?.checked_mul(margin)?))
        .ok_or_else(|| ScanError::Invalid("contact sheet dimensions overflow".into()))?;
    crate::domain::image::checked_image_len(width, height, PixelFormat::Rgb8.bpp())?;
    let mut sheet = image::RgbImage::from_pixel(width, height, Rgb([255, 255, 255]));
    for (index, path) in images.iter().enumerate() {
        check_contact_sheet_cancellation(cancellation)?;
        let source = buffer_to_dynamic(&load_image(path)?)?.to_rgb8();
        let thumb = image::imageops::resize(&source, tw, th, image::imageops::FilterType::Triangle);
        check_contact_sheet_cancellation(cancellation)?;
        let col = index as u32 % cols;
        let row = index as u32 / cols;
        image::imageops::replace(
            &mut sheet,
            &thumb,
            i64::from(margin + col * (tw + margin)),
            i64::from(margin + row * (th + margin)),
        );
    }
    let mut out = path.as_ref().to_path_buf();
    if out.extension().is_none() {
        out.set_extension("bmp");
    }
    let buffer = ImageBuffer::new(width, height, PixelFormat::Rgb8, sheet.into_raw())?;
    save_image_with_cancellation(out, &buffer, None, None, cancellation)
}

fn check_contact_sheet_cancellation(cancellation: Option<&CancellationToken>) -> Result<()> {
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err(ScanError::Cancelled(
            "contact sheet publication cancelled".into(),
        ));
    }
    Ok(())
}

/// Save the unprocessed buffer, defaulting to TIFF when no extension is provided.
pub fn save_raw_image(
    path: impl AsRef<Path>,
    image: &ImageBuffer,
    dpi: Option<u32>,
) -> Result<PathBuf> {
    save_raw_image_with_cancellation(path, image, dpi, None)
}

/// Save the unprocessed buffer with cooperative cancellation for external
/// encoders, defaulting to TIFF when no extension is provided.
pub(crate) fn save_raw_image_with_cancellation(
    path: impl AsRef<Path>,
    image: &ImageBuffer,
    dpi: Option<u32>,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    let mut out = path.as_ref().to_path_buf();
    if out.extension().is_none() {
        out.set_extension("tif");
    }
    save_image_with_cancellation(out, image, dpi, None, cancellation)
}
