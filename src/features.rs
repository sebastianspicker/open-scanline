//! Runtime capability reporting used by `info --module features`.

use crate::backend_process::{availability_probe_succeeds, CommandSpec};
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
    let features = vec![
        capability("device.mock", true, "built in", None),
        capability("device.file", true, "built in", None),
        capability(
            "device.wia",
            crate::wia::available(),
            "Windows Image Acquisition command adapter",
            Some("Windows and PowerShell with WIA support"),
        ),
        capability(
            "device.sane",
            crate::sane::available(),
            "scanimage command adapter",
            Some("scanimage on PATH"),
        ),
        capability(
            "device.escl",
            true,
            "HTTP/HTTPS eSCL client with explicit, mDNS and bounded subnet discovery",
            Some("network access to an eSCL scanner"),
        ),
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
        capability("pipeline", true, "built in", None),
        capability(
            "pipeline.scanner-profile",
            true,
            "validated open-scanline IT8 profile correction",
            None,
        ),
        capability("ocr.offline-5x7", true, "built in", None),
        capability(
            "ocr.tesseract",
            tesseract,
            "Tesseract command integration",
            Some("tesseract on PATH"),
        ),
        capability("ml.user-onnx", true, "tract", None),
        capability(
            "batch.streaming",
            true,
            "bounded one-session logical-side stream",
            None,
        ),
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
    ];
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
