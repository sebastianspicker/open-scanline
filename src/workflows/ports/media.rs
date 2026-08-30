//! Cohesive boundary between workflows and native media handling.

use crate::domain::image::ImageBuffer;
use crate::error::Result;
use crate::workflows::operation::CancellationToken;
use std::path::{Path, PathBuf};

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
        offline: bool,
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
    #[allow(clippy::too_many_arguments)]
    fn publish_pdf_from_paths(
        &self,
        paths: &[PathBuf],
        destination: &Path,
        dpi: u32,
        title: &str,
        password: Option<&str>,
        searchable_pages: Option<Vec<String>>,
        profile: Option<&serde_json::Value>,
        cancellation: Option<&CancellationToken>,
    ) -> Result<PathBuf>;

    /// Assemble TIFF pages, optionally applying a scanner profile as pages load.
    fn publish_tiff_from_paths(
        &self,
        paths: &[PathBuf],
        destination: &Path,
        dpi: Option<u32>,
        profile: Option<&serde_json::Value>,
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
