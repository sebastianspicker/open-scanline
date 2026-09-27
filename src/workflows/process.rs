//! Shared image process path used by CLI `process` and GUI extended actions.

use crate::domain::export::ExportOptions;
use crate::domain::image::ImageBuffer;
use crate::domain::processing::{apply_pipeline_owned, white_balance, PipelinePrefs};
use crate::error::{Result, ScanError};
use crate::operation::CancellationToken;
use crate::workflows::publication::{
    apply_export_profile_owned, prepare_export_options, save_final_image_with_searchable_text,
    PreparedExportOptions,
};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessOptions {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub pipeline: PipelinePrefs,
    pub quality: Option<u8>,
}

/// Runtime controls for the canonical process entry.
#[derive(Default)]
pub(crate) struct ProcessRunOptions {
    pub(crate) export: ExportOptions,
    pub(crate) cancellation: Option<CancellationToken>,
}

/// Run a typed load, process, and save request against the native adapters.
pub(crate) fn process_image_file(
    options: &ProcessOptions,
    run: ProcessRunOptions,
) -> Result<PathBuf> {
    // Validate destination semantics and profile before writing any output.
    validate_process_destination(&options.dst)?;
    let export = prepare_export_options(&options.dst, &run.export)?;
    let image = crate::infrastructure::media::load_image(&options.src)?;
    process_and_publish_page(
        image,
        PageWorkflowRequest {
            destination: &options.dst,
            pipeline: &options.pipeline,
            extra_white_balance: false,
            dpi: None,
            quality: options.quality,
            export: &export,
            cancellation: run.cancellation.as_ref(),
        },
    )
    .map(|publication| publication.path)
}

/// The shared per-page workflow kernel: processing plan, profile, optional
/// OCR, then publication. Capture and file processing provide acquisition or
/// loading before calling it; batch keeps document assembly outside it.
pub(crate) fn process_and_publish_page(
    image: ImageBuffer,
    request: PageWorkflowRequest<'_>,
) -> Result<PagePublication> {
    let mut image = apply_pipeline_owned(image, request.pipeline)?;
    if request.extra_white_balance {
        image = white_balance(&image)?;
    }
    let image = apply_export_profile_owned(image, request.export)?;
    let searchable_text = request.export.recognize(&image, request.cancellation)?;
    let path = save_final_image_with_searchable_text(
        request.destination,
        &image,
        request.dpi,
        request.quality,
        request.export,
        searchable_text.as_deref(),
        request.cancellation,
    )?;
    Ok(PagePublication {
        path,
        searchable_text,
    })
}

/// Inputs supplied by an individual capture or file-processing use case to
/// the common page workflow kernel.
pub(crate) struct PageWorkflowRequest<'a> {
    pub(crate) destination: &'a std::path::Path,
    pub(crate) pipeline: &'a PipelinePrefs,
    pub(crate) extra_white_balance: bool,
    pub(crate) dpi: Option<u32>,
    pub(crate) quality: Option<u8>,
    pub(crate) export: &'a PreparedExportOptions,
    pub(crate) cancellation: Option<&'a CancellationToken>,
}

pub(crate) struct PagePublication {
    pub(crate) path: PathBuf,
    pub(crate) searchable_text: Option<String>,
}

fn validate_process_destination(destination: &std::path::Path) -> Result<()> {
    crate::infrastructure::runtime::atomic_publish::validate_output_leaf(
        destination,
        "process output",
    )?;
    let extension = destination
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| ScanError::Invalid("process output has no supported extension".into()))?;
    if !crate::infrastructure::media::supported_extensions().contains(&extension.as_str()) {
        return Err(ScanError::Invalid(format!(
            "unsupported process output extension '.{extension}'"
        )));
    }
    Ok(())
}
