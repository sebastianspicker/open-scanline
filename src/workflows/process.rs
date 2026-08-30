//! Shared image process path used by CLI `process` and GUI extended actions.

use crate::domain::image::ImageBuffer;
use crate::domain::processing::{apply_pipeline, white_balance, PipelinePrefs};
use crate::error::{Result, ScanError};
use crate::workflows::operation::CancellationToken;
use crate::workflows::ports::media::MediaPort;
use crate::workflows::publication::{
    prepare_export_options_with_media, save_final_image_with_searchable_text_with_media,
    ExportOptions, PreparedExportOptions,
};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessOptions {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub pipeline: PipelinePrefs,
    pub quality: Option<u8>,
}

/// Run a typed load, process, and save request.
/// Process a file through explicitly supplied media infrastructure.
pub fn process_image_file_with_export_options_and_cancellation_with_media<M: MediaPort>(
    options: &ProcessOptions,
    export: &ExportOptions,
    cancellation: Option<&CancellationToken>,
    media: &M,
) -> Result<PathBuf> {
    // Validate destination semantics and profile before writing any output.
    validate_process_destination(&options.dst, media)?;
    let export = prepare_export_options_with_media(&options.dst, export, media)?;
    let image = media.load(&options.src)?;
    process_and_publish_page_with_media(
        image,
        PageWorkflowRequest {
            destination: &options.dst,
            pipeline: &options.pipeline,
            extra_white_balance: false,
            dpi: None,
            quality: options.quality,
            export: &export,
            cancellation,
        },
        media,
    )
    .map(|publication| publication.path)
}

/// The shared per-page workflow kernel: processing plan, profile, optional
/// OCR, then publication. Capture and file processing provide acquisition or
/// loading before calling it; batch keeps document assembly outside it.
pub(crate) fn process_and_publish_page_with_media<M: MediaPort>(
    image: ImageBuffer,
    request: PageWorkflowRequest<'_>,
    media: &M,
) -> Result<PagePublication> {
    let mut image = apply_pipeline(&image, request.pipeline)?;
    if request.extra_white_balance {
        image = white_balance(&image)?;
    }
    let image = media.apply_scanner_profile(&image, request.export.profile())?;
    let searchable_text = request
        .export
        .needs_searchable_text()
        .then(|| {
            media.recognize(
                &image,
                request.export.ocr_language(),
                request.export.uses_offline_ocr(),
                request.cancellation,
            )
        })
        .transpose()?;
    let path = save_final_image_with_searchable_text_with_media(
        request.destination,
        &image,
        request.dpi,
        request.quality,
        request.export,
        searchable_text.as_deref(),
        request.cancellation,
        media,
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

fn validate_process_destination<M: MediaPort>(
    destination: &std::path::Path,
    media: &M,
) -> Result<()> {
    media.validate_output_leaf(destination, "process output")?;
    let extension = destination
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| ScanError::Invalid("process output has no supported extension".into()))?;
    if !media.supports_extension(&extension) {
        return Err(ScanError::Invalid(format!(
            "unsupported process output extension '.{extension}'"
        )));
    }
    Ok(())
}
