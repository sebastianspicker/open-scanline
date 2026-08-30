//! Additive export controls shared by scan, process, and batch entry points.

use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use crate::workflows::operation::CancellationToken;
use crate::workflows::ports::media::MediaPort;
use crate::workflows::settings::validate_ocr_language;
use std::path::{Path, PathBuf};

/// OCR implementation used when creating a searchable PDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrEngine {
    /// Built-in OCR with no external executable dependency.
    Offline,
    /// The system `tesseract` executable.
    Tesseract,
}

/// Runtime-only controls for a scan, process, or batch export.
///
/// `pdf_password` is used only while writing a PDF and is never persisted by
/// this library. Existing argument structs intentionally do not embed these
/// settings so their public struct-literal API remains compatible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportOptions {
    pub pdf_password: Option<String>,
    pub searchable_pdf: bool,
    pub ocr_language: String,
    pub ocr_engine: OcrEngine,
    pub scanner_profile: Option<PathBuf>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            pdf_password: None,
            searchable_pdf: false,
            ocr_language: "eng".into(),
            ocr_engine: OcrEngine::Offline,
            scanner_profile: None,
        }
    }
}

pub(crate) struct PreparedExportOptions {
    options: ExportOptions,
    profile: Option<serde_json::Value>,
}

impl PreparedExportOptions {
    pub(crate) fn needs_searchable_text(&self) -> bool {
        self.options.searchable_pdf
    }

    pub(crate) fn ocr_language(&self) -> &str {
        &self.options.ocr_language
    }

    pub(crate) fn uses_offline_ocr(&self) -> bool {
        matches!(self.options.ocr_engine, OcrEngine::Offline)
    }

    pub(crate) fn profile(&self) -> Option<&serde_json::Value> {
        self.profile.as_ref()
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
    if options.searchable_pdf {
        validate_ocr_language(&options.ocr_language)?;
    }
    if !has_pdf_destination && (options.searchable_pdf || options.pdf_password.is_some()) {
        return Err(ScanError::Invalid(
            "searchable PDF and PDF password options require a .pdf destination".into(),
        ));
    }
    if let Some(password) = options.pdf_password.as_deref() {
        if password.is_empty() {
            return Err(ScanError::Invalid(
                "PDF password must not be empty when encryption is requested".into(),
            ));
        }
        media.validate_pdf_password(password)?;
    }
    let profile = options
        .scanner_profile
        .as_deref()
        .map(|path| media.load_scanner_profile(path))
        .transpose()?;
    Ok(PreparedExportOptions {
        options: options.clone(),
        profile,
    })
}

pub(crate) fn apply_export_profile_with_media<M: MediaPort>(
    image: &ImageBuffer,
    prepared: &PreparedExportOptions,
    media: &M,
) -> Result<ImageBuffer> {
    media.apply_scanner_profile(image, prepared.profile())
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
    Ok(Some(media.recognize(
        image,
        prepared.ocr_language(),
        prepared.uses_offline_ocr(),
        cancellation,
    )?))
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
    media.publish_pdf_from_paths(
        paths,
        destination,
        dpi,
        "open-scanline multipage",
        prepared.options.pdf_password.as_deref(),
        searchable_pages,
        None,
        cancellation,
    )
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
            prepared.profile(),
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
                let image = apply_export_profile_with_media(&media.load(path)?, prepared, media)?;
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
    media.publish_pdf_from_paths(
        paths,
        destination,
        dpi,
        "open-scanline multipage",
        prepared.options.pdf_password.as_deref(),
        prepared.options.searchable_pdf.then_some(searchable_pages),
        prepared.profile(),
        cancellation,
    )
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
