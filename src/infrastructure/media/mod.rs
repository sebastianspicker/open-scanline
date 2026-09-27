//! Native image, profile, OCR, and document-media integrations.

pub(crate) mod aggregate_cache;
mod codecs;
pub mod icc;
mod multipage;
pub mod ocr;
mod pdf;
mod pixels;
mod preview;
mod tiff;

pub use codecs::{
    convert_image, detect_format, detect_format_bytes, is_png_magic, load_image,
    load_image_with_limits, read_magic, save_image, save_image_with_cancellation,
    supported_extensions, to_rgb_bytes, validate_png_magic, NativeImageDecoder, PNG_MAGIC,
};
pub use icc::*;
pub(crate) use multipage::save_raw_image_with_cancellation;
pub use multipage::{
    save_index_contact_sheet, save_index_contact_sheet_with_cancellation, save_raw_image,
};
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

/// Convert an image while forwarding cancellation to command-backed encoders.
pub fn convert_image_with_cancellation(
    input: impl AsRef<std::path::Path>,
    output: impl AsRef<std::path::Path>,
    dpi: Option<u32>,
    cancellation: crate::operation::CancellationToken,
) -> crate::error::Result<std::path::PathBuf> {
    let image = load_image(input)?;
    save_image_with_cancellation(output, &image, dpi, None, Some(&cancellation))
}

/// Internal request for assembling a PDF from already-published pages.
pub(crate) struct PdfPathPublication<'a> {
    pub(crate) paths: &'a [std::path::PathBuf],
    pub(crate) destination: &'a std::path::Path,
    pub(crate) dpi: u32,
    pub(crate) title: &'a str,
    pub(crate) password: Option<&'a str>,
    pub(crate) searchable_pages: Option<Vec<String>>,
    pub(crate) transform: Option<&'a icc::PreparedScannerProfile>,
    pub(crate) cancellation: Option<&'a crate::operation::CancellationToken>,
}

/// Prepare a validated output directory before incremental publication.
pub(crate) fn prepare_output_directory(directory: &std::path::Path) -> crate::error::Result<()> {
    std::fs::create_dir_all(directory)?;
    Ok(())
}

/// Assemble a PDF from already-published pages, optionally re-decoding and
/// profile-transforming each page as it loads.
pub(crate) fn publish_pdf_from_paths(
    request: PdfPathPublication<'_>,
) -> crate::error::Result<std::path::PathBuf> {
    let PdfPathPublication {
        paths,
        destination,
        dpi,
        title,
        password,
        searchable_pages,
        transform,
        cancellation,
    } = request;
    let options = PdfOptions {
        dpi,
        title: title.into(),
        password: password.map(str::to_owned),
        searchable_pages,
    };
    match transform {
        Some(transform) => pdf::save_pdf_from_paths_with_loader_and_transform(
            paths,
            destination,
            &options,
            |path| load_image(path),
            |image| transform.apply_owned(image),
            cancellation,
        ),
        None => save_pdf_from_paths_with_options_and_cancellation(
            paths,
            destination,
            &options,
            cancellation,
        ),
    }
}

/// Assemble TIFF pages, optionally applying a scanner profile as pages load.
pub(crate) fn publish_tiff_from_paths(
    paths: &[std::path::PathBuf],
    destination: &std::path::Path,
    dpi: Option<u32>,
    transform: Option<&icc::PreparedScannerProfile>,
    cancellation: Option<&crate::operation::CancellationToken>,
) -> crate::error::Result<std::path::PathBuf> {
    match transform {
        Some(transform) => tiff::save_multipage_tiff_with_loader_and_transform(
            paths,
            destination,
            dpi,
            |path| load_image(path),
            |image| transform.apply_owned(image),
            cancellation,
        ),
        None => save_multipage_tiff_with_cancellation(paths, destination, dpi, cancellation),
    }
}

/// Private document-publication session used when a batch requests several
/// aggregate derivatives from the same already-published pages.
pub(crate) struct NativeAggregateSession {
    pub(crate) loader: aggregate_cache::SharedPageLoader,
}

impl NativeAggregateSession {
    pub(crate) fn new() -> Self {
        Self {
            loader: aggregate_cache::SharedPageLoader::new(),
        }
    }

    pub(crate) fn publish_pdf(
        &mut self,
        request: PdfPathPublication<'_>,
    ) -> crate::error::Result<std::path::PathBuf> {
        let PdfPathPublication {
            paths,
            destination,
            dpi,
            title,
            password,
            searchable_pages,
            transform,
            cancellation,
        } = request;
        let options = PdfOptions {
            dpi,
            title: title.into(),
            password: password.map(str::to_owned),
            searchable_pages,
        };
        pdf::save_pdf_from_paths_with_loader_and_transform(
            paths,
            destination,
            &options,
            |path| self.loader.load(path, cancellation),
            |image| match transform {
                Some(transform) => transform.apply_owned(image),
                None => Ok(image),
            },
            cancellation,
        )
    }

    pub(crate) fn publish_tiff(
        &mut self,
        paths: &[std::path::PathBuf],
        destination: &std::path::Path,
        dpi: Option<u32>,
        transform: Option<&icc::PreparedScannerProfile>,
        cancellation: Option<&crate::operation::CancellationToken>,
    ) -> crate::error::Result<std::path::PathBuf> {
        tiff::save_multipage_tiff_with_loader_and_transform(
            paths,
            destination,
            dpi,
            |path| self.loader.load(path, cancellation),
            |image| match transform {
                Some(transform) => transform.apply_owned(image),
                None => Ok(image),
            },
            cancellation,
        )
    }

    pub(crate) fn publish_contact_sheet(
        &mut self,
        paths: &[std::path::PathBuf],
        destination: &std::path::Path,
        cancellation: Option<&crate::operation::CancellationToken>,
    ) -> crate::error::Result<std::path::PathBuf> {
        multipage::save_index_contact_sheet_with_loader(
            paths,
            destination,
            4,
            160,
            None,
            4,
            |path| self.loader.load(path, cancellation),
            cancellation,
        )
    }
}
