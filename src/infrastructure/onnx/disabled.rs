//! Public contracts remain available when the inference runtime is omitted.
use super::{OnnxInferenceOptions, OnnxReport};
use crate::error::Result;
use std::path::Path;

#[derive(Debug)]
pub(crate) struct OnnxWorker;

impl OnnxWorker {
    pub(crate) fn from_executable(_executable: impl AsRef<Path>) -> Result<Self> {
        super::ensure_enabled()?;
        Ok(Self)
    }

    pub(crate) fn run(
        &self,
        input: &Path,
        model: &Path,
        options: &OnnxInferenceOptions,
    ) -> Result<OnnxReport> {
        run_isolated_onnx(input, model, options)
    }
}

pub(crate) fn run_isolated_onnx(
    _input: &Path,
    _model: &Path,
    _options: &OnnxInferenceOptions,
) -> Result<OnnxReport> {
    Err(crate::error::ScanError::Unsupported(
        "ONNX inference is not compiled in; rebuild with the `onnx` feature".into(),
    ))
}

pub(crate) fn run_isolated_onnx_with_executable(
    input: &Path,
    model: &Path,
    options: &OnnxInferenceOptions,
    _executable: &Path,
) -> Result<OnnxReport> {
    run_isolated_onnx(input, model, options)
}

pub(crate) fn run_onnx_worker(
    _input: &Path,
    _model: &Path,
    _report: &Path,
    _options: &OnnxInferenceOptions,
    _worker_protocol: &str,
) -> Result<()> {
    super::ensure_enabled()
}
