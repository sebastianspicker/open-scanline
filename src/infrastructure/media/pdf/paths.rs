use super::check_pdf_cancellation;
use super::document::{
    create_pdf_document, create_pdf_parent, finish_pdf_document, validate_pdf_options, PdfOptions,
};
use super::page::{PdfPageBuilder, PdfPageContext};
use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use crate::infrastructure::media::load_image;
use crate::workflows::operation::CancellationToken;
use std::path::{Path, PathBuf};

/// Combine on-disk page images into one multipage PDF.
pub fn save_multipage_pdf(
    pages: &[PathBuf],
    out: impl AsRef<Path>,
    options: &PdfOptions,
) -> Result<PathBuf> {
    save_pdf_from_paths_with_options(pages, out, options)
}

/// Combine page images while allowing cancellation before each page and publication.
pub fn save_multipage_pdf_with_cancellation(
    pages: &[PathBuf],
    out: impl AsRef<Path>,
    options: &PdfOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    save_pdf_from_paths_with_options_and_cancellation(pages, out, options, cancellation)
}

/// Build a PDF while loading only one raw page image at a time. The PDF
/// document retains its compressed image streams, but large decoded page
/// buffers are released before the next page is read.
pub fn save_pdf_from_paths_with_options(
    pages: &[PathBuf],
    out: impl AsRef<Path>,
    options: &PdfOptions,
) -> Result<PathBuf> {
    save_pdf_from_paths_with_options_and_cancellation(pages, out, options, None)
}

/// Build a PDF from page paths while allowing cancellation before each page
/// and immediately before atomic publication.
pub fn save_pdf_from_paths_with_options_and_cancellation(
    pages: &[PathBuf],
    out: impl AsRef<Path>,
    options: &PdfOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    save_pdf_from_paths_with_options_and_transform_and_cancellation(
        pages,
        out,
        options,
        |image| Ok(image.clone()),
        cancellation,
    )
}

/// Build a PDF from paths while transforming one decoded page at a time.
///
/// This keeps the decoded source-page lifetime bounded to the current page;
/// the PDF document still owns its encoded page streams until publication.
pub fn save_pdf_from_paths_with_options_and_transform<F>(
    pages: &[PathBuf],
    out: impl AsRef<Path>,
    options: &PdfOptions,
    transform: F,
) -> Result<PathBuf>
where
    F: FnMut(&ImageBuffer) -> Result<ImageBuffer>,
{
    save_pdf_from_paths_with_options_and_transform_and_cancellation(
        pages, out, options, transform, None,
    )
}

/// Build a transformed PDF while allowing cancellation before every page,
/// transform, object append, and atomic publication.
pub fn save_pdf_from_paths_with_options_and_transform_and_cancellation<F>(
    pages: &[PathBuf],
    out: impl AsRef<Path>,
    options: &PdfOptions,
    mut transform: F,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf>
where
    F: FnMut(&ImageBuffer) -> Result<ImageBuffer>,
{
    save_pdf_from_paths_with_loader_and_transform(
        pages,
        out.as_ref(),
        options,
        |path| load_image(path),
        |image| transform(&image),
        cancellation,
    )
}

/// Internal owned-page path used by the batch aggregate decode cache.
pub(crate) fn save_pdf_from_paths_with_loader_and_transform<L, F>(
    pages: &[PathBuf],
    out: &Path,
    options: &PdfOptions,
    mut load: L,
    mut transform: F,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf>
where
    L: FnMut(&Path) -> Result<ImageBuffer>,
    F: FnMut(ImageBuffer) -> Result<ImageBuffer>,
{
    validate_path_pdf_request(pages, options, cancellation)?;
    create_pdf_parent(out)?;
    let (mut document, pages_id, searchable_font) = create_pdf_document(options, cancellation)?;
    let mut page_ids = Vec::with_capacity(pages.len());
    {
        let mut builder = PdfPageBuilder {
            document: &mut document,
            context: PdfPageContext {
                pages_id,
                searchable_font,
                dpi: options.dpi.max(1) as f64,
            },
            retained_image_stream_bytes: 0,
        };
        append_path_pages(
            &mut builder,
            pages,
            options,
            &mut load,
            &mut transform,
            cancellation,
            &mut page_ids,
        )?;
    }
    finish_pdf_document(
        &mut document,
        pages_id,
        &page_ids,
        out,
        options,
        cancellation,
    )?;
    Ok(out.to_path_buf())
}

fn validate_path_pdf_request(
    pages: &[PathBuf],
    options: &PdfOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<()> {
    if pages.is_empty() {
        return Err(ScanError::Invalid(
            "save_multipage_pdf requires at least one page".into(),
        ));
    }
    validate_pdf_options(pages.len(), options)?;
    check_pdf_cancellation(cancellation)
}

fn append_path_pages<L, F>(
    builder: &mut PdfPageBuilder<'_>,
    pages: &[PathBuf],
    options: &PdfOptions,
    load: &mut L,
    transform: &mut F,
    cancellation: Option<&CancellationToken>,
    page_ids: &mut Vec<lopdf::ObjectId>,
) -> Result<()>
where
    L: FnMut(&Path) -> Result<ImageBuffer>,
    F: FnMut(ImageBuffer) -> Result<ImageBuffer>,
{
    for (index, page) in pages.iter().enumerate() {
        let image = transformed_page(page, load, transform, cancellation)?;
        let text = options
            .searchable_pages
            .as_ref()
            .map(|pages| pages[index].as_str());
        page_ids.push(builder.add_page_owned(index, image, text, cancellation)?);
    }
    Ok(())
}

fn transformed_page<L, F>(
    path: &Path,
    load: &mut L,
    transform: &mut F,
    cancellation: Option<&CancellationToken>,
) -> Result<ImageBuffer>
where
    L: FnMut(&Path) -> Result<ImageBuffer>,
    F: FnMut(ImageBuffer) -> Result<ImageBuffer>,
{
    check_pdf_cancellation(cancellation)?;
    let image = load(path)?;
    check_pdf_cancellation(cancellation)?;
    let image = transform(image)?;
    check_pdf_cancellation(cancellation)?;
    Ok(image)
}
