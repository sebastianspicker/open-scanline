//! User-supplied ONNX inference and contained worker execution.

#[cfg(not(feature = "onnx"))]
mod disabled;
#[cfg(feature = "onnx")]
mod isolation;
#[cfg(not(feature = "onnx"))]
use disabled as isolation;
mod model;

pub(crate) use isolation::{run_isolated_onnx_with_executable, run_onnx_worker, OnnxWorker};
#[allow(deprecated)]
pub use model::{
    run_user_onnx, run_user_onnx_with_options, run_user_onnx_with_worker, OnnxInferenceOptions,
    OnnxInputLayout, OnnxNormalization, OnnxOutputSummary, OnnxReport, OnnxRuntime,
};

fn ensure_enabled() -> crate::error::Result<()> {
    if cfg!(feature = "onnx") {
        Ok(())
    } else {
        Err(crate::error::ScanError::Unsupported(
            "ONNX inference is not compiled in; rebuild with the `onnx` feature".into(),
        ))
    }
}
