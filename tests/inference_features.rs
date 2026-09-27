//! Omitted inference support keeps CLI and library contracts without side effects.
#[cfg(not(feature = "onnx"))]
#[test]
fn omitted_onnx_rejects_before_validating_inputs_or_creating_reports() {
    use open_scanline::core::{ImageBuffer, PixelFormat, ScanError};
    use open_scanline::ml::{run_user_onnx_with_worker, OnnxInferenceOptions};
    let image = ImageBuffer::new(1, 1, PixelFormat::Gray8, vec![0]).unwrap();
    let result = run_user_onnx_with_worker(
        &image,
        "missing.onnx",
        &OnnxInferenceOptions::default(),
        "missing-worker",
    );
    assert!(
        matches!(result, Err(ScanError::Unsupported(message)) if message.contains("not compiled"))
    );
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_open-scanline"))
        .args(["onnx", "--in", "missing.png", "--model", "missing.onnx"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("not compiled"));
}

#[cfg(not(feature = "ocrs"))]
#[test]
fn omitted_ocrs_install_rejects_before_opening_sources() {
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_open-scanline"))
        .args([
            "ocr-model",
            "install",
            "--detection",
            "missing.rten",
            "--recognition",
            "missing.rten",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("not compiled"));
}

#[test]
fn diagnostics_distinguish_compiled_support() {
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_open-scanline"))
        .args(["info", "--module", "features"])
        .output()
        .unwrap();
    assert!(result.status.success());
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let features = value["features"]["features"].as_array().unwrap();
    for (id, compiled) in [
        ("ocr.ocrs", cfg!(feature = "ocrs")),
        ("ml.user-onnx", cfg!(feature = "onnx")),
    ] {
        let entry = features.iter().find(|entry| entry["id"] == id).unwrap();
        assert_eq!(entry["compiled"], compiled);
        if !compiled {
            assert_eq!(entry["available"], false);
        }
    }
}
