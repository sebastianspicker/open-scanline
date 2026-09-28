//! Batch multi-page scan orchestration.

mod destinations;
pub(crate) mod outputs;
mod plan;

use crate::domain::acquisition::MAX_SCAN_PAGES;
use crate::domain::acquisition::{
    validate_scan_dpi, DeviceOpenPolicy, ScanMode, ScanProgress, ScanRequest,
};
use crate::domain::export::ExportOptions;
use crate::domain::image::{checked_image_len, ImageBuffer, PixelFormat};
use crate::domain::processing::PipelinePrefs;
use crate::error::{Result, ScanError};
use crate::infrastructure::acquisition::{DeviceSession, ScanPagesEnd, ScanPagesResult};
use crate::operation::CancellationToken;
use crate::workflows::process::{process_and_publish_page, PageWorkflowRequest};
use crate::workflows::publication::{prepare_export_options_for_pdf, PreparedExportOptions};
use destinations::validate_batch_destinations;
use outputs::{
    check_output_cancellation, report_batch_completion, write_requested_outputs_with_observer,
};
pub(crate) use plan::{plan_batch_outputs, BatchOutputRequest};
use std::path::PathBuf;

/// A file whose publication completed during a batch workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BatchPublishedOutput {
    pub(crate) kind: BatchPublishedOutputKind,
    pub(crate) path: PathBuf,
}

/// The role of a successfully published batch file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BatchPublishedOutputKind {
    Page,
    Document,
    ContactSheet,
}

/// Trustworthy batch events emitted after the represented operation completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BatchWorkflowEvent {
    Published(BatchPublishedOutput),
    PublishingOutputs,
}

pub(crate) type BatchEventObserver = dyn Fn(BatchWorkflowEvent);

#[derive(Clone, Copy)]
pub(super) struct BatchRunHooks<'a> {
    pub(super) cancel_check: Option<&'a BatchCancelCheck>,
    pub(super) token: Option<&'a CancellationToken>,
    pub(super) observer: Option<&'a BatchEventObserver>,
}

/// Detailed successful result for GUI reporting without changing the public facade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BatchScanReport {
    pub(crate) page_paths: Vec<PathBuf>,
    pub(crate) end: ScanPagesEnd,
}

/// Extended batch arguments.
///
/// The limit counts logical sides, including both sides of duplex sheets.
pub const MAX_BATCH_PAGES: u32 = MAX_SCAN_PAGES;

/// Cooperative cancellation hook for batch acquisition.
///
/// This remains separate from [`BatchScanArgs`] so existing public struct
/// literals stay source-compatible.
pub type BatchCancelCheck = dyn Fn() -> bool + Send + Sync;

pub struct BatchScanArgs {
    pub device: String,
    pub out_dir: PathBuf,
    pub pages: u32,
    pub width: u32,
    pub height: u32,
    pub seed: u32,
    pub dpi: u32,
    pub mode: ScanMode,
    pub duplex: bool,
    pub format: String,
    pub multipage_tiff: Option<PathBuf>,
    pub multipage_pdf: Option<PathBuf>,
    pub multipage_out: Option<PathBuf>,
    pub contact_sheet: Option<PathBuf>,
    pub pipeline: PipelinePrefs,
    pub on_progress: Option<Box<dyn Fn(ScanProgress) + Send>>,
}

impl Default for BatchScanArgs {
    fn default() -> Self {
        Self {
            device: "mock".into(),
            out_dir: PathBuf::from("batch_out"),
            pages: 2,
            width: 320,
            height: 240,
            seed: 1,
            dpi: 150,
            mode: ScanMode::Reflective,
            duplex: false,
            format: "png".into(),
            multipage_tiff: None,
            multipage_pdf: None,
            multipage_out: None,
            contact_sheet: None,
            pipeline: PipelinePrefs::default(),
            on_progress: None,
        }
    }
}

/// Runtime controls for the canonical batch-capture entry.
#[derive(Default)]
pub(crate) struct BatchCaptureOptions<'a> {
    pub(crate) export: ExportOptions,
    pub(crate) cancel_check: Option<&'a BatchCancelCheck>,
    pub(crate) token: Option<CancellationToken>,
    pub(crate) policy: DeviceOpenPolicy,
    pub(crate) observer: Option<&'a BatchEventObserver>,
}

/// Run batch capture through the native acquisition and media adapters.
pub(crate) fn run_batch_scan(
    args: BatchScanArgs,
    options: BatchCaptureOptions<'_>,
) -> Result<Vec<PathBuf>> {
    run_batch_scan_with_report(args, options).map(|report| report.page_paths)
}

/// Run a batch while reporting only successfully published files.
pub(crate) fn run_batch_scan_with_report(
    args: BatchScanArgs,
    options: BatchCaptureOptions<'_>,
) -> Result<BatchScanReport> {
    let BatchCaptureOptions {
        export,
        cancel_check,
        token,
        policy,
        observer,
    } = options;
    let hooks = BatchRunHooks {
        cancel_check,
        token: token.as_ref(),
        observer,
    };
    check_output_cancellation(hooks.cancel_check, hooks.token)?;
    let prepared = prepare_batch(&args, &export)?;
    let pages =
        scan_batch_pages_with_export(&args, &prepared.file_ext, &prepared.export, policy, hooks)?;
    if let Some(observer) = hooks.observer {
        observer(BatchWorkflowEvent::PublishingOutputs);
    }
    write_requested_outputs_with_observer(
        &args,
        &pages.paths,
        &prepared.export,
        pages.searchable_text,
        hooks,
    )?;
    report_batch_completion(&args, pages.paths.len());
    Ok(BatchScanReport {
        page_paths: pages.paths,
        end: pages.end,
    })
}

struct PreparedBatch {
    export: PreparedExportOptions,
    file_ext: String,
}

fn prepare_batch(args: &BatchScanArgs, export: &ExportOptions) -> Result<PreparedBatch> {
    let pdf_destinations = pdf_destinations(args);
    if pdf_destinations.len() > 1 && export_uses_pdf_features(export) {
        return Err(ScanError::Invalid(
            "PDF export options require exactly one multipage PDF destination".into(),
        ));
    }
    let export = prepare_export_options_for_pdf(!pdf_destinations.is_empty(), export)?;
    validate_batch_args(args)?;
    let file_ext = batch_file_extension(&args.format)?;
    validate_batch_destinations(args, &file_ext)?;
    crate::infrastructure::media::prepare_output_directory(&args.out_dir)?;
    Ok(PreparedBatch { export, file_ext })
}

fn export_uses_pdf_features(export: &ExportOptions) -> bool {
    export.searchable_pdf || export.pdf_password.is_some()
}

fn pdf_destinations(args: &BatchScanArgs) -> Vec<&PathBuf> {
    let mut destinations = args.multipage_pdf.iter().collect::<Vec<_>>();
    if let Some(path) = args.multipage_out.as_ref().filter(|path| {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
    }) {
        destinations.push(path);
    }
    destinations
}

fn validate_batch_args(args: &BatchScanArgs) -> Result<()> {
    validate_batch_page_count(args.pages)?;
    checked_image_len(args.width, args.height, PixelFormat::Rgb8.bpp())?;
    validate_scan_dpi(args.dpi, args.dpi)?;
    validate_batch_duplex(args)
}

fn validate_batch_page_count(pages: u32) -> Result<()> {
    if pages < 1 {
        return Err(ScanError::Invalid("pages must be >= 1".into()));
    }
    if pages > MAX_BATCH_PAGES {
        return Err(ScanError::Invalid(format!(
            "pages must be <= {MAX_BATCH_PAGES}"
        )));
    }
    Ok(())
}

fn validate_batch_duplex(args: &BatchScanArgs) -> Result<()> {
    if args.duplex && args.mode != ScanMode::Document {
        return Err(ScanError::Invalid(
            "duplex batch acquisition requires the document/ADF source".into(),
        ));
    }
    if args.duplex && !args.pages.is_multiple_of(2) {
        return Err(ScanError::Invalid(
            "duplex batch page count is logical sides and must be even".into(),
        ));
    }
    Ok(())
}

fn batch_file_extension(format: &str) -> Result<String> {
    let mut ext = format.to_ascii_lowercase();
    if let Some(stripped) = ext.strip_prefix('.') {
        ext = stripped.to_string();
    }
    // Deliberately narrower than `infrastructure::media::supported_extensions()`:
    // a batch page is always a raster image, so "pdf" (a container format) is
    // never a valid per-page extension even though it is a supported output.
    if ext == "pdf" || !crate::infrastructure::media::supported_extensions().contains(&ext.as_str())
    {
        return Err(ScanError::Invalid(format!(
            "unsupported batch format '{ext}'"
        )));
    }
    Ok(if ext == "jpeg" { "jpg".into() } else { ext })
}

struct BatchPages {
    paths: Vec<PathBuf>,
    searchable_text: Vec<String>,
    end: ScanPagesEnd,
}

fn scan_batch_pages_with_export(
    args: &BatchScanArgs,
    file_ext: &str,
    export: &PreparedExportOptions,
    policy: DeviceOpenPolicy,
    hooks: BatchRunHooks<'_>,
) -> Result<BatchPages> {
    let device_id = crate::infrastructure::acquisition::resolve_device_id(Some(&args.device));
    let session = crate::infrastructure::acquisition::open_device_with_policy(&device_id, policy)?;
    if let Some(token) = hooks.token {
        session.bind_cancellation(token.clone());
    }
    let result =
        scan_batch_pages_with_session_export(args, file_ext, &device_id, &session, export, hooks);
    session.close();
    result
}

fn scan_batch_pages_with_session_export(
    args: &BatchScanArgs,
    file_ext: &str,
    device_id: &str,
    session: &(impl DeviceSession + ?Sized),
    export: &PreparedExportOptions,
    hooks: BatchRunHooks<'_>,
) -> Result<BatchPages> {
    let (request, pipeline) = batch_scan_request(args, device_id);
    cancel_batch_if_requested(session, hooks.cancel_check, hooks.token)?;
    session.set_params(&request)?;
    let mut collector = BatchPageCollector::new(args, file_ext, session, export, pipeline, hooks);
    let summary = session.scan_pages(&request, args.pages, &mut |image| collector.add(image))?;
    collector.finish(summary)
}

fn batch_scan_request(args: &BatchScanArgs, device_id: &str) -> (ScanRequest, PipelinePrefs) {
    let mut pipeline = args.pipeline.clone();
    let region = pipeline.crop.take();
    let request = ScanRequest {
        device_id: device_id.into(),
        mode: args.mode,
        duplex: args.duplex,
        dpi_x: args.dpi,
        dpi_y: args.dpi,
        width: args.width,
        height: args.height,
        pixel_format: PixelFormat::Rgb8,
        region,
        seed: args.seed,
        pipeline: pipeline.clone(),
    };
    (request, pipeline)
}

struct BatchPageCollector<'a, S: DeviceSession + ?Sized> {
    args: &'a BatchScanArgs,
    file_ext: &'a str,
    session: &'a S,
    export: &'a PreparedExportOptions,
    hooks: BatchRunHooks<'a>,
    pipeline: PipelinePrefs,
    paths: Vec<PathBuf>,
    searchable_pages: Vec<String>,
    searchable_text_bytes: usize,
}

impl<'a, S: DeviceSession + ?Sized> BatchPageCollector<'a, S> {
    fn new(
        args: &'a BatchScanArgs,
        file_ext: &'a str,
        session: &'a S,
        export: &'a PreparedExportOptions,
        pipeline: PipelinePrefs,
        hooks: BatchRunHooks<'a>,
    ) -> Self {
        Self {
            args,
            file_ext,
            session,
            export,
            hooks,
            pipeline,
            paths: Vec::with_capacity(args.pages as usize),
            searchable_pages: Vec::with_capacity(args.pages as usize),
            searchable_text_bytes: 0,
        }
    }

    fn add(&mut self, image: ImageBuffer) -> Result<()> {
        cancel_batch_if_requested(self.session, self.hooks.cancel_check, self.hooks.token)?;
        let index = self.paths.len() as u32;
        self.report_progress(index);
        let page_path = self
            .args
            .out_dir
            .join(format!("page_{:03}.{}", index + 1, self.file_ext));
        let publication = process_and_publish_page(
            image,
            PageWorkflowRequest {
                destination: &page_path,
                pipeline: &self.pipeline,
                extra_white_balance: false,
                dpi: Some(self.args.dpi),
                quality: None,
                export: self.export,
                cancellation: self.hooks.token,
            },
        )?;
        self.paths.push(publication.path);
        if let (Some(observer), Some(path)) = (self.hooks.observer, self.paths.last()) {
            observer(BatchWorkflowEvent::Published(BatchPublishedOutput {
                kind: BatchPublishedOutputKind::Page,
                path: path.clone(),
            }));
        }
        if let Some(text) = publication.searchable_text {
            self.searchable_text_bytes =
                crate::infrastructure::media::checked_pdf_searchable_text_total(
                    self.searchable_text_bytes,
                    &text,
                    self.searchable_pages.len(),
                )?;
            self.searchable_pages.push(text);
        }
        Ok(())
    }

    fn report_progress(&self, index: u32) {
        if let Some(ref callback) = self.args.on_progress {
            callback(ScanProgress::new(
                "batch",
                index as f64 / self.args.pages as f64,
                format!("page {}/{}", index + 1, self.args.pages),
            ));
        }
    }

    fn finish(self, summary: ScanPagesResult) -> Result<BatchPages> {
        cancel_batch_if_requested(self.session, self.hooks.cancel_check, self.hooks.token)?;
        validate_scan_summary(self.args, &summary, self.paths.len())?;
        Ok(BatchPages {
            paths: self.paths,
            searchable_text: self.searchable_pages,
            end: summary.end,
        })
    }
}

fn validate_scan_summary(
    args: &BatchScanArgs,
    summary: &ScanPagesResult,
    emitted_paths: usize,
) -> Result<()> {
    if summary.emitted != emitted_paths as u32 {
        return Err(ScanError::Other(format!(
            "scanner reported {} pages but emitted {}",
            summary.emitted, emitted_paths
        )));
    }
    match summary.end {
        ScanPagesEnd::LimitReached if summary.emitted != args.pages => {
            return Err(ScanError::Other(format!(
                "scanner stopped after {} of {} requested pages",
                summary.emitted, args.pages
            )));
        }
        ScanPagesEnd::FeederExhausted if summary.emitted == 0 => {
            return Err(ScanError::Unsupported("document feeder is empty".into()));
        }
        ScanPagesEnd::FeederExhausted if args.duplex && !summary.emitted.is_multiple_of(2) => {
            return Err(ScanError::Other(format!(
                "document feeder ended after an incomplete duplex pair ({} sides)",
                summary.emitted
            )));
        }
        _ => {}
    }
    Ok(())
}

fn cancel_batch_if_requested(
    session: &(impl DeviceSession + ?Sized),
    cancel_check: Option<&BatchCancelCheck>,
    token: Option<&CancellationToken>,
) -> Result<()> {
    if is_batch_cancelled(cancel_check, token) {
        session.cancel();
        return Err(ScanError::Cancelled("batch scan cancelled".into()));
    }
    Ok(())
}

fn is_batch_cancelled(
    cancel_check: Option<&BatchCancelCheck>,
    token: Option<&CancellationToken>,
) -> bool {
    token.is_some_and(CancellationToken::is_cancelled) || cancel_check.is_some_and(|check| check())
}
