//! Public ONNX model facade; implementation stays in focused private modules.

#[cfg(feature = "onnx")]
mod execution;
#[cfg(feature = "onnx")]
mod input;
mod types;
#[cfg(feature = "onnx")]
mod validation;

use crate::domain::image::ImageBuffer;
use crate::error::Result;
use std::path::Path;

pub use types::{
    OnnxInferenceOptions, OnnxInputLayout, OnnxNormalization, OnnxOutputSummary, OnnxReport,
};

/// Run a user-supplied ONNX model in a resource-contained worker process.
///
/// This historical convenience API cannot authenticate a worker without an
/// explicit path, so it deliberately fails closed. Use [`OnnxRuntime`] or
/// [`run_user_onnx_with_worker`] instead.
#[deprecated(
    note = "use OnnxRuntime::from_worker(...).run(...) or run_user_onnx_with_worker with an explicit worker"
)]
pub fn run_user_onnx(image: &ImageBuffer, model_path: impl AsRef<Path>) -> Result<OnnxReport> {
    run_user_onnx_with_options_unavailable(
        image,
        model_path.as_ref(),
        &OnnxInferenceOptions::default(),
    )
}

#[deprecated(
    note = "use OnnxRuntime::from_worker(...).run_with_options(...) or run_user_onnx_with_worker with an explicit worker"
)]
pub fn run_user_onnx_with_options(
    image: &ImageBuffer,
    model_path: impl AsRef<Path>,
    options: &OnnxInferenceOptions,
) -> Result<OnnxReport> {
    run_user_onnx_with_options_unavailable(image, model_path.as_ref(), options)
}

fn run_user_onnx_with_options_unavailable(
    image: &ImageBuffer,
    model_path: &Path,
    options: &OnnxInferenceOptions,
) -> Result<OnnxReport> {
    super::ensure_enabled()?;
    let input = crate::infrastructure::runtime::TemporaryOutput::new("onnx-input", "png")?;
    crate::infrastructure::media::save_image(input.path(), image, None, None)?;
    super::isolation::run_isolated_onnx(input.path(), model_path, options)
}

/// A reusable, explicitly selected ONNX worker runtime.
#[derive(Debug)]
pub struct OnnxRuntime {
    worker: super::OnnxWorker,
}

impl OnnxRuntime {
    /// Bind this runtime to an explicit, version-matched Open Scanline worker.
    pub fn from_worker(worker_executable: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            worker: super::OnnxWorker::from_executable(worker_executable)?,
        })
    }

    /// Run a model using default input options.
    pub fn run(&self, image: &ImageBuffer, model_path: impl AsRef<Path>) -> Result<OnnxReport> {
        self.run_with_options(image, model_path, &OnnxInferenceOptions::default())
    }

    /// Run a model using explicit input options.
    pub fn run_with_options(
        &self,
        image: &ImageBuffer,
        model_path: impl AsRef<Path>,
        options: &OnnxInferenceOptions,
    ) -> Result<OnnxReport> {
        super::ensure_enabled()?;
        let input = crate::infrastructure::runtime::TemporaryOutput::new("onnx-input", "png")?;
        crate::infrastructure::media::save_image(input.path(), image, None, None)?;
        self.worker.run(input.path(), model_path.as_ref(), options)
    }
}

/// Run an ONNX model with an explicit Open Scanline worker executable.
pub fn run_user_onnx_with_worker(
    image: &ImageBuffer,
    model_path: impl AsRef<Path>,
    options: &OnnxInferenceOptions,
    worker_executable: impl AsRef<Path>,
) -> Result<OnnxReport> {
    OnnxRuntime::from_worker(worker_executable)?.run_with_options(image, model_path, options)
}

/// Trusted-only in-process implementation used by the contained worker.
#[cfg(feature = "onnx")]
pub(crate) fn run_trusted_user_onnx_with_options(
    image: &ImageBuffer,
    model_path: impl AsRef<Path>,
    options: &OnnxInferenceOptions,
) -> Result<OnnxReport> {
    execution::run(image, model_path.as_ref(), options)
}
