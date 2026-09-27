use super::{load_image, save_image_with_cancellation};
use crate::domain::image::{ImageBuffer, PixelFormat};
use crate::error::{Result, ScanError};
use crate::operation::CancellationToken;
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
    save_index_contact_sheet_with_loader(
        images,
        path.as_ref(),
        across,
        thumb_width,
        thumb_height,
        margin,
        |source| load_image(source),
        cancellation,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn save_index_contact_sheet_with_loader<L>(
    images: &[PathBuf],
    path: &Path,
    across: u32,
    thumb_width: u32,
    thumb_height: Option<u32>,
    margin: u32,
    mut load: L,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf>
where
    L: FnMut(&Path) -> Result<ImageBuffer>,
{
    if images.is_empty() {
        return Err(ScanError::Invalid(
            "save_index_contact_sheet requires at least one image".into(),
        ));
    }
    check_contact_sheet_cancellation(cancellation)?;
    let layout = ContactSheetLayout::new(images.len(), across, thumb_width, thumb_height, margin)?;
    let mut sheet = image::RgbImage::from_pixel(layout.width, layout.height, Rgb([255, 255, 255]));
    layout.paste_thumbnails(&mut sheet, images, &mut load, cancellation)?;
    let mut out = path.to_path_buf();
    if out.extension().is_none() {
        out.set_extension("bmp");
    }
    let buffer = ImageBuffer::new(
        layout.width,
        layout.height,
        PixelFormat::Rgb8,
        sheet.into_raw(),
    )?;
    save_image_with_cancellation(out, &buffer, None, None, cancellation)
}

struct ContactSheetLayout {
    cols: u32,
    thumb_width: u32,
    thumb_height: u32,
    margin: u32,
    width: u32,
    height: u32,
}

impl ContactSheetLayout {
    fn new(
        image_count: usize,
        across: u32,
        thumb_width: u32,
        thumb_height: Option<u32>,
        margin: u32,
    ) -> Result<Self> {
        let cols = across.max(1);
        let thumb_width = thumb_width.max(8);
        let thumb_height = thumb_height
            .unwrap_or_else(|| (thumb_width * 3 / 4).max(8))
            .max(8);
        let rows = u32::try_from(image_count)
            .map_err(|_| ScanError::Invalid("contact sheet has too many images".into()))?
            .div_ceil(cols);
        let width = checked_contact_sheet_dimension(cols, thumb_width, margin)?;
        let height = checked_contact_sheet_dimension(rows, thumb_height, margin)?;
        crate::domain::image::checked_image_len(width, height, PixelFormat::Rgb8.bpp())?;
        Ok(Self {
            cols,
            thumb_width,
            thumb_height,
            margin,
            width,
            height,
        })
    }

    fn paste_thumbnails<L>(
        &self,
        sheet: &mut image::RgbImage,
        images: &[PathBuf],
        load: &mut L,
        cancellation: Option<&CancellationToken>,
    ) -> Result<()>
    where
        L: FnMut(&Path) -> Result<ImageBuffer>,
    {
        for (index, path) in images.iter().enumerate() {
            check_contact_sheet_cancellation(cancellation)?;
            let source = super::pixels::into_rgb_image(load(path)?, "contact-sheet")?;
            let thumb = image::imageops::resize(
                &source,
                self.thumb_width,
                self.thumb_height,
                image::imageops::FilterType::Triangle,
            );
            check_contact_sheet_cancellation(cancellation)?;
            let (column, row) = (index as u32 % self.cols, index as u32 / self.cols);
            image::imageops::replace(sheet, &thumb, self.x(column), self.y(row));
        }
        Ok(())
    }

    fn x(&self, column: u32) -> i64 {
        i64::from(self.margin + column * (self.thumb_width + self.margin))
    }
    fn y(&self, row: u32) -> i64 {
        i64::from(self.margin + row * (self.thumb_height + self.margin))
    }
}

fn checked_contact_sheet_dimension(cells: u32, thumbnail: u32, margin: u32) -> Result<u32> {
    cells
        .checked_mul(thumbnail)
        .and_then(|value| value.checked_add(cells.checked_add(1)?.checked_mul(margin)?))
        .ok_or_else(|| ScanError::Invalid("contact sheet dimensions overflow".into()))
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
