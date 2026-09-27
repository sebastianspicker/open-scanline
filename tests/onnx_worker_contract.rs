#![cfg(feature = "onnx")]

use open_scanline::core::{ImageBuffer, PixelFormat};
use open_scanline::imaging::save_image;
#[cfg(not(windows))]
#[allow(deprecated)]
use open_scanline::ml::{
    run_user_onnx, run_user_onnx_with_worker, OnnxInferenceOptions, OnnxRuntime,
};
use std::process::Command;

fn push_varint(bytes: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        bytes.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    bytes.push(value as u8);
}

fn push_varint_field(bytes: &mut Vec<u8>, field: u8, value: u64) {
    push_varint(bytes, (field as u64) << 3);
    push_varint(bytes, value);
}

fn push_message_field(bytes: &mut Vec<u8>, field: u8, message: &[u8]) {
    push_varint(bytes, ((field as u64) << 3) | 2);
    push_varint(bytes, message.len() as u64);
    bytes.extend_from_slice(message);
}

fn value_info(name: &str, shape: &[u64]) -> Vec<u8> {
    let mut tensor_shape = Vec::new();
    for dimension in shape {
        let mut dim = Vec::new();
        push_varint_field(&mut dim, 1, *dimension);
        push_message_field(&mut tensor_shape, 1, &dim);
    }
    let mut tensor_type = Vec::new();
    push_varint_field(&mut tensor_type, 1, 1);
    push_message_field(&mut tensor_type, 2, &tensor_shape);
    let mut type_proto = Vec::new();
    push_message_field(&mut type_proto, 1, &tensor_type);
    let mut value = Vec::new();
    push_message_field(&mut value, 1, name.as_bytes());
    push_message_field(&mut value, 2, &type_proto);
    value
}

fn identity_onnx() -> Vec<u8> {
    let mut node = Vec::new();
    push_message_field(&mut node, 1, b"pixels");
    push_message_field(&mut node, 2, b"result");
    push_message_field(&mut node, 4, b"Identity");
    let mut graph = Vec::new();
    push_message_field(&mut graph, 1, &node);
    push_message_field(&mut graph, 2, b"isolated-worker-test");
    push_message_field(&mut graph, 11, &value_info("pixels", &[1, 3, 2, 2]));
    push_message_field(&mut graph, 12, &value_info("result", &[1, 3, 2, 2]));
    let mut opset = Vec::new();
    push_varint_field(&mut opset, 2, 13);
    let mut model = Vec::new();
    push_varint_field(&mut model, 1, 7);
    push_message_field(&mut model, 7, &graph);
    push_message_field(&mut model, 8, &opset);
    model
}

fn test_directory(name: &str) -> std::path::PathBuf {
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce).unwrap();
    let directory = std::env::temp_dir().join(format!(
        "open-scanline-{name}-{}-{}",
        std::process::id(),
        u128::from_le_bytes(nonce)
    ));
    std::fs::create_dir(&directory).unwrap();
    directory
}

fn test_image() -> ImageBuffer {
    ImageBuffer::new(
        2,
        2,
        PixelFormat::Rgb8,
        vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
    )
    .unwrap()
}

fn cli_test_assets() -> (
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let directory = test_directory("onnx-worker-contract");
    let input = directory.join("input.png");
    let model = directory.join("identity.onnx");
    save_image(&input, &test_image(), None, None).unwrap();
    std::fs::write(&model, identity_onnx()).unwrap();

    #[cfg(windows)]
    let executable = {
        // Cargo may hard-link Windows integration-test binaries into target/debug.
        // Copy the same bytes so this installed-artifact contract still exercises the
        // production worker's fail-closed single-link policy.
        let executable = directory.join("open-scanline.exe");
        assert!(std::fs::copy(env!("CARGO_BIN_EXE_open-scanline"), &executable).unwrap() > 0);
        executable
    };
    #[cfg(not(windows))]
    let executable = std::path::PathBuf::from(env!("CARGO_BIN_EXE_open-scanline"));

    (directory, input, model, executable)
}

#[test]
fn cli_runs_user_model_in_the_contained_worker() {
    let (directory, input, model, executable) = cli_test_assets();

    let output = Command::new(executable)
        .args([
            "onnx",
            "--in",
            input.to_str().unwrap(),
            "--model",
            model.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(directory);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(report["engine"], "tract-onnx");
    assert_eq!(
        report["outputs"][0]["shape"],
        serde_json::json!([1, 3, 2, 2])
    );
}

#[cfg(not(windows))]
#[test]
fn library_uses_an_explicit_version_matched_worker() {
    let directory = test_directory("onnx-library-worker-contract");
    let model = directory.join("identity.onnx");
    std::fs::write(&model, identity_onnx()).unwrap();
    let image = test_image();

    let runtime = OnnxRuntime::from_worker(env!("CARGO_BIN_EXE_open-scanline")).unwrap();
    let report = runtime.run(&image, &model).unwrap();
    let _ = std::fs::remove_dir_all(directory);

    assert!(report.ok);
    assert_eq!(report.engine, "tract-onnx");
    assert_eq!(report.outputs[0].shape, vec![1, 3, 2, 2]);
}

#[cfg(not(windows))]
#[test]
fn runtime_rejects_missing_non_regular_and_wrong_version_workers() {
    let directory = test_directory("onnx-invalid-worker-contract");
    let missing = directory.join("missing-worker");
    assert!(OnnxRuntime::from_worker(&missing).is_err());
    assert!(OnnxRuntime::from_worker(&directory).is_err());

    let wrong_version = directory.join("wrong-version-worker");
    std::fs::write(&wrong_version, "#!/bin/sh\necho open-scanline 0.0.0\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&wrong_version, std::fs::Permissions::from_mode(0o700)).unwrap();
    let error = OnnxRuntime::from_worker(&wrong_version).unwrap_err();
    assert!(error.to_string().contains("version-matched"));
    let _ = std::fs::remove_dir_all(directory);
}

#[cfg(not(windows))]
#[test]
fn runtime_rejects_a_worker_replaced_after_construction() {
    let directory = test_directory("onnx-replaced-worker-contract");
    let worker = directory.join("open-scanline-worker");
    let replacement = directory.join("open-scanline-worker-replacement");
    std::fs::write(&worker, "#!/bin/sh\necho open-scanline 1.0.0\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = OnnxRuntime::from_worker(&worker).unwrap();
    std::fs::write(&replacement, "replaced worker").unwrap();
    std::fs::rename(&replacement, &worker).unwrap();

    let model = directory.join("identity.onnx");
    std::fs::write(&model, identity_onnx()).unwrap();
    let error = runtime.run(&test_image(), &model).unwrap_err();
    assert!(error.to_string().contains("was replaced"));
    let _ = std::fs::remove_dir_all(directory);
}

#[cfg(not(windows))]
#[test]
fn runtime_rejects_same_inode_content_changes_with_restored_metadata() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let directory = test_directory("onnx-mutated-worker-contract");
    let worker = directory.join("open-scanline-worker");
    let original = "#!/bin/sh\n# worker-a\necho open-scanline 1.0.0\n";
    let replacement = "#!/bin/sh\n# worker-b\necho open-scanline 1.0.0\n";
    assert_eq!(original.len(), replacement.len());
    std::fs::write(&worker, original).unwrap();
    std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
    let original_modified = std::fs::metadata(&worker).unwrap().modified().unwrap();
    let runtime = OnnxRuntime::from_worker(&worker).unwrap();

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&worker)
        .unwrap();
    file.write_all(replacement.as_bytes()).unwrap();
    file.sync_all().unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(original_modified))
        .unwrap();

    let model = directory.join("identity.onnx");
    std::fs::write(&model, identity_onnx()).unwrap();
    let error = runtime.run(&test_image(), &model).unwrap_err();
    assert!(error.to_string().contains("was replaced"));
    let _ = std::fs::remove_dir_all(directory);
}

#[cfg(not(windows))]
#[test]
fn one_shot_wrapper_uses_the_explicit_worker_runtime() {
    let directory = test_directory("onnx-one-shot-worker-contract");
    let model = directory.join("identity.onnx");
    std::fs::write(&model, identity_onnx()).unwrap();
    let report = run_user_onnx_with_worker(
        &test_image(),
        &model,
        &OnnxInferenceOptions::default(),
        env!("CARGO_BIN_EXE_open-scanline"),
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(directory);
    assert!(report.ok);
}

#[cfg(not(windows))]
#[test]
#[allow(deprecated)]
fn legacy_api_does_not_discover_a_sibling_worker() {
    let directory = test_directory("onnx-no-auto-discovery-contract");
    let model = directory.join("identity.onnx");
    std::fs::write(&model, identity_onnx()).unwrap();
    let error = run_user_onnx(&test_image(), &model).unwrap_err();
    let _ = std::fs::remove_dir_all(directory);
    assert!(error
        .to_string()
        .contains("automatic worker discovery is disabled"));
}
