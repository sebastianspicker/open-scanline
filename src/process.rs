//! Shared image process path used by CLI `process` and GUI extended actions.

use crate::atomic_write::validate_output_leaf;
use crate::core::{PipelinePrefs, Result, ScanError};
use crate::device::CancellationToken;
use crate::export::{
    apply_export_profile, prepare_export_options, save_final_image_with_cancellation, ExportOptions,
};
use crate::imaging::{load_image, supported_extensions};
use crate::pipeline::apply_pipeline;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessOptions {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub pipeline: PipelinePrefs,
    pub quality: Option<u8>,
}

/// Run a typed load, process, and save request.
pub fn process_image_file(options: &ProcessOptions) -> Result<PathBuf> {
    process_image_file_with_export_options(options, &ExportOptions::default())
}

/// Process an image while allowing command-backed export stages to observe cancellation.
pub fn process_image_file_with_token(
    options: &ProcessOptions,
    token: CancellationToken,
) -> Result<PathBuf> {
    process_image_file_with_export_options_and_token(options, &ExportOptions::default(), token)
}

/// Process an image with runtime-only PDF/OCR/profile export controls.
pub fn process_image_file_with_export_options(
    options: &ProcessOptions,
    export: &ExportOptions,
) -> Result<PathBuf> {
    process_image_file_with_export_options_and_cancellation(options, export, None)
}

/// Process with export options and a shared cancellation token.
pub fn process_image_file_with_export_options_and_token(
    options: &ProcessOptions,
    export: &ExportOptions,
    token: CancellationToken,
) -> Result<PathBuf> {
    process_image_file_with_export_options_and_cancellation(options, export, Some(&token))
}

fn process_image_file_with_export_options_and_cancellation(
    options: &ProcessOptions,
    export: &ExportOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    // Validate destination semantics and profile before writing any output.
    validate_process_destination(&options.dst)?;
    let export = prepare_export_options(&options.dst, export)?;
    let image = load_image(&options.src)?;
    let image = apply_pipeline(&image, &options.pipeline)?;
    let image = apply_export_profile(&image, &export)?;
    save_final_image_with_cancellation(
        &options.dst,
        &image,
        None,
        options.quality,
        &export,
        cancellation,
    )
}

fn validate_process_destination(destination: &std::path::Path) -> Result<()> {
    validate_output_leaf(destination, "process output")?;
    let extension = destination
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| ScanError::Invalid("process output has no supported extension".into()))?;
    if !supported_extensions().contains(&extension.as_str()) {
        return Err(ScanError::Invalid(format!(
            "unsupported process output extension '.{extension}'"
        )));
    }
    Ok(())
}
