//! Additive export controls shared by scan, process, and batch entry points.

use crate::core::{ImageBuffer, Result, ScanError};
use crate::device::CancellationToken;
use crate::icc::{apply_scanner_profile, load_scanner_profile};
use crate::imaging::{
    load_image, save_image_with_cancellation,
    save_multipage_tiff_from_paths_with_transform_and_cancellation,
    save_pdf_from_paths_with_options_and_cancellation,
    save_pdf_from_paths_with_options_and_transform_and_cancellation,
    save_pdf_with_options_and_cancellation, PdfOptions,
};
use crate::ocr::ocr_image_with_cancellation;
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
    if options.searchable_pdf {
        crate::config::validate_ocr_language(&options.ocr_language)?;
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
        crate::imaging::validate_pdf_password(password)?;
    }
    let profile = options
        .scanner_profile
        .as_deref()
        .map(load_scanner_profile)
        .transpose()?;
    Ok(PreparedExportOptions {
        options: options.clone(),
        profile,
    })
}

pub(crate) fn apply_export_profile(
    image: &ImageBuffer,
    prepared: &PreparedExportOptions,
) -> Result<ImageBuffer> {
    match prepared.profile.as_ref() {
        Some(profile) => apply_scanner_profile(image, profile),
        None => Ok(image.clone()),
    }
}

/// Token-aware searchable-text generation for scan and batch workflows.
pub(crate) fn searchable_text_with_cancellation(
    image: &ImageBuffer,
    prepared: &PreparedExportOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<Option<String>> {
    if !prepared.options.searchable_pdf {
        return Ok(None);
    }
    let offline = matches!(prepared.options.ocr_engine, OcrEngine::Offline);
    Ok(Some(
        ocr_image_with_cancellation(image, &prepared.options.ocr_language, offline, cancellation)?
            .text,
    ))
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

/// Save a final scan image, forwarding a shared token to external OCR/JXL tools.
pub(crate) fn save_final_image_with_cancellation(
    destination: &Path,
    image: &ImageBuffer,
    dpi: Option<u32>,
    quality: Option<u8>,
    prepared: &PreparedExportOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    let is_pdf = destination
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
    if is_pdf {
        let text = searchable_text_with_cancellation(image, prepared, cancellation)?;
        return save_pdf_with_options_and_cancellation(
            destination,
            std::slice::from_ref(image),
            &PdfOptions {
                dpi: dpi.unwrap_or(150),
                title: "open-scanline scan".into(),
                password: prepared.options.pdf_password.clone(),
                searchable_pages: text.map(|entry| vec![entry]),
            },
            cancellation,
        );
    }
    save_image_with_cancellation(destination, image, dpi, quality, cancellation)
}

#[allow(dead_code)]
pub(crate) fn save_final_pdf_from_paths(
    destination: &Path,
    paths: &[PathBuf],
    dpi: u32,
    prepared: &PreparedExportOptions,
    searchable_pages: Vec<String>,
) -> Result<PathBuf> {
    save_final_pdf_from_paths_with_cancellation(
        destination,
        paths,
        dpi,
        prepared,
        searchable_pages,
        None,
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
    save_pdf_from_paths_with_options_and_cancellation(
        paths,
        destination,
        &PdfOptions {
            dpi,
            title: "open-scanline multipage".into(),
            password: prepared.options.pdf_password.clone(),
            searchable_pages,
        },
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
        "tif" | "tiff" => save_multipage_tiff_from_paths_with_transform_and_cancellation(
            paths,
            destination,
            Some(dpi),
            |image| apply_export_profile(image, &prepared),
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
            let image = apply_export_profile(&load_image(path)?, prepared)?;
            searchable_text_with_cancellation(&image, prepared, cancellation)?
                .ok_or_else(|| ScanError::Other("searchable PDF did not produce OCR text".into()))
        })?
    } else {
        Vec::new()
    };
    save_pdf_from_paths_with_options_and_transform_and_cancellation(
        paths,
        destination,
        &PdfOptions {
            dpi,
            title: "open-scanline multipage".into(),
            password: prepared.options.pdf_password.clone(),
            searchable_pages: prepared.options.searchable_pdf.then_some(searchable_pages),
        },
        |image| apply_export_profile(image, prepared),
        cancellation,
    )
}

fn collect_searchable_pages(
    paths: &[PathBuf],
    mut recognize: impl FnMut(&Path) -> Result<String>,
) -> Result<Vec<String>> {
    let mut pages = Vec::with_capacity(paths.len());
    let mut aggregate = 0_usize;
    for (index, path) in paths.iter().enumerate() {
        let text = recognize(path)?;
        aggregate = crate::imaging::checked_pdf_searchable_text_total(aggregate, &text, index)?;
        pages.push(text);
    }
    Ok(pages)
}
