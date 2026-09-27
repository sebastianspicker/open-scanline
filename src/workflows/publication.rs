//! Additive export controls shared by scan, process, and batch entry points.

use crate::domain::export::ExportOptions;
use crate::domain::image::ImageBuffer;
use crate::domain::settings::validate_ocr_language;
use crate::error::{Result, ScanError};
use crate::infrastructure::media::icc::PreparedScannerProfile;
use crate::infrastructure::media::ocr::ExportOcrJob;
use crate::infrastructure::media::PdfPathPublication;
use crate::operation::CancellationToken;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) struct PreparedExportOptions {
    options: ExportOptions,
    transform: Option<Arc<PreparedScannerProfile>>,
    ocr_job: Option<Arc<ExportOcrJob>>,
}

impl PreparedExportOptions {
    pub(crate) fn needs_searchable_text(&self) -> bool {
        self.options.searchable_pdf
    }

    pub(crate) fn ocr_language(&self) -> &str {
        &self.options.ocr_language
    }

    pub(crate) fn transform(&self) -> Option<&PreparedScannerProfile> {
        self.transform.as_deref()
    }

    pub(crate) fn pdf_password(&self) -> Option<&str> {
        self.options.pdf_password.as_deref()
    }

    /// Recognize page text when this export produces a searchable PDF.
    /// The OCR job is prepared exactly when `searchable_pdf` is requested.
    pub(crate) fn recognize(
        &self,
        image: &ImageBuffer,
        cancellation: Option<&CancellationToken>,
    ) -> Result<Option<String>> {
        self.ocr_job
            .as_deref()
            .map(|job| job.recognize(image, self.ocr_language(), cancellation))
            .transpose()
    }
}

pub(crate) fn prepare_export_options(
    destination: &Path,
    options: &ExportOptions,
) -> Result<PreparedExportOptions> {
    let is_pdf = destination
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
    prepare_export_options_for_pdf(is_pdf, options)
}

pub(crate) fn prepare_export_options_for_pdf(
    has_pdf_destination: bool,
    options: &ExportOptions,
) -> Result<PreparedExportOptions> {
    validate_searchable_options(options)?;
    validate_pdf_destination(has_pdf_destination, options)?;
    validate_export_password(options)?;
    let transform = load_export_transform(options)?;
    let ocr_job = options
        .searchable_pdf
        .then(|| crate::infrastructure::media::ocr::prepare_job(options.ocr_engine))
        .transpose()?;
    Ok(PreparedExportOptions {
        options: options.clone(),
        transform,
        ocr_job,
    })
}

fn validate_searchable_options(options: &ExportOptions) -> Result<()> {
    if options.searchable_pdf {
        validate_ocr_language(&options.ocr_language)?;
    }
    Ok(())
}

fn validate_pdf_destination(has_pdf_destination: bool, options: &ExportOptions) -> Result<()> {
    let has_pdf_only_option = options.searchable_pdf || options.pdf_password.is_some();
    if !has_pdf_destination && has_pdf_only_option {
        return Err(ScanError::Invalid(
            "searchable PDF and PDF password options require a .pdf destination".into(),
        ));
    }
    Ok(())
}

fn validate_export_password(options: &ExportOptions) -> Result<()> {
    if let Some(password) = options.pdf_password.as_deref() {
        if password.is_empty() {
            return Err(ScanError::Invalid(
                "PDF password must not be empty when encryption is requested".into(),
            ));
        }
        crate::infrastructure::media::validate_pdf_password(password)?;
    }
    Ok(())
}

fn load_export_transform(options: &ExportOptions) -> Result<Option<Arc<PreparedScannerProfile>>> {
    options
        .scanner_profile
        .as_deref()
        .map(|path| {
            let profile = crate::infrastructure::media::icc::load_scanner_profile(path)?;
            let prepared = crate::infrastructure::media::icc::prepare_scanner_profile(&profile)?;
            Ok(Arc::new(prepared))
        })
        .transpose()
}

pub(crate) fn apply_export_profile(
    image: &ImageBuffer,
    prepared: &PreparedExportOptions,
) -> Result<ImageBuffer> {
    apply_export_profile_owned(image.clone(), prepared)
}

pub(crate) fn apply_export_profile_owned(
    image: ImageBuffer,
    prepared: &PreparedExportOptions,
) -> Result<ImageBuffer> {
    match prepared.transform() {
        Some(transform) => transform.apply_owned(image),
        None => Ok(image),
    }
}

/// Token-aware searchable-text generation for scan and batch workflows.
pub(crate) fn searchable_text_with_cancellation(
    image: &ImageBuffer,
    prepared: &PreparedExportOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<Option<String>> {
    prepared.recognize(image, cancellation)
}

pub(crate) fn save_final_image(
    destination: &Path,
    image: &ImageBuffer,
    dpi: Option<u32>,
    quality: Option<u8>,
    prepared: &PreparedExportOptions,
) -> Result<PathBuf> {
    save_final_image_with_cancellation(destination, image, dpi, quality, prepared, None)
}

pub(crate) fn save_final_image_with_cancellation(
    destination: &Path,
    image: &ImageBuffer,
    dpi: Option<u32>,
    quality: Option<u8>,
    prepared: &PreparedExportOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    save_final_image_with_searchable_text(
        destination,
        image,
        dpi,
        quality,
        prepared,
        None,
        cancellation,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn save_final_image_with_searchable_text(
    destination: &Path,
    image: &ImageBuffer,
    dpi: Option<u32>,
    quality: Option<u8>,
    prepared: &PreparedExportOptions,
    searchable_text: Option<&str>,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    let is_pdf = destination
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
    if is_pdf {
        let text = match searchable_text {
            Some(text) => Some(text.to_owned()),
            None => searchable_text_with_cancellation(image, prepared, cancellation)?,
        };
        return crate::infrastructure::media::save_pdf_with_options_and_cancellation(
            destination,
            std::slice::from_ref(image),
            &crate::infrastructure::media::PdfOptions {
                dpi: dpi.unwrap_or(150),
                title: "open-scanline scan".into(),
                password: prepared.options.pdf_password.clone(),
                searchable_pages: text.map(|entry| vec![entry]),
            },
            cancellation,
        );
    }
    crate::infrastructure::media::save_image_with_cancellation(
        destination,
        image,
        dpi,
        quality,
        cancellation,
    )
}

/// Build a batch PDF while forwarding cancellation through its final writer.
pub(crate) fn save_final_pdf_from_paths_with_cancellation(
    destination: &Path,
    paths: &[PathBuf],
    dpi: u32,
    prepared: &PreparedExportOptions,
    searchable_pages: Vec<String>,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    let searchable_pages = prepared.options.searchable_pdf.then_some(searchable_pages);
    crate::infrastructure::media::publish_pdf_from_paths(PdfPathPublication {
        paths,
        destination,
        dpi,
        title: "open-scanline multipage",
        password: prepared.options.pdf_password.as_deref(),
        searchable_pages,
        transform: None,
        cancellation,
    })
}

/// Rebuild a multipage GUI destination from its ordered source images.
///
/// Validation happens before a container writer creates an output directory.
/// PDF pages are decoded and profile-transformed one at a time during the
/// final write; OCR observes the same profile-adjusted pixels.
/// Token-aware multipage export. OCR is interrupted promptly when a shared
/// scan/batch/GUI cancellation token is cancelled.
#[cfg_attr(not(any(feature = "gui", test)), allow(dead_code))]
pub(crate) fn save_final_multipage_from_paths_with_cancellation(
    destination: &Path,
    paths: &[PathBuf],
    dpi: u32,
    options: &ExportOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    let prepared = prepare_export_options(destination, options)?;
    let extension = destination
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| ScanError::Invalid("multipage destination has no extension".into()))?;
    match extension.as_str() {
        "pdf" => save_profiled_pdf_from_paths_with_cancellation(
            destination,
            paths,
            dpi,
            &prepared,
            cancellation,
        ),
        "tif" | "tiff" => crate::infrastructure::media::publish_tiff_from_paths(
            paths,
            destination,
            Some(dpi),
            prepared.transform(),
            cancellation,
        ),
        _ => Err(ScanError::Invalid(format!(
            "multipage export supports PDF or TIFF, not '.{extension}'"
        ))),
    }
}

fn save_profiled_pdf_from_paths_with_cancellation(
    destination: &Path,
    paths: &[PathBuf],
    dpi: u32,
    prepared: &PreparedExportOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    let searchable_pages = if prepared.options.searchable_pdf {
        collect_searchable_pages(paths, |path| {
            let image = apply_export_profile_owned(
                crate::infrastructure::media::load_image(path)?,
                prepared,
            )?;
            searchable_text_with_cancellation(&image, prepared, cancellation)?
                .ok_or_else(|| ScanError::Other("searchable PDF did not produce OCR text".into()))
        })?
    } else {
        Vec::new()
    };
    crate::infrastructure::media::publish_pdf_from_paths(PdfPathPublication {
        paths,
        destination,
        dpi,
        title: "open-scanline multipage",
        password: prepared.options.pdf_password.as_deref(),
        searchable_pages: prepared.options.searchable_pdf.then_some(searchable_pages),
        transform: prepared.transform(),
        cancellation,
    })
}

fn collect_searchable_pages(
    paths: &[PathBuf],
    mut recognize: impl FnMut(&Path) -> Result<String>,
) -> Result<Vec<String>> {
    let mut pages = Vec::with_capacity(paths.len());
    let mut aggregate = 0_usize;
    for (index, path) in paths.iter().enumerate() {
        let text = recognize(path)?;
        aggregate = crate::infrastructure::media::checked_pdf_searchable_text_total(
            aggregate, &text, index,
        )?;
        pages.push(text);
    }
    Ok(pages)
}
