//! Pure export vocabulary shared by scan, process, and batch entry points.

use std::path::PathBuf;
use std::str::FromStr;

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

impl OcrEngine {
    /// Canonical lowercase names accepted by [`FromStr`], in declaration order.
    /// The single source of truth for every config/CLI/GUI OCR engine choice
    /// list, so they cannot drift out of sync with one another.
    pub const NAMES: [&'static str; 3] = ["offline", "ocrs", "tesseract"];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Ocrs => "ocrs",
            Self::Tesseract => "tesseract",
        }
    }
}

/// A name outside [`OcrEngine::NAMES`] was given where an OCR engine choice
/// is required.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrEngineParseError(pub String);

impl std::fmt::Display for OcrEngineParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "unknown OCR engine: {}", self.0)
    }
}

impl std::error::Error for OcrEngineParseError {}

impl FromStr for OcrEngine {
    type Err = OcrEngineParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "offline" => Ok(Self::Offline),
            "ocrs" => Ok(Self::Ocrs),
            "tesseract" => Ok(Self::Tesseract),
            _ => Err(OcrEngineParseError(value.to_string())),
        }
    }
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
