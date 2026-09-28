use crate::error::{Result, ScanError};
use std::path::Path;

pub(super) const MAX_WORKER_REPORT_BYTES: u64 = 1024 * 1024;

pub(super) fn validate_onnx_paths(input: &Path, model: &Path) -> Result<()> {
    require_file(input, "ONNX input image")?;
    require_file(model, "ONNX model")
}

fn require_file(path: &Path, label: &str) -> Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        Err(ScanError::Invalid(format!(
            "{label} not found: {}",
            path.display()
        )))
    }
}

pub(super) fn worker_unavailable(reason: impl std::fmt::Display) -> ScanError {
    ScanError::Unsupported(format!(
        "no trusted Open Scanline ONNX worker is available: {reason}"
    ))
}

pub(super) fn check_report_size(size: u64) -> Result<()> {
    if size > MAX_WORKER_REPORT_BYTES {
        Err(ScanError::Other(
            "ONNX worker report exceeded its size limit".into(),
        ))
    } else {
        Ok(())
    }
}

pub(super) fn working_directory(executable: &Path) -> Result<&Path> {
    if !executable.is_absolute() {
        return Err(ScanError::Invalid(format!(
            "ONNX worker executable must be absolute: {}",
            executable.display()
        )));
    }
    executable.parent().ok_or_else(|| {
        ScanError::Invalid(format!(
            "ONNX worker executable has no parent directory: {}",
            executable.display()
        ))
    })
}
