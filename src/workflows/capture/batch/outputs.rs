use super::{
    is_batch_cancelled, BatchCancelCheck, BatchEventObserver, BatchPublishedOutput,
    BatchPublishedOutputKind, BatchRunHooks, BatchScanArgs, BatchWorkflowEvent,
};
use crate::error::{Result, ScanError};
use crate::infrastructure::media::{NativeAggregateSession, PdfPathPublication};
use crate::operation::CancellationToken;
use crate::workflows::publication::{
    save_final_pdf_from_paths_with_cancellation, PreparedExportOptions,
};
use std::path::{Path, PathBuf};

#[cfg(test)]
pub(crate) fn write_requested_outputs(
    args: &BatchScanArgs,
    paths: &[PathBuf],
    export: &PreparedExportOptions,
    searchable_pages: Vec<String>,
    cancel_check: Option<&BatchCancelCheck>,
    token: Option<&CancellationToken>,
) -> Result<()> {
    let hooks = BatchRunHooks {
        cancel_check,
        token,
        observer: None,
    };
    write_requested_outputs_with_observer(args, paths, export, searchable_pages, hooks)
}

pub(super) fn write_requested_outputs_with_observer(
    args: &BatchScanArgs,
    paths: &[PathBuf],
    export: &PreparedExportOptions,
    searchable_pages: Vec<String>,
    hooks: BatchRunHooks<'_>,
) -> Result<()> {
    let mut session = begin_session(args, paths);
    write_multipage_output(args, paths, export, &searchable_pages, &mut session, hooks)?;
    write_tiff_output(args, paths, &mut session, hooks)?;
    write_pdf_output(args, paths, export, &searchable_pages, &mut session, hooks)?;
    write_contact_sheet(args, paths, &mut session, hooks)
}

fn report_published(
    observer: Option<&BatchEventObserver>,
    kind: BatchPublishedOutputKind,
    path: PathBuf,
) {
    if let Some(observer) = observer {
        observer(BatchWorkflowEvent::Published(BatchPublishedOutput {
            kind,
            path,
        }));
    }
}

fn begin_session(args: &BatchScanArgs, _paths: &[PathBuf]) -> Option<NativeAggregateSession> {
    (aggregate_output_count(args) > 1).then(NativeAggregateSession::new)
}

fn aggregate_output_count(args: &BatchScanArgs) -> usize {
    usize::from(args.multipage_out.is_some())
        + usize::from(args.multipage_tiff.is_some())
        + usize::from(args.multipage_pdf.is_some())
        + usize::from(args.contact_sheet.is_some())
}

fn write_multipage_output(
    args: &BatchScanArgs,
    paths: &[PathBuf],
    export: &PreparedExportOptions,
    searchable_pages: &[String],
    session: &mut Option<NativeAggregateSession>,
    hooks: BatchRunHooks<'_>,
) -> Result<()> {
    check_output_cancellation(hooks.cancel_check, hooks.token)?;
    if let Some(ref output) = args.multipage_out {
        if is_pdf(output) {
            let path = write_pdf(
                output,
                paths,
                args.dpi,
                export,
                searchable_pages,
                hooks.token,
                session,
            )?;
            report_published(hooks.observer, BatchPublishedOutputKind::Document, path);
        } else {
            let path = write_multipage(paths, output, args.dpi, hooks.token, session)?;
            report_published(hooks.observer, BatchPublishedOutputKind::Document, path);
        }
    }
    Ok(())
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

fn write_tiff_output(
    args: &BatchScanArgs,
    paths: &[PathBuf],
    session: &mut Option<NativeAggregateSession>,
    hooks: BatchRunHooks<'_>,
) -> Result<()> {
    check_output_cancellation(hooks.cancel_check, hooks.token)?;
    if let Some(ref tiff) = args.multipage_tiff {
        let path = match session.as_mut() {
            Some(session) => {
                session.publish_tiff(paths, tiff, Some(args.dpi), None, hooks.token)?
            }
            None => crate::infrastructure::media::publish_tiff_from_paths(
                paths,
                tiff,
                Some(args.dpi),
                None,
                hooks.token,
            )?,
        };
        report_published(hooks.observer, BatchPublishedOutputKind::Document, path);
    }
    Ok(())
}

fn write_pdf_output(
    args: &BatchScanArgs,
    paths: &[PathBuf],
    export: &PreparedExportOptions,
    searchable_pages: &[String],
    session: &mut Option<NativeAggregateSession>,
    hooks: BatchRunHooks<'_>,
) -> Result<()> {
    check_output_cancellation(hooks.cancel_check, hooks.token)?;
    if let Some(ref pdf) = args.multipage_pdf {
        let path = write_pdf(
            pdf,
            paths,
            args.dpi,
            export,
            searchable_pages,
            hooks.token,
            session,
        )?;
        report_published(hooks.observer, BatchPublishedOutputKind::Document, path);
    }
    Ok(())
}

fn write_contact_sheet(
    args: &BatchScanArgs,
    paths: &[PathBuf],
    session: &mut Option<NativeAggregateSession>,
    hooks: BatchRunHooks<'_>,
) -> Result<()> {
    check_output_cancellation(hooks.cancel_check, hooks.token)?;
    if let Some(ref sheet) = args.contact_sheet {
        let path = match session.as_mut() {
            Some(session) => session.publish_contact_sheet(paths, sheet, hooks.token)?,
            None => crate::infrastructure::media::save_index_contact_sheet_with_cancellation(
                paths,
                sheet,
                4,
                160,
                None,
                4,
                hooks.token,
            )?,
        };
        report_published(hooks.observer, BatchPublishedOutputKind::ContactSheet, path);
    }
    Ok(())
}

pub(super) fn check_output_cancellation(
    cancel_check: Option<&BatchCancelCheck>,
    token: Option<&CancellationToken>,
) -> Result<()> {
    if is_batch_cancelled(cancel_check, token) {
        return Err(ScanError::Cancelled("batch scan cancelled".into()));
    }
    Ok(())
}

pub(super) fn report_batch_completion(args: &BatchScanArgs, page_count: usize) {
    if let Some(ref callback) = args.on_progress {
        callback(crate::domain::acquisition::ScanProgress::new(
            "done",
            1.0,
            format!("{page_count} pages"),
        ));
    }
}

fn write_multipage(
    paths: &[PathBuf],
    output: &Path,
    dpi: u32,
    token: Option<&CancellationToken>,
    session: &mut Option<NativeAggregateSession>,
) -> Result<PathBuf> {
    let extension = output_extension(output);
    if extension == "pdf" {
        return write_plain_pdf(paths, output, dpi, token, session);
    }
    if matches!(extension.as_str(), "tif" | "tiff") || output.extension().is_none() {
        let destination = output_with_tiff_extension(output);
        return match session.as_mut() {
            Some(session) => session.publish_tiff(paths, &destination, Some(dpi), None, token),
            None => crate::infrastructure::media::publish_tiff_from_paths(
                paths,
                &destination,
                Some(dpi),
                None,
                token,
            ),
        };
    }
    Err(ScanError::Invalid(format!(
        "multipage output must use .pdf, .tif, or .tiff, not '.{extension}'"
    )))
}

fn output_extension(output: &Path) -> String {
    output
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("tif")
        .to_ascii_lowercase()
}

fn output_with_tiff_extension(output: &Path) -> PathBuf {
    if output.extension().is_none() {
        output.with_extension("tif")
    } else {
        output.to_path_buf()
    }
}

fn write_plain_pdf(
    paths: &[PathBuf],
    output: &Path,
    dpi: u32,
    token: Option<&CancellationToken>,
    session: &mut Option<NativeAggregateSession>,
) -> Result<PathBuf> {
    let request = PdfPathPublication {
        paths,
        destination: output,
        dpi,
        title: "open-scanline multipage",
        password: None,
        searchable_pages: None,
        transform: None,
        cancellation: token,
    };
    match session.as_mut() {
        Some(session) => session.publish_pdf(request),
        None => crate::infrastructure::media::publish_pdf_from_paths(request),
    }
}

#[allow(clippy::too_many_arguments)]
fn write_pdf(
    output: &Path,
    paths: &[PathBuf],
    dpi: u32,
    export: &PreparedExportOptions,
    searchable_pages: &[String],
    token: Option<&CancellationToken>,
    session: &mut Option<NativeAggregateSession>,
) -> Result<PathBuf> {
    let request = PdfPathPublication {
        paths,
        destination: output,
        dpi,
        title: "open-scanline multipage",
        password: export.pdf_password(),
        searchable_pages: export
            .needs_searchable_text()
            .then(|| searchable_pages.to_vec()),
        transform: None,
        cancellation: token,
    };
    match session.as_mut() {
        Some(session) => session.publish_pdf(request),
        None => save_final_pdf_from_paths_with_cancellation(
            output,
            paths,
            dpi,
            export,
            searchable_pages.to_vec(),
            token,
        ),
    }
}
