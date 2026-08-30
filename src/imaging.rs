//! Compatibility facade for the historical imaging API.

pub use crate::infrastructure::media::{
    append_page_to_multipage, convert_image, convert_image_with_cancellation, detect_format,
    detect_format_bytes, image_buffer_to_rgba, is_png_magic, load_image, load_image_with_limits,
    read_magic, save_image, save_image_with_cancellation, save_index_contact_sheet,
    save_index_contact_sheet_with_cancellation, save_multipage_pdf,
    save_multipage_pdf_with_cancellation, save_multipage_tiff,
    save_multipage_tiff_from_paths_with_transform,
    save_multipage_tiff_from_paths_with_transform_and_cancellation,
    save_multipage_tiff_with_cancellation, save_pdf_from_paths_with_options,
    save_pdf_from_paths_with_options_and_cancellation,
    save_pdf_from_paths_with_options_and_transform,
    save_pdf_from_paths_with_options_and_transform_and_cancellation, save_pdf_with_options,
    save_pdf_with_options_and_cancellation, save_raw_image, supported_extensions, to_rgb_bytes,
    validate_png_magic, PdfOptions, PNG_MAGIC,
};
