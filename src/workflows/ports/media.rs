//! Cohesive boundary between workflows and native media handling.

use crate::domain::image::ImageBuffer;
use crate::error::Result;
use crate::workflows::operation::CancellationToken;
use crate::workflows::publication::OcrEngine;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Prepared image adjustment owned by an infrastructure adapter.
pub(crate) trait ImageTransform: Send + Sync {
    fn apply_owned(&self, image: ImageBuffer) -> Result<ImageBuffer>;
}

/// One export-scoped OCR executor. Implementations may lazily prepare native
/// engine state and reuse it across all pages in the export.
pub(crate) trait OcrJob: Send + Sync {
    fn recognize(
        &self,
        image: &ImageBuffer,
        language: &str,
        cancellation: Option<&CancellationToken>,
    ) -> Result<String>;
}

/// Private document-publication session used when a batch requests several
/// aggregate derivatives from the same already-published pages.
pub(crate) trait AggregateMediaSession {
    fn publish_pdf(&mut self, request: PdfPathPublication<'_>) -> Result<PathBuf>;

    fn publish_tiff(
        &mut self,
        paths: &[PathBuf],
        destination: &Path,
        dpi: Option<u32>,
        transform: Option<&dyn ImageTransform>,
        cancellation: Option<&CancellationToken>,
    ) -> Result<PathBuf>;

    fn publish_contact_sheet(
        &mut self,
        paths: &[PathBuf],
        destination: &Path,
        cancellation: Option<&CancellationToken>,
    ) -> Result<PathBuf>;
}

/// Internal request for assembling a PDF from already-published pages.
pub(crate) struct PdfPathPublication<'a> {
    pub(crate) paths: &'a [PathBuf],
    pub(crate) destination: &'a Path,
    pub(crate) dpi: u32,
    pub(crate) title: &'a str,
    pub(crate) password: Option<&'a str>,
    pub(crate) searchable_pages: Option<Vec<String>>,
    pub(crate) transform: Option<&'a dyn ImageTransform>,
    pub(crate) cancellation: Option<&'a CancellationToken>,
}

/// The media operations that workflows need as one coherent boundary.
///
/// Codec selection, PDF/TIFF details, OCR implementation, and scanner-profile
/// representation remain infrastructure concerns. Workflows only load, adjust,
/// recognize, and publish page or document media through this port.
pub trait MediaPort: Send + Sync {
    /// Verify a leaf can be atomically replaced without touching the filesystem.
    fn validate_output_leaf(&self, path: &Path, label: &str) -> Result<()>;

    /// Determine whether two output paths target the same eventual file.
    fn output_paths_alias(&self, left: &Path, right: &Path) -> Result<bool>;

    /// Prepare a validated output directory before incremental publication.
    fn prepare_output_directory(&self, directory: &Path) -> Result<()>;

    /// Report whether an image extension can be published by the native adapter.
    fn supports_extension(&self, extension: &str) -> bool;

    fn load(&self, source: &Path) -> Result<ImageBuffer>;

    /// Load and validate a scanner profile at the application boundary.
    fn load_scanner_profile(&self, source: &Path) -> Result<serde_json::Value>;

    /// Prepare a reusable typed profile. Legacy adapters may keep using the
    /// JSON application hook by accepting this default.
    fn prepare_scanner_profile(
        &self,
        _profile: &serde_json::Value,
    ) -> Result<Option<Arc<dyn ImageTransform>>> {
        Ok(None)
    }

    /// Prepare private OCR state once for an export. Adapters that do not
    /// provide a job retain the legacy per-call `recognize` behavior.
    fn prepare_ocr_job(&self, _engine: OcrEngine) -> Result<Option<Arc<dyn OcrJob>>> {
        Ok(None)
    }

    /// Begin a shared decoded-page session for aggregate outputs. The default
    /// preserves existing adapters and their publication call ordering.
    fn begin_aggregate_session(
        &self,
        _paths: &[PathBuf],
    ) -> Result<Option<Box<dyn AggregateMediaSession + '_>>> {
        Ok(None)
    }

    /// Reject passwords the PDF backend cannot safely encode.
    fn validate_pdf_password(&self, password: &str) -> Result<()>;

    fn apply_scanner_profile(
        &self,
        image: &ImageBuffer,
        profile: Option<&serde_json::Value>,
    ) -> Result<ImageBuffer>;

    fn recognize(
        &self,
        image: &ImageBuffer,
        language: &str,
        engine: OcrEngine,
        cancellation: Option<&CancellationToken>,
    ) -> Result<String>;

    fn publish_page(
        &self,
        destination: &Path,
        image: &ImageBuffer,
        dpi: Option<u32>,
        quality: Option<u8>,
        cancellation: Option<&CancellationToken>,
    ) -> Result<PathBuf>;

    /// Publish the acquired, pre-pipeline archive image.
    fn publish_raw(
        &self,
        destination: &Path,
        image: &ImageBuffer,
        dpi: Option<u32>,
        cancellation: Option<&CancellationToken>,
    ) -> Result<PathBuf>;

    /// Account for searchable-PDF text while preserving backend limits.
    fn checked_pdf_searchable_text_total(
        &self,
        current: usize,
        text: &str,
        page_index: usize,
    ) -> Result<usize>;

    /// Publish a PDF from in-memory pages with optional OCR text and encryption.
    #[allow(clippy::too_many_arguments)]
    fn publish_pdf(
        &self,
        destination: &Path,
        images: &[ImageBuffer],
        dpi: u32,
        title: &str,
        password: Option<&str>,
        searchable_pages: Option<Vec<String>>,
        cancellation: Option<&CancellationToken>,
    ) -> Result<PathBuf>;

    /// Assemble a PDF from already-published pages.
    fn publish_pdf_from_paths(&self, request: PdfPathPublication<'_>) -> Result<PathBuf>;

    /// Assemble TIFF pages, optionally applying a scanner profile as pages load.
    fn publish_tiff_from_paths(
        &self,
        paths: &[PathBuf],
        destination: &Path,
        dpi: Option<u32>,
        transform: Option<&dyn ImageTransform>,
        cancellation: Option<&CancellationToken>,
    ) -> Result<PathBuf>;

    /// Publish the requested contact-sheet derivative from published pages.
    fn publish_contact_sheet(
        &self,
        paths: &[PathBuf],
        destination: &Path,
        cancellation: Option<&CancellationToken>,
    ) -> Result<PathBuf>;
}
