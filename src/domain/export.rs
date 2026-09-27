//! Pure export vocabulary shared by scan, process, and batch entry points.

use std::path::PathBuf;

/// OCR implementation used when creating a searchable PDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrEngine {
    /// Built-in OCR with no external executable dependency.
    Offline,
    /// Locally installed OCRS RTen model pack (printed Latin `eng`).
    Ocrs,
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
