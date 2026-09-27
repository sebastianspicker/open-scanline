//! OCR dispatch and result contracts.

use super::load_image;
use crate::domain::image::ImageBuffer;
use crate::error::Result;
use crate::workflows::operation::CancellationToken;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

pub mod model_pack;
pub mod ocrs_runner;
pub mod template;
pub mod tesseract;

pub const OFFLINE_OCR_ENGINE: &str = "offline-template";

enum ExportOcrJob {
    Offline,
    Ocrs(ocrs_runner::OcrsJob),
    Tesseract,
}

impl crate::workflows::ports::media::OcrJob for ExportOcrJob {
    fn recognize(
        &self,
        image: &ImageBuffer,
        language: &str,
        cancellation: Option<&CancellationToken>,
    ) -> Result<String> {
        match self {
            Self::Offline => Ok(template::recognize(image)?.text),
            Self::Ocrs(job) => job.recognize(image, language, cancellation),
            Self::Tesseract => Ok(tesseract::recognize(image, language, cancellation)?.text),
        }
    }
}

pub(crate) fn prepare_job(
    engine: crate::workflows::publication::OcrEngine,
) -> Result<Arc<dyn crate::workflows::ports::media::OcrJob>> {
    if engine == crate::workflows::publication::OcrEngine::Ocrs && !cfg!(feature = "ocrs") {
        return Err(crate::error::ScanError::Unsupported(
            "OCRS support was not compiled into this build".into(),
        ));
    }
    let job = match engine {
        crate::workflows::publication::OcrEngine::Offline => ExportOcrJob::Offline,
        crate::workflows::publication::OcrEngine::Ocrs => {
            ExportOcrJob::Ocrs(ocrs_runner::OcrsJob::new())
        }
        crate::workflows::publication::OcrEngine::Tesseract => ExportOcrJob::Tesseract,
    };
    Ok(Arc::new(job))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrResult {
    pub text: String,
    pub engine: String,
    pub confidence: f64,
    pub language: String,
}

impl OcrResult {
    pub fn as_dict(&self) -> Value {
        json!({"text": self.text, "engine": self.engine, "confidence": self.confidence,
            "confidence_available": self.engine != ocrs_runner::OCRS_ENGINE,
            "language": self.language, "ok": true})
    }
}

/// Legacy built-in template OCR entry point.
pub fn ocr_image_offline(image: &ImageBuffer) -> Result<OcrResult> {
    template::recognize(image)
}

/// Legacy Tesseract availability probe.
pub fn tesseract_available() -> bool {
    tesseract::available()
}

/// Legacy Tesseract OCR entry point.
pub fn ocr_image_tesseract(image: &ImageBuffer, language: &str) -> Result<OcrResult> {
    ocr_image_tesseract_with_cancellation(image, language, None)
}

/// Legacy cancellation-aware Tesseract OCR entry point.
pub fn ocr_image_tesseract_with_cancellation(
    image: &ImageBuffer,
    language: &str,
    cancellation: Option<&CancellationToken>,
) -> Result<OcrResult> {
    tesseract::recognize(image, language, cancellation)
}

/// Legacy boolean mapping: true is the built-in template engine; false is Tesseract.
pub fn ocr_image(image: &ImageBuffer, language: &str, offline: bool) -> Result<OcrResult> {
    ocr_image_with_cancellation(image, language, offline, None)
}

pub fn ocr_image_with_cancellation(
    image: &ImageBuffer,
    language: &str,
    offline: bool,
    cancellation: Option<&CancellationToken>,
) -> Result<OcrResult> {
    if offline {
        ocr_image_offline(image)
    } else {
        ocr_image_tesseract_with_cancellation(image, language, cancellation)
    }
}

pub fn ocr_image_with_engine_with_cancellation(
    image: &ImageBuffer,
    language: &str,
    engine: crate::workflows::publication::OcrEngine,
    cancellation: Option<&CancellationToken>,
) -> Result<OcrResult> {
    match engine {
        crate::workflows::publication::OcrEngine::Offline => ocr_image_offline(image),
        crate::workflows::publication::OcrEngine::Ocrs => ocrs_runner::recognize(
            image,
            if language.is_empty() { "eng" } else { language },
            cancellation,
        ),
        crate::workflows::publication::OcrEngine::Tesseract => {
            ocr_image_tesseract_with_cancellation(image, language, cancellation)
        }
    }
}

pub fn ocr_file(path: impl AsRef<Path>, language: &str, offline: bool) -> Result<OcrResult> {
    ocr_image(&load_image(path)?, language, offline)
}

pub fn ocr_file_with_cancellation(
    path: impl AsRef<Path>,
    language: &str,
    offline: bool,
    cancellation: CancellationToken,
) -> Result<OcrResult> {
    ocr_image_with_cancellation(&load_image(path)?, language, offline, Some(&cancellation))
}

pub fn ocr_file_with_engine_with_cancellation(
    path: impl AsRef<Path>,
    language: &str,
    engine: crate::workflows::publication::OcrEngine,
    cancellation: CancellationToken,
) -> Result<OcrResult> {
    if engine == crate::workflows::publication::OcrEngine::Ocrs && !cfg!(feature = "ocrs") {
        return Err(crate::error::ScanError::Unsupported(
            "OCRS support was not compiled into this build".into(),
        ));
    }
    ocr_image_with_engine_with_cancellation(
        &load_image(path)?,
        language,
        engine,
        Some(&cancellation),
    )
}

pub fn ocr_module_info() -> Value {
    let tess = tesseract_available();
    let mut engines = vec![OFFLINE_OCR_ENGINE, "ocrs"];
    if tess {
        engines.push("tesseract");
    }
    json!({"tesseract_available": tess,
        "ocrs": model_pack::status().unwrap_or_else(|error| json!({
            "compiled": cfg!(feature = "ocrs"),
            "installed": false,
            "integrity": error.to_string()
        })),
        "engines": engines,
        "default_language": "eng", "ok": true})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::image::PixelFormat;
    use crate::error::ScanError;
    use crate::workflows::publication::OcrEngine;
    fn sample() -> ImageBuffer {
        ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![255; 3]).unwrap()
    }
    #[test]
    fn legacy_true_is_offline() {
        assert_eq!(
            ocr_image(&sample(), "eng", true).unwrap().engine,
            OFFLINE_OCR_ENGINE
        );
    }
    #[test]
    fn ocrs_rejects_non_english_before_pack_lookup() {
        assert!(matches!(
            ocr_image_with_engine_with_cancellation(&sample(), "deu", OcrEngine::Ocrs, None),
            Err(ScanError::Unsupported(_))
        ));
    }
    #[test]
    fn ocrs_confidence_is_unavailable() {
        assert_eq!(
            OcrResult {
                text: "x".into(),
                engine: ocrs_runner::OCRS_ENGINE.into(),
                confidence: 0.0,
                language: "eng".into()
            }
            .as_dict()["confidence_available"],
            false
        );
    }

    #[cfg(not(feature = "ocrs"))]
    #[test]
    fn disabled_module_status_reports_compilation_separately() {
        let status = ocr_module_info();
        assert_eq!(status["ocrs"]["compiled"], false);
        assert!(status["engines"]
            .as_array()
            .unwrap()
            .iter()
            .any(|engine| engine == "ocrs"));
    }

    #[cfg(not(feature = "ocrs"))]
    #[test]
    fn disabled_engine_file_call_rejects_before_loading_input() {
        let error = ocr_file_with_engine_with_cancellation(
            Path::new("missing-input-that-must-not-be-read.png"),
            "eng",
            OcrEngine::Ocrs,
            CancellationToken::new(),
        )
        .unwrap_err();
        assert!(
            matches!(error, ScanError::Unsupported(message) if message.contains("not compiled"))
        );
    }
}
