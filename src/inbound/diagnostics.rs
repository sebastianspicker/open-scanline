//! Runtime capability reporting used by `info --module features`.

use crate::infrastructure::runtime::{availability_probe_succeeds, CommandSpec};
use crate::{APP_NAME, VERSION};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
struct Capability {
    id: &'static str,
    available: bool,
    implementation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    requirement: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    confidence_available: Option<bool>,
    compiled: bool,
}

fn capability(
    id: &'static str,
    available: bool,
    implementation: &'static str,
    requirement: Option<&'static str>,
) -> Capability {
    Capability {
        id,
        available,
        implementation,
        requirement,
        confidence_available: None,
        compiled: cfg!(feature = "gui") || !matches!(id, "gui" | "gui.native-dialogs"),
    }
}

fn command_available(program: &str, arg: &str) -> bool {
    availability_probe_succeeds(
        &CommandSpec {
            program: program.into(),
            args: vec![arg.into()],
        },
        Duration::from_secs(2),
        program,
    )
}

pub fn feature_matrix() -> Value {
    let tesseract = command_available("tesseract", "--version");
    let cjxl = command_available("cjxl", "--version");
    let mut features = device_capabilities();
    features.extend(codec_capabilities(cjxl));
    features.extend(document_capabilities());
    features.extend(processing_capabilities(tesseract));
    features.extend(interface_capabilities());
    json!({
        "app": APP_NAME,
        "version": VERSION,
        "features": features,
        "notes": [
            "hardware availability is reported separately from compiled support",
            "TWAIN support is host integration, not a native acquisition data source"
        ]
    })
}

fn device_capabilities() -> Vec<Capability> {
    vec![
        capability("device.mock", true, "built in", None),
        capability("device.file", true, "built in", None),
        capability(
            "device.wia",
            crate::infrastructure::acquisition::wia::available(),
            "Windows Image Acquisition command adapter",
            Some("Windows and PowerShell with WIA support"),
        ),
        capability(
            "device.sane",
            crate::infrastructure::acquisition::sane::available(),
            "scanimage command adapter",
            Some("scanimage on PATH"),
        ),
        capability(
            "device.escl",
            true,
            "HTTP/HTTPS eSCL client with explicit, mDNS and bounded subnet discovery",
            Some("network access to an eSCL scanner"),
        ),
    ]
}

fn codec_capabilities(cjxl: bool) -> Vec<Capability> {
    vec![
        capability("codec.png", true, "image crate", None),
        capability("codec.jpeg", true, "image crate", None),
        capability("codec.tiff", true, "image and tiff crates", None),
        capability("codec.webp", true, "image crate", None),
        capability("codec.bmp", true, "image crate", None),
        capability("codec.gif", true, "image crate", None),
        capability(
            "codec.jpeg-xl",
            cjxl,
            "libjxl command integration",
            Some("cjxl on PATH"),
        ),
    ]
}

fn document_capabilities() -> Vec<Capability> {
    vec![
        capability("format.pdf", true, "lopdf", None),
        capability(
            "format.pdf-searchable",
            true,
            "lopdf plus selected OCR engine",
            None,
        ),
        capability(
            "format.pdf-aes256",
            true,
            "lopdf PDF 2.0 AES-256 encryption",
            None,
        ),
        capability("format.multipage-pdf", true, "lopdf", None),
        capability("format.multipage-tiff", true, "tiff crate", None),
    ]
}

fn processing_capabilities(tesseract: bool) -> Vec<Capability> {
    vec![
        capability("pipeline", true, "built in", None),
        capability(
            "pipeline.scanner-profile",
            true,
            "validated open-scanline IT8 profile correction",
            None,
        ),
        capability("ocr.offline-5x7", true, "built in", None),
        Capability {
            id: "ocr.ocrs",
            available: cfg!(feature = "ocrs")
                && crate::infrastructure::media::ocr::model_pack::status()
                    .map(|status| status["installed"].as_bool().unwrap_or(false))
                    .unwrap_or(false),
            implementation: "locally installed OCRS RTen model pack",
            requirement: Some("ocr-model install with valid detection and recognition models"),
            confidence_available: Some(false),
            compiled: cfg!(feature = "ocrs"),
        },
        capability(
            "ocr.tesseract",
            tesseract,
            "Tesseract command integration",
            Some("tesseract on PATH"),
        ),
        Capability {
            compiled: cfg!(feature = "onnx"),
            ..capability(
                "ml.user-onnx",
                cfg!(feature = "onnx"),
                "tract",
                Some("onnx feature"),
            )
        },
        capability(
            "batch.streaming",
            true,
            "bounded one-session logical-side stream",
            None,
        ),
    ]
}

fn interface_capabilities() -> Vec<Capability> {
    vec![
        capability(
            "gui",
            cfg!(feature = "gui"),
            "egui/eframe",
            Some("gui feature"),
        ),
        capability(
            "gui.native-dialogs",
            cfg!(feature = "gui"),
            "rfd",
            Some("gui feature and a desktop session"),
        ),
        capability("config", true, "JSON application config", None),
        capability("i18n", true, "29 embedded catalogs", None),
        capability("packaging", true, "zip crate with launch validation", None),
        capability(
            "twain.host-integration",
            true,
            "headless plugin host launcher",
            Some("a separate TWAIN-capable host or bridge"),
        ),
    ]
}

pub fn ml_module_info() -> Value {
    json!({
        "auto_crop": true,
        "orientation": true,
        "run_user_onnx": cfg!(feature = "onnx"),
        "onnxruntime_available": false,
        "tract_onnx_available": cfg!(feature = "onnx"),
        "cli_worker_isolation": true,
        "explicit_worker_runtime": true,
        "automatic_worker_discovery": false,
        "bundled_models": [],
        "methods": ["luma-threshold-bbox", "gradient-energy", "user-onnx"],
        "ok": true,
    })
}

#[cfg(test)]
mod tests {
    use super::feature_matrix;

    #[test]
    fn ocrs_diagnostics_disclose_that_confidence_is_not_available() {
        let features = feature_matrix()["features"].as_array().unwrap().clone();
        let ocrs = features
            .iter()
            .find(|entry| entry["id"] == "ocr.ocrs")
            .unwrap();
        assert_eq!(ocrs["confidence_available"], false);
    }
}
