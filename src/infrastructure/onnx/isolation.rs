//! Process containment facade for user-supplied ONNX workloads.

mod launch;
mod limits;
mod paths;
mod worker_image;

use super::model::run_trusted_user_onnx_with_options;
use super::OnnxInferenceOptions;
use crate::error::{Result, ScanError};
use std::path::Path;

pub(crate) const ONNX_WORKER_PROTOCOL: &str = concat!(env!("CARGO_PKG_VERSION"), ":1");
pub(crate) use worker_image::OnnxWorker;

pub(crate) fn run_isolated_onnx(
    input: &Path,
    model: &Path,
    options: &OnnxInferenceOptions,
) -> Result<super::OnnxReport> {
    paths::validate_onnx_paths(input, model)?;
    let _ = options;
    Err(paths::worker_unavailable("automatic worker discovery is disabled because a current-executable-derived path cannot authenticate the worker image; use run_user_onnx_with_worker with an explicit worker path"))
}

pub(crate) fn run_isolated_onnx_with_executable(
    input: &Path,
    model: &Path,
    options: &OnnxInferenceOptions,
    executable: &Path,
) -> Result<super::OnnxReport> {
    paths::validate_onnx_paths(input, model)?;
    OnnxWorker::from_executable(executable)?.run(input, model, options)
}

pub(crate) fn run_onnx_worker(
    input: &Path,
    model: &Path,
    report: &Path,
    options: &OnnxInferenceOptions,
    worker_protocol: &str,
) -> Result<()> {
    if worker_protocol != ONNX_WORKER_PROTOCOL {
        return Err(ScanError::Unsupported(format!(
            "ONNX worker protocol mismatch: expected {ONNX_WORKER_PROTOCOL}"
        )));
    }
    launch::start_parent_liveness_watchdog();
    limits::apply_worker_limits()?;
    limits::set_worker_thread_limits();
    let image = crate::infrastructure::media::load_image(input)?;
    let result = run_trusted_user_onnx_with_options(&image, model, options)?;
    let json = serde_json::to_vec(&result)
        .map_err(|error| ScanError::Other(format!("could not serialize ONNX report: {error}")))?;
    paths::check_report_size(json.len() as u64)?;
    crate::infrastructure::runtime::atomic_publish::write_file_atomic(report, &json)
}
