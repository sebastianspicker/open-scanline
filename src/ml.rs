//! Compatibility facade for ONNX integration and historical ML helpers.

pub use crate::domain::processing::{
    apply_auto_crop, apply_auto_crop_with_params, apply_orientation,
    apply_orientation_with_degrees, auto_crop_bounds, detect_orientation, AutoCropResult,
    OrientationResult,
};
pub use crate::inbound::diagnostics::ml_module_info;
pub use crate::infrastructure::onnx::{
    run_user_onnx, run_user_onnx_with_options, run_user_onnx_with_worker, OnnxInferenceOptions,
    OnnxInputLayout, OnnxNormalization, OnnxOutputSummary, OnnxReport,
};
