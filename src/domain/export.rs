//! Pure export vocabulary shared by scan, process, and batch entry points.

use crate::error::ScanError;
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

impl FromStr for OcrEngine {
    type Err = ScanError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "offline" => Ok(Self::Offline),
            "ocrs" => Ok(Self::Ocrs),
            "tesseract" => Ok(Self::Tesseract),
            _ => Err(ScanError::Invalid(format!("unknown OCR engine: {value}"))),
        }
    }
}

/// Runtime-only controls for a scan, process, or batch export.
///
/// `pdf_password` is used only while writing a PDF and is never persisted by
/// this library. Existing argument structs intentionally do not embed these
/// settings so their public struct-literal API remains compatible.
#[derive(Clone, PartialEq, Eq)]
pub struct ExportOptions {
    pub pdf_password: Option<String>,
    pub searchable_pdf: bool,
    pub ocr_language: String,
    pub ocr_engine: OcrEngine,
    pub scanner_profile: Option<PathBuf>,
}

/// Overwrite a PDF password with zeros before releasing it.
pub fn clear_pdf_password(pdf_password: &mut Option<String>) {
    if let Some(password) = pdf_password.as_mut() {
        // SAFETY: an all-zero byte string is valid UTF-8.
        for byte in unsafe { password.as_bytes_mut() } {
            // Volatile, so the store is not removed as dead before the drop.
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
    *pdf_password = None;
}

impl std::fmt::Debug for ExportOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExportOptions")
            .field(
                "pdf_password",
                &self.pdf_password.as_ref().map(|_| "<redacted>"),
            )
            .field("searchable_pdf", &self.searchable_pdf)
            .field("ocr_language", &self.ocr_language)
            .field("ocr_engine", &self.ocr_engine)
            .field("scanner_profile", &self.scanner_profile)
            .finish()
    }
}

impl Drop for ExportOptions {
    fn drop(&mut self) {
        clear_pdf_password(&mut self.pdf_password);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_and_clearing_never_keep_the_password() {
        let mut options = ExportOptions::default();
        options.pdf_password = Some("hunter2".into());
        assert!(!format!("{options:?}").contains("hunter2"));
        clear_pdf_password(&mut options.pdf_password);
        assert_eq!(options.pdf_password, None);
    }
}
