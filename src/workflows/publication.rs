//! Additive export controls shared by scan, process, and batch entry points.

use crate::domain::export::{ExportOptions, OcrEngine};
use crate::domain::image::ImageBuffer;
use crate::domain::settings::validate_ocr_language;
use crate::error::{Result, ScanError};
use crate::operation::CancellationToken;
use crate::workflows::ports::media::{ImageTransform, MediaPort, PdfPathPublication};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) struct PreparedExportOptions {
    options: ExportOptions,
    profile: Option<serde_json::Value>,
    transform: Option<Arc<dyn ImageTransform>>,
    ocr_job: Option<Arc<dyn crate::workflows::ports::media::OcrJob>>,
}

impl PreparedExportOptions {
    pub(crate) fn needs_searchable_text(&self) -> bool {
        self.options.searchable_pdf
    }

    pub(crate) fn ocr_language(&self) -> &str {
        &self.options.ocr_language
    }

    pub(crate) fn ocr_engine(&self) -> OcrEngine {
        self.options.ocr_engine
    }

    pub(crate) fn profile(&self) -> Option<&serde_json::Value> {
        self.profile.as_ref()
    }

    pub(crate) fn transform(&self) -> Option<&dyn ImageTransform> {
        self.transform.as_deref()
    }

    pub(crate) fn pdf_password(&self) -> Option<&str> {
        self.options.pdf_password.as_deref()
    }

    pub(crate) fn recognize<M: MediaPort>(
        &self,
        image: &ImageBuffer,
        cancellation: Option<&CancellationToken>,
        media: &M,
    ) -> Result<String> {
        match self.ocr_job.as_deref() {
            Some(job) => job.recognize(image, self.ocr_language(), cancellation),
            None => media.recognize(image, self.ocr_language(), self.ocr_engine(), cancellation),
        }
    }
}

pub(crate) fn prepare_export_options_with_media<M: MediaPort>(
    destination: &Path,
    options: &ExportOptions,
    media: &M,
) -> Result<PreparedExportOptions> {
    let is_pdf = destination
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
    prepare_export_options_for_pdf_with_media(is_pdf, options, media)
}

pub(crate) fn prepare_export_options_for_pdf_with_media<M: MediaPort>(
    has_pdf_destination: bool,
    options: &ExportOptions,
    media: &M,
) -> Result<PreparedExportOptions> {
    validate_searchable_options(options)?;
    validate_pdf_destination(has_pdf_destination, options)?;
    validate_export_password(options, media)?;
    let profile = load_export_profile(options, media)?;
    let transform = profile
        .as_ref()
        .map(|profile| media.prepare_scanner_profile(profile))
        .transpose()?
        .flatten();
    let ocr_job = options
        .searchable_pdf
        .then(|| media.prepare_ocr_job(options.ocr_engine))
        .transpose()?
        .flatten();
    Ok(PreparedExportOptions {
        options: options.clone(),
        profile,
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

fn validate_export_password<M: MediaPort>(options: &ExportOptions, media: &M) -> Result<()> {
    if let Some(password) = options.pdf_password.as_deref() {
        if password.is_empty() {
            return Err(ScanError::Invalid(
                "PDF password must not be empty when encryption is requested".into(),
            ));
        }
        media.validate_pdf_password(password)?;
    }
    Ok(())
}

fn load_export_profile<M: MediaPort>(
    options: &ExportOptions,
    media: &M,
) -> Result<Option<serde_json::Value>> {
    options
        .scanner_profile
        .as_deref()
        .map(|path| media.load_scanner_profile(path))
        .transpose()
}

pub(crate) fn apply_export_profile_with_media<M: MediaPort>(
    image: &ImageBuffer,
    prepared: &PreparedExportOptions,
    media: &M,
) -> Result<ImageBuffer> {
    apply_export_profile_owned_with_media(image.clone(), prepared, media)
}

pub(crate) fn apply_export_profile_owned_with_media<M: MediaPort>(
    image: ImageBuffer,
    prepared: &PreparedExportOptions,
    media: &M,
) -> Result<ImageBuffer> {
    if let Some(transform) = prepared.transform() {
        return transform.apply_owned(image);
    }
    match prepared.profile() {
        Some(profile) => media.apply_scanner_profile(&image, Some(profile)),
        None => Ok(image),
    }
}

/// Token-aware searchable-text generation for scan and batch workflows.
pub(crate) fn searchable_text_with_cancellation_with_media<M: MediaPort>(
    image: &ImageBuffer,
    prepared: &PreparedExportOptions,
    cancellation: Option<&CancellationToken>,
    media: &M,
) -> Result<Option<String>> {
    if !prepared.options.searchable_pdf {
        return Ok(None);
    }
    Ok(Some(prepared.recognize(image, cancellation, media)?))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn save_final_image_with_searchable_text_with_media<M: MediaPort>(
    destination: &Path,
    image: &ImageBuffer,
    dpi: Option<u32>,
    quality: Option<u8>,
    prepared: &PreparedExportOptions,
    searchable_text: Option<&str>,
    cancellation: Option<&CancellationToken>,
    media: &M,
) -> Result<PathBuf> {
    let is_pdf = destination
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
    if is_pdf {
        let text = match searchable_text {
            Some(text) => Some(text.to_owned()),
            None => {
                searchable_text_with_cancellation_with_media(image, prepared, cancellation, media)?
            }
        };
        return media.publish_pdf(
            destination,
            std::slice::from_ref(image),
            dpi.unwrap_or(150),
            "open-scanline scan",
            prepared.options.pdf_password.as_deref(),
            text.map(|entry| vec![entry]),
            cancellation,
        );
    }
    media.publish_page(destination, image, dpi, quality, cancellation)
}

/// Build a batch PDF while forwarding cancellation through its final writer.
pub(crate) fn save_final_pdf_from_paths_with_cancellation_with_media<M: MediaPort>(
    destination: &Path,
    paths: &[PathBuf],
    dpi: u32,
    prepared: &PreparedExportOptions,
    searchable_pages: Vec<String>,
    cancellation: Option<&CancellationToken>,
    media: &M,
) -> Result<PathBuf> {
    let searchable_pages = prepared.options.searchable_pdf.then_some(searchable_pages);
    media.publish_pdf_from_paths(PdfPathPublication {
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
pub(crate) fn save_final_multipage_from_paths_with_cancellation_with_media<M: MediaPort>(
    destination: &Path,
    paths: &[PathBuf],
    dpi: u32,
    options: &ExportOptions,
    cancellation: Option<&CancellationToken>,
    media: &M,
) -> Result<PathBuf> {
    let prepared = prepare_export_options_with_media(destination, options, media)?;
    let extension = destination
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| ScanError::Invalid("multipage destination has no extension".into()))?;
    match extension.as_str() {
        "pdf" => save_profiled_pdf_from_paths_with_cancellation_with_media(
            destination,
            paths,
            dpi,
            &prepared,
            cancellation,
            media,
        ),
        "tif" | "tiff" => media.publish_tiff_from_paths(
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

fn save_profiled_pdf_from_paths_with_cancellation_with_media<M: MediaPort>(
    destination: &Path,
    paths: &[PathBuf],
    dpi: u32,
    prepared: &PreparedExportOptions,
    cancellation: Option<&CancellationToken>,
    media: &M,
) -> Result<PathBuf> {
    let searchable_pages = if prepared.options.searchable_pdf {
        collect_searchable_pages(
            paths,
            |path| {
                let image =
                    apply_export_profile_owned_with_media(media.load(path)?, prepared, media)?;
                searchable_text_with_cancellation_with_media(&image, prepared, cancellation, media)?
                    .ok_or_else(|| {
                        ScanError::Other("searchable PDF did not produce OCR text".into())
                    })
            },
            media,
        )?
    } else {
        Vec::new()
    };
    media.publish_pdf_from_paths(PdfPathPublication {
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

fn collect_searchable_pages<M: MediaPort>(
    paths: &[PathBuf],
    mut recognize: impl FnMut(&Path) -> Result<String>,
    media: &M,
) -> Result<Vec<String>> {
    let mut pages = Vec::with_capacity(paths.len());
    let mut aggregate = 0_usize;
    for (index, path) in paths.iter().enumerate() {
        let text = recognize(path)?;
        aggregate = media.checked_pdf_searchable_text_total(aggregate, &text, index)?;
        pages.push(text);
    }
    Ok(pages)
}
