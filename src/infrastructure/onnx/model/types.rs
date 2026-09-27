use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// Tensor arrangement used for a user supplied image model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnnxInputLayout {
    /// Infer the layout from the model's static channel axis; defaults to NCHW.
    #[default]
    Auto,
    Nchw,
    Nhwc,
}

/// Pixel conversion performed before inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OnnxNormalization {
    /// Convert 8-bit source samples to `0.0..=1.0` floating point values.
    #[default]
    ZeroToOne,
    /// Preserve 8-bit sample magnitudes in a floating point tensor.
    None,
}

/// Explicit user-model input policy: first model input, auto layout, and RGB
/// values in `0..1` by default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OnnxInferenceOptions {
    pub input_name: Option<String>,
    pub layout: OnnxInputLayout,
    pub normalization: OnnxNormalization,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OnnxOutputSummary {
    pub shape: Vec<usize>,
    pub datum_type: String,
    pub size: usize,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub mean: Option<f64>,
    pub sample: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OnnxReport {
    pub ok: bool,
    pub engine: String,
    pub model: PathBuf,
    pub input_name: String,
    pub input_shape: Vec<usize>,
    pub layout: OnnxInputLayout,
    pub normalization: OnnxNormalization,
    pub outputs: Vec<OnnxOutputSummary>,
}

impl OnnxReport {
    pub fn as_dict(&self) -> Value {
        serde_json::to_value(self).expect("ONNX inference report is serializable")
    }
}
