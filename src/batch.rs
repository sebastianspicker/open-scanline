//! Batch multi-page scan orchestration.

use crate::atomic_write::{output_paths_alias, validate_output_leaf};
use crate::core::{
    PipelinePrefs, PixelFormat, Result, ScanError, ScanMode, ScanProgress, ScanRequest,
};
use crate::device::{
    open_device_with_policy, resolve_device_id, CancellationToken, DeviceOpenPolicy, DeviceSession,
    ScanPagesEnd,
};
use crate::export::{
    apply_export_profile, prepare_export_options_for_pdf,
    save_final_pdf_from_paths_with_cancellation, searchable_text_with_cancellation, ExportOptions,
    PreparedExportOptions,
};
use crate::imaging::{
    save_image_with_cancellation, save_index_contact_sheet_with_cancellation,
    save_multipage_pdf_with_cancellation, save_multipage_tiff_with_cancellation, PdfOptions,
};
use crate::pipeline::apply_pipeline;
use std::path::{Path, PathBuf};

/// Extended batch arguments.
///
/// The limit counts logical sides, including both sides of duplex sheets.
pub const MAX_BATCH_PAGES: u32 = crate::device::MAX_SCAN_PAGES;

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

/// Full batch path with multipage containers.
pub fn run_batch_scan(args: BatchScanArgs) -> Result<Vec<PathBuf>> {
    run_batch_scan_with_export_options_and_token(
        args,
        &ExportOptions::default(),
        CancellationToken::new(),
    )
}

/// Full batch path bound to a shared cancellation signal.
pub fn run_batch_scan_with_token(
    args: BatchScanArgs,
    token: CancellationToken,
) -> Result<Vec<PathBuf>> {
    run_batch_scan_with_export_options_and_token(args, &ExportOptions::default(), token)
}

/// Batch scan with runtime-only PDF/OCR/profile export controls.
pub fn run_batch_scan_with_export_options(
    args: BatchScanArgs,
    export: &ExportOptions,
) -> Result<Vec<PathBuf>> {
    run_batch_scan_with_export_options_and_token(args, export, CancellationToken::new())
}

/// Batch scan with export controls and a token bound directly to the session.
pub fn run_batch_scan_with_export_options_and_token(
    args: BatchScanArgs,
    export: &ExportOptions,
    token: CancellationToken,
) -> Result<Vec<PathBuf>> {
    run_batch_scan_with_export_options_and_token_and_policy(
        args,
        export,
        token,
        DeviceOpenPolicy::default(),
    )
}

/// Token-aware batch scan with explicit device-opening policy.
pub fn run_batch_scan_with_export_options_and_token_and_policy(
    args: BatchScanArgs,
    export: &ExportOptions,
    token: CancellationToken,
    policy: DeviceOpenPolicy,
) -> Result<Vec<PathBuf>> {
    run_batch_scan_with_export_options_inner(args, export, None, Some(token), policy)
}

/// Batch scan with runtime-only export controls and cooperative cancellation.
///
/// The check runs before acquisition and between emitted pages. Native device
/// calls remain cooperative: a backend that blocks inside one acquisition call
/// cannot observe the flag until it returns to the page callback.
pub fn run_batch_scan_with_export_options_and_cancel(
    args: BatchScanArgs,
    export: &ExportOptions,
    cancel_check: Option<&BatchCancelCheck>,
) -> Result<Vec<PathBuf>> {
    run_batch_scan_with_export_options_inner(
        args,
        export,
        cancel_check,
        None,
        DeviceOpenPolicy::default(),
    )
}

fn run_batch_scan_with_export_options_inner(
    args: BatchScanArgs,
    export: &ExportOptions,
    cancel_check: Option<&BatchCancelCheck>,
    token: Option<CancellationToken>,
    policy: DeviceOpenPolicy,
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
    let export = prepare_export_options_for_pdf(!pdf_destinations.is_empty(), export)?;
    validate_batch_args(&args)?;
    let file_ext = batch_file_extension(&args.format)?;
    validate_batch_destinations(&args, &file_ext)?;
    std::fs::create_dir_all(&args.out_dir)?;
    let pages = scan_batch_pages_with_export(
        &args,
        &file_ext,
        &export,
        cancel_check,
        token.clone(),
        policy,
    )?;
    write_requested_outputs(
        &args,
        &pages.paths,
        &export,
        pages.searchable_text,
        cancel_check,
        token.as_ref(),
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
    crate::core::checked_image_len(args.width, args.height, PixelFormat::Rgb8.bpp())?;
    crate::core::validate_scan_dpi(args.dpi, args.dpi)?;
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

fn validate_batch_destinations(args: &BatchScanArgs, file_ext: &str) -> Result<()> {
    let page_paths = (1..=args.pages)
        .map(|page| {
            let path = args.out_dir.join(format!("page_{page:03}.{file_ext}"));
            validate_output_leaf(&path, &format!("generated page {page}"))?;
            Ok(path)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut destinations: Vec<(&str, PathBuf)> = Vec::new();
    if let Some(path) = &args.multipage_out {
        validate_multipage_output_path(path)?;
        let effective = if path.extension().is_none() {
            path.with_extension("tif")
        } else {
            path.clone()
        };
        validate_output_leaf(&effective, "multipage output")?;
        destinations.push(("multipage output", effective));
    }
    if let Some(path) = &args.multipage_tiff {
        validate_named_container_path(path, "multipage TIFF", &["tif", "tiff"])?;
        destinations.push(("multipage TIFF", path.clone()));
    }
    if let Some(path) = &args.multipage_pdf {
        validate_named_container_path(path, "multipage PDF", &["pdf"])?;
        destinations.push(("multipage PDF", path.clone()));
    }
    if let Some(path) = &args.contact_sheet {
        let effective = if path.extension().is_none() {
            path.with_extension("bmp")
        } else {
            path.clone()
        };
        validate_image_output_path(&effective, "contact sheet")?;
        destinations.push(("contact sheet", effective));
    }

    for index in 0..destinations.len() {
        let (label, path) = &destinations[index];
        for (other_label, other_path) in &destinations[..index] {
            if output_paths_alias(path, other_path)? {
                return Err(ScanError::Invalid(format!(
                    "{label} destination aliases {other_label}: {}",
                    path.display()
                )));
            }
        }
        for (page_index, page_path) in page_paths.iter().enumerate() {
            if output_paths_alias(path, page_path)? {
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

fn validate_multipage_output_path(path: &Path) -> Result<()> {
    if path.extension().is_none() {
        return Ok(());
    }
    validate_named_container_path(path, "multipage output", &["pdf", "tif", "tiff"])
}

fn validate_named_container_path(path: &Path, label: &str, allowed: &[&str]) -> Result<()> {
    validate_output_leaf(path, label)?;
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

fn validate_image_output_path(path: &Path, label: &str) -> Result<()> {
    validate_output_leaf(path, label)?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| ScanError::Invalid(format!("{label} has no supported extension")))?;
    if !crate::imaging::supported_extensions().contains(&extension.as_str()) {
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

fn scan_batch_pages_with_export(
    args: &BatchScanArgs,
    file_ext: &str,
    export: &PreparedExportOptions,
    cancel_check: Option<&BatchCancelCheck>,
    token: Option<CancellationToken>,
    policy: DeviceOpenPolicy,
) -> Result<BatchPages> {
    let device_id = resolve_device_id(Some(&args.device));
    let session = open_device_with_policy(&device_id, policy)?;
    if let Some(token) = token.as_ref() {
        session.bind_cancellation(token.clone());
    }
    let result = scan_batch_pages_with_session_export(
        args,
        file_ext,
        &device_id,
        &session,
        export,
        cancel_check,
        token.as_ref(),
    );
    session.close();
    result
}

fn scan_batch_pages_with_session_export(
    args: &BatchScanArgs,
    file_ext: &str,
    device_id: &str,
    session: &impl DeviceSession,
    export: &PreparedExportOptions,
    cancel_check: Option<&BatchCancelCheck>,
    token: Option<&CancellationToken>,
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
        let processed = apply_export_profile(&apply_pipeline(&image, &pipeline)?, export)?;
        if let Some(text) = searchable_text_with_cancellation(&processed, export, token)? {
            searchable_text_bytes = crate::imaging::checked_pdf_searchable_text_total(
                searchable_text_bytes,
                &text,
                searchable_pages.len(),
            )?;
            searchable_pages.push(text);
        }
        let page_path = args
            .out_dir
            .join(format!("page_{:03}.{}", index + 1, file_ext));
        paths.push(save_image_with_cancellation(
            page_path,
            &processed,
            Some(args.dpi),
            None,
            token,
        )?);
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
    session: &impl DeviceSession,
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

fn write_requested_outputs(
    args: &BatchScanArgs,
    paths: &[PathBuf],
    export: &PreparedExportOptions,
    searchable_pages: Vec<String>,
    cancel_check: Option<&BatchCancelCheck>,
    token: Option<&CancellationToken>,
) -> Result<()> {
    check_output_cancellation(cancel_check, token)?;
    if let Some(ref mp) = args.multipage_out {
        if mp
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
        {
            save_final_pdf_from_paths_with_cancellation(
                mp,
                paths,
                args.dpi,
                export,
                searchable_pages.clone(),
                token,
            )?;
        } else {
            write_multipage(paths, mp, args.dpi, token)?;
        }
    }
    check_output_cancellation(cancel_check, token)?;
    if let Some(ref tiff) = args.multipage_tiff {
        save_multipage_tiff_with_cancellation(paths, tiff, Some(args.dpi), token)?;
    }
    check_output_cancellation(cancel_check, token)?;
    if let Some(ref pdf) = args.multipage_pdf {
        save_final_pdf_from_paths_with_cancellation(
            pdf,
            paths,
            args.dpi,
            export,
            searchable_pages,
            token,
        )?;
    }
    check_output_cancellation(cancel_check, token)?;
    if let Some(ref sheet) = args.contact_sheet {
        save_index_contact_sheet_with_cancellation(paths, sheet, 4, 160, None, 4, token)?;
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

fn write_multipage(
    paths: &[PathBuf],
    out: &Path,
    dpi: u32,
    token: Option<&CancellationToken>,
) -> Result<PathBuf> {
    let ext = out
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("tif")
        .to_ascii_lowercase();
    if ext == "pdf" {
        return save_multipage_pdf_with_cancellation(
            paths,
            out,
            &PdfOptions {
                dpi,
                title: "open-scanline multipage".into(),
                ..PdfOptions::default()
            },
            token,
        );
    }
    if matches!(ext.as_str(), "tif" | "tiff") || out.extension().is_none() {
        let dest = if out.extension().is_none() {
            out.with_extension("tif")
        } else {
            out.to_path_buf()
        };
        return save_multipage_tiff_with_cancellation(paths, dest, Some(dpi), token);
    }
    Err(ScanError::Invalid(format!(
        "multipage output must use .pdf, .tif, or .tiff, not '.{ext}'"
    )))
}
