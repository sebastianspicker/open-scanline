//! Batch multi-page scan orchestration.

use crate::domain::acquisition::{
    validate_scan_dpi, DeviceOpenPolicy, ScanMode, ScanProgress, ScanRequest,
};
use crate::domain::image::{checked_image_len, PixelFormat};
use crate::domain::processing::PipelinePrefs;
use crate::error::{Result, ScanError};
use crate::workflows::operation::CancellationToken;
use crate::workflows::ports::acquisition::{
    AcquisitionPort, DeviceSession, ScanPagesEnd, MAX_SCAN_PAGES,
};
use crate::workflows::ports::media::MediaPort;
use crate::workflows::process::{process_and_publish_page_with_media, PageWorkflowRequest};
use crate::workflows::publication::{
    prepare_export_options_for_pdf_with_media,
    save_final_pdf_from_paths_with_cancellation_with_media, ExportOptions, PreparedExportOptions,
};
use std::path::{Path, PathBuf};

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

/// Run batch capture through explicitly supplied acquisition and media adapters.
pub fn run_batch_scan_with_export_options_inner_with_ports<A: AcquisitionPort, M: MediaPort>(
    args: BatchScanArgs,
    export: &ExportOptions,
    cancel_check: Option<&BatchCancelCheck>,
    token: Option<CancellationToken>,
    policy: DeviceOpenPolicy,
    acquisition: &A,
    media: &M,
) -> Result<Vec<PathBuf>> {
    if is_batch_cancelled(cancel_check, token.as_ref()) {
        return Err(ScanError::Cancelled("batch scan cancelled".into()));
    }
    let pdf_destinations = pdf_destinations(&args);
    if pdf_destinations.len() > 1 && export_uses_pdf_features(export) {
        return Err(ScanError::Invalid(
            "PDF export options require exactly one multipage PDF destination".into(),
        ));
    }
    let export =
        prepare_export_options_for_pdf_with_media(!pdf_destinations.is_empty(), export, media)?;
    validate_batch_args(&args)?;
    let file_ext = batch_file_extension(&args.format)?;
    validate_batch_destinations(&args, &file_ext, media)?;
    media.prepare_output_directory(&args.out_dir)?;
    let pages = scan_batch_pages_with_export(
        &args,
        &file_ext,
        &export,
        cancel_check,
        token.clone(),
        policy,
        acquisition,
        media,
    )?;
    write_requested_outputs(
        &args,
        &pages.paths,
        &export,
        pages.searchable_text,
        cancel_check,
        token.as_ref(),
        media,
    )?;
    report_batch_completion(&args, pages.paths.len());
    Ok(pages.paths)
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
    if args.pages < 1 {
        return Err(ScanError::Invalid("pages must be >= 1".into()));
    }
    if args.pages > MAX_BATCH_PAGES {
        return Err(ScanError::Invalid(format!(
            "pages must be <= {MAX_BATCH_PAGES}"
        )));
    }
    checked_image_len(args.width, args.height, PixelFormat::Rgb8.bpp())?;
    validate_scan_dpi(args.dpi, args.dpi)?;
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
    if !matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "tif" | "tiff" | "webp" | "bmp" | "gif" | "jxl"
    ) {
        return Err(ScanError::Invalid(format!(
            "unsupported batch format '{ext}'"
        )));
    }
    Ok(if ext == "jpeg" { "jpg".into() } else { ext })
}

fn validate_batch_destinations<M: MediaPort>(
    args: &BatchScanArgs,
    file_ext: &str,
    media: &M,
) -> Result<()> {
    let page_paths = (1..=args.pages)
        .map(|page| {
            let path = args.out_dir.join(format!("page_{page:03}.{file_ext}"));
            media.validate_output_leaf(&path, &format!("generated page {page}"))?;
            Ok(path)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut destinations: Vec<(&str, PathBuf)> = Vec::new();
    if let Some(path) = &args.multipage_out {
        validate_multipage_output_path(path, media)?;
        let effective = if path.extension().is_none() {
            path.with_extension("tif")
        } else {
            path.clone()
        };
        media.validate_output_leaf(&effective, "multipage output")?;
        destinations.push(("multipage output", effective));
    }
    if let Some(path) = &args.multipage_tiff {
        validate_named_container_path(path, "multipage TIFF", &["tif", "tiff"], media)?;
        destinations.push(("multipage TIFF", path.clone()));
    }
    if let Some(path) = &args.multipage_pdf {
        validate_named_container_path(path, "multipage PDF", &["pdf"], media)?;
        destinations.push(("multipage PDF", path.clone()));
    }
    if let Some(path) = &args.contact_sheet {
        let effective = if path.extension().is_none() {
            path.with_extension("bmp")
        } else {
            path.clone()
        };
        validate_image_output_path(&effective, "contact sheet", media)?;
        destinations.push(("contact sheet", effective));
    }

    for index in 0..destinations.len() {
        let (label, path) = &destinations[index];
        for (other_label, other_path) in &destinations[..index] {
            if media.output_paths_alias(path, other_path)? {
                return Err(ScanError::Invalid(format!(
                    "{label} destination aliases {other_label}: {}",
                    path.display()
                )));
            }
        }
        for (page_index, page_path) in page_paths.iter().enumerate() {
            if media.output_paths_alias(path, page_path)? {
                let page = page_index + 1;
                return Err(ScanError::Invalid(format!(
                    "{label} destination aliases generated page {page}: {}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

fn validate_multipage_output_path<M: MediaPort>(path: &Path, media: &M) -> Result<()> {
    if path.extension().is_none() {
        return Ok(());
    }
    validate_named_container_path(path, "multipage output", &["pdf", "tif", "tiff"], media)
}

fn validate_named_container_path<M: MediaPort>(
    path: &Path,
    label: &str,
    allowed: &[&str],
    media: &M,
) -> Result<()> {
    media.validate_output_leaf(path, label)?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| ScanError::Invalid(format!("{label} has no supported extension")))?;
    if !allowed.contains(&extension.as_str()) {
        return Err(ScanError::Invalid(format!(
            "unsupported {label} extension '.{extension}'"
        )));
    }
    Ok(())
}

fn validate_image_output_path<M: MediaPort>(path: &Path, label: &str, media: &M) -> Result<()> {
    media.validate_output_leaf(path, label)?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| ScanError::Invalid(format!("{label} has no supported extension")))?;
    if !media.supports_extension(&extension) {
        return Err(ScanError::Invalid(format!(
            "unsupported {label} extension '.{extension}'"
        )));
    }
    Ok(())
}

struct BatchPages {
    paths: Vec<PathBuf>,
    searchable_text: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
fn scan_batch_pages_with_export(
    args: &BatchScanArgs,
    file_ext: &str,
    export: &PreparedExportOptions,
    cancel_check: Option<&BatchCancelCheck>,
    token: Option<CancellationToken>,
    policy: DeviceOpenPolicy,
    acquisition: &impl AcquisitionPort,
    media: &impl MediaPort,
) -> Result<BatchPages> {
    let device_id = acquisition.resolve_device_id(Some(&args.device));
    let session = acquisition.open_device_with_policy(&device_id, policy)?;
    if let Some(token) = token.as_ref() {
        session.bind_cancellation(token.clone());
    }
    let result = scan_batch_pages_with_session_export(
        args,
        file_ext,
        &device_id,
        &*session,
        export,
        cancel_check,
        token.as_ref(),
        media,
    );
    session.close();
    result
}

#[allow(clippy::too_many_arguments)]
fn scan_batch_pages_with_session_export(
    args: &BatchScanArgs,
    file_ext: &str,
    device_id: &str,
    session: &(impl DeviceSession + ?Sized),
    export: &PreparedExportOptions,
    cancel_check: Option<&BatchCancelCheck>,
    token: Option<&CancellationToken>,
    media: &impl MediaPort,
) -> Result<BatchPages> {
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
    cancel_batch_if_requested(session, cancel_check, token)?;
    session.set_params(&request)?;
    let mut paths: Vec<PathBuf> = Vec::with_capacity(args.pages as usize);
    let mut searchable_pages = Vec::with_capacity(args.pages as usize);
    let mut searchable_text_bytes = 0_usize;
    let summary = session.scan_pages(&request, args.pages, &mut |image| {
        cancel_batch_if_requested(session, cancel_check, token)?;
        let index = paths.len() as u32;
        if let Some(ref cb) = args.on_progress {
            cb(ScanProgress::new(
                "batch",
                index as f64 / args.pages as f64,
                format!("page {}/{}", index + 1, args.pages),
            ));
        }
        let page_path = args
            .out_dir
            .join(format!("page_{:03}.{}", index + 1, file_ext));
        let publication = process_and_publish_page_with_media(
            image,
            PageWorkflowRequest {
                destination: &page_path,
                pipeline: &pipeline,
                extra_white_balance: false,
                dpi: Some(args.dpi),
                quality: None,
                export,
                cancellation: token,
            },
            media,
        )?;
        if let Some(text) = publication.searchable_text {
            searchable_text_bytes = media.checked_pdf_searchable_text_total(
                searchable_text_bytes,
                &text,
                searchable_pages.len(),
            )?;
            searchable_pages.push(text);
        }
        paths.push(publication.path);
        Ok(())
    })?;
    cancel_batch_if_requested(session, cancel_check, token)?;
    if summary.emitted != paths.len() as u32 {
        return Err(ScanError::Other(format!(
            "scanner reported {} pages but emitted {}",
            summary.emitted,
            paths.len()
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
        ScanPagesEnd::FeederExhausted if args.duplex && summary.emitted % 2 != 0 => {
            return Err(ScanError::Other(format!(
                "document feeder ended after an incomplete duplex pair ({} sides)",
                summary.emitted
            )));
        }
        _ => {}
    }
    Ok(BatchPages {
        paths,
        searchable_text: searchable_pages,
    })
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

fn write_requested_outputs<M: MediaPort>(
    args: &BatchScanArgs,
    paths: &[PathBuf],
    export: &PreparedExportOptions,
    searchable_pages: Vec<String>,
    cancel_check: Option<&BatchCancelCheck>,
    token: Option<&CancellationToken>,
    media: &M,
) -> Result<()> {
    check_output_cancellation(cancel_check, token)?;
    if let Some(ref mp) = args.multipage_out {
        if mp
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
        {
            save_final_pdf_from_paths_with_cancellation_with_media(
                mp,
                paths,
                args.dpi,
                export,
                searchable_pages.clone(),
                token,
                media,
            )?;
        } else {
            write_multipage(paths, mp, args.dpi, token, media)?;
        }
    }
    check_output_cancellation(cancel_check, token)?;
    if let Some(ref tiff) = args.multipage_tiff {
        media.publish_tiff_from_paths(paths, tiff, Some(args.dpi), None, token)?;
    }
    check_output_cancellation(cancel_check, token)?;
    if let Some(ref pdf) = args.multipage_pdf {
        save_final_pdf_from_paths_with_cancellation_with_media(
            pdf,
            paths,
            args.dpi,
            export,
            searchable_pages,
            token,
            media,
        )?;
    }
    check_output_cancellation(cancel_check, token)?;
    if let Some(ref sheet) = args.contact_sheet {
        media.publish_contact_sheet(paths, sheet, token)?;
    }
    Ok(())
}

fn check_output_cancellation(
    cancel_check: Option<&BatchCancelCheck>,
    token: Option<&CancellationToken>,
) -> Result<()> {
    if is_batch_cancelled(cancel_check, token) {
        return Err(ScanError::Cancelled("batch scan cancelled".into()));
    }
    Ok(())
}

fn report_batch_completion(args: &BatchScanArgs, page_count: usize) {
    if let Some(ref cb) = args.on_progress {
        cb(ScanProgress::new(
            "done",
            1.0,
            format!("{page_count} pages"),
        ));
    }
}

fn write_multipage<M: MediaPort>(
    paths: &[PathBuf],
    out: &Path,
    dpi: u32,
    token: Option<&CancellationToken>,
    media: &M,
) -> Result<PathBuf> {
    let ext = out
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("tif")
        .to_ascii_lowercase();
    if ext == "pdf" {
        return media.publish_pdf_from_paths(
            paths,
            out,
            dpi,
            "open-scanline multipage",
            None,
            None,
            None,
            token,
        );
    }
    if matches!(ext.as_str(), "tif" | "tiff") || out.extension().is_none() {
        let dest = if out.extension().is_none() {
            out.with_extension("tif")
        } else {
            out.to_path_buf()
        };
        return media.publish_tiff_from_paths(paths, &dest, Some(dpi), None, token);
    }
    Err(ScanError::Invalid(format!(
        "multipage output must use .pdf, .tif, or .tiff, not '.{ext}'"
    )))
}
