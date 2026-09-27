//! Shared image process path used by CLI `process` and GUI extended actions.

use crate::domain::export::ExportOptions;
use crate::domain::image::ImageBuffer;
use crate::domain::processing::{apply_pipeline_owned, white_balance, PipelinePrefs};
use crate::error::Result;
use crate::operation::CancellationToken;
use crate::workflows::publication::{
    apply_export_profile, apply_export_profile_owned, prepare_export_options,
    save_final_image_with_cancellation, save_final_image_with_searchable_text,
    validate_supported_output_path, PreparedExportOptions,
};
use std::path::{Path, PathBuf};

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
    validate_supported_output_path(&options.dst, "process output")?;
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

/// Process a source into `options.dst`, then optionally republish that
/// result through a prepared export profile onto `published_path`.
///
/// GUI-style callers that write a PDF or JPEG XL destination point
/// `options.dst` at a temporary raster first (so a raster copy survives for
/// further interactive work) and supply `prepared_export` to re-encode that
/// raster onto the real `published_path` afterward. Callers that publish
/// directly pass `prepared_export: None` and `options.dst == published_path`.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub(crate) fn process_and_republish(
    options: &ProcessOptions,
    working_export: ExportOptions,
    published_path: &Path,
    dpi: u32,
    prepared_export: Option<&PreparedExportOptions>,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    let raster = process_image_file(
        options,
        ProcessRunOptions {
            export: working_export,
            cancellation: cancellation.cloned(),
        },
    )?;
    let Some(prepared) = prepared_export else {
        return Ok(raster);
    };
    let image = apply_export_profile(
        &crate::infrastructure::media::load_image(&raster)?,
        prepared,
    )?;
    save_final_image_with_cancellation(
        published_path,
        &image,
        Some(dpi),
        None,
        prepared,
        cancellation,
    )
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
