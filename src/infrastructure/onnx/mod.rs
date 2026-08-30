//! User-supplied ONNX inference and contained worker execution.

mod isolation;
mod model;

pub(crate) use isolation::{run_isolated_onnx_with_executable, run_onnx_worker};
pub use model::{
    run_user_onnx, run_user_onnx_with_options, run_user_onnx_with_worker, OnnxInferenceOptions,
    OnnxInputLayout, OnnxNormalization, OnnxOutputSummary, OnnxReport,
};
