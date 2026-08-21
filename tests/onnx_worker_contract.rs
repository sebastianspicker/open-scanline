use open_scanline::core::{ImageBuffer, PixelFormat};
use open_scanline::imaging::save_image;
use open_scanline::ml::{run_user_onnx_with_worker, OnnxInferenceOptions};
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

#[test]
fn cli_runs_user_model_in_the_contained_worker() {
    let directory = std::env::temp_dir().join(format!(
        "open-scanline-onnx-worker-contract-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let input = directory.join("input.png");
    let model = directory.join("identity.onnx");
    save_image(
        &input,
        &ImageBuffer::new(
            2,
            2,
            PixelFormat::Rgb8,
            vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
        )
        .unwrap(),
        None,
        None,
    )
    .unwrap();
    std::fs::write(&model, identity_onnx()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_open-scanline"))
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
    let directory = std::env::temp_dir().join(format!(
        "open-scanline-onnx-library-worker-contract-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let model = directory.join("identity.onnx");
    std::fs::write(&model, identity_onnx()).unwrap();
    let image = ImageBuffer::new(
        2,
        2,
        PixelFormat::Rgb8,
        vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
    )
    .unwrap();

    let report = run_user_onnx_with_worker(
        &image,
        &model,
        &OnnxInferenceOptions::default(),
        env!("CARGO_BIN_EXE_open-scanline"),
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(directory);

    assert!(report.ok);
    assert_eq!(report.engine, "tract-onnx");
    assert_eq!(report.outputs[0].shape, vec![1, 3, 2, 2]);
}
