//! Image encode/load/save (OSL-IMAGING) via the `image` crate.

mod codecs;
mod multipage;
mod pdf;
mod preview;
mod tiff;

pub use codecs::{
    convert_image, detect_format, detect_format_bytes, is_png_magic, load_image,
    load_image_with_limits, read_magic, save_image, save_image_with_cancellation,
    supported_extensions, to_rgb_bytes, validate_png_magic, PNG_MAGIC,
};

use crate::core::Result;
use crate::device::CancellationToken;
use std::path::{Path, PathBuf};

/// Convert an image while forwarding cancellation to command-backed encoders.
pub fn convert_image_with_cancellation(
    inp: impl AsRef<Path>,
    out: impl AsRef<Path>,
    dpi: Option<u32>,
    cancellation: CancellationToken,
) -> Result<PathBuf> {
    let image = load_image(inp)?;
    save_image_with_cancellation(out, &image, dpi, None, Some(&cancellation))
}
pub(crate) use multipage::save_raw_image_with_cancellation;
pub use multipage::{
    save_index_contact_sheet, save_index_contact_sheet_with_cancellation, save_raw_image,
};
#[cfg(test)]
pub(crate) use pdf::MAX_PDF_SEARCHABLE_PAGE_UTF8_BYTES;
pub(crate) use pdf::{checked_pdf_searchable_text_total, validate_pdf_password};
pub use pdf::{
    save_multipage_pdf, save_multipage_pdf_with_cancellation, save_pdf_from_paths_with_options,
    save_pdf_from_paths_with_options_and_cancellation,
    save_pdf_from_paths_with_options_and_transform,
    save_pdf_from_paths_with_options_and_transform_and_cancellation, save_pdf_with_options,
    save_pdf_with_options_and_cancellation, PdfOptions,
};
pub use preview::image_buffer_to_rgba;
pub use tiff::{
    append_page_to_multipage, save_multipage_tiff, save_multipage_tiff_from_paths_with_transform,
    save_multipage_tiff_from_paths_with_transform_and_cancellation,
    save_multipage_tiff_with_cancellation,
};

#[cfg(test)]
mod tests;
