//! Native entry wrappers for file processing.

use crate::composition::Runtime;
use crate::error::Result;
use crate::workflows::operation::CancellationToken;
use crate::workflows::process::process_image_file_with_export_options_and_cancellation_with_media;
pub use crate::workflows::process::ProcessOptions;
use crate::workflows::publication::ExportOptions;
use std::path::PathBuf;

pub fn process_image_file(options: &ProcessOptions) -> Result<PathBuf> {
    process_image_file_with_export_options(options, &ExportOptions::default())
}

pub fn process_image_file_with_token(
    options: &ProcessOptions,
    token: CancellationToken,
) -> Result<PathBuf> {
    process_image_file_with_export_options_and_token(options, &ExportOptions::default(), token)
}

pub fn process_image_file_with_export_options(
    options: &ProcessOptions,
    export: &ExportOptions,
) -> Result<PathBuf> {
    process_image_file_with_runtime(options, export, None)
}

pub fn process_image_file_with_export_options_and_token(
    options: &ProcessOptions,
    export: &ExportOptions,
    token: CancellationToken,
) -> Result<PathBuf> {
    process_image_file_with_runtime(options, export, Some(&token))
}

fn process_image_file_with_runtime(
    options: &ProcessOptions,
    export: &ExportOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    let runtime = Runtime::default();
    process_image_file_with_export_options_and_cancellation_with_media(
        options,
        export,
        cancellation,
        runtime.media(),
    )
}
