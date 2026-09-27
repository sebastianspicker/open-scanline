#[cfg(feature = "gui")]
use super::*;
#[cfg(feature = "gui")]
use crate::inbound::gui::actions::ReprocessAction;

#[cfg(feature = "gui")]
impl OpenScanlineApp {
    pub(in crate::inbound::gui) fn start_reprocess(&mut self) {
        if self.job_active() || self.closing {
            return;
        }
        let Some(job) = self.prepare_reprocess_job() else {
            return;
        };
        self.start_file_job(
            job.cancel,
            job.pending_pdf_password,
            "status.processing",
            move || {
                let result = execute_reprocess_action(
                    &job.action,
                    &job.published_path,
                    job.prepared_export.as_ref(),
                    &job.token,
                );
                reprocess_job_event(result, job.working_image)
            },
        );
    }

    fn prepare_reprocess_job(&mut self) -> Option<PreparedReprocessJob> {
        let mut action = self.state.prepare_reprocess()?;
        let cancel = Arc::new(AtomicBool::new(false));
        let token = CancellationToken::from_arc(Arc::clone(&cancel));
        let pending_pdf_password = action.pending_pdf_password.clone();
        let published_path = action.options.dst.clone();
        let (working_image, prepared_export) = prepare_working_output(
            &published_path,
            &action.export,
            destination_needs_working_image(&published_path),
            "gui-reprocess",
        )
        .map_err(|error| self.scan_plan_error(pending_pdf_password.clone(), error))
        .ok()?;
        if let Some(output) = working_image.as_ref() {
            action.options.dst = output.path().to_path_buf();
        }
        Some(PreparedReprocessJob {
            action,
            cancel,
            pending_pdf_password,
            token,
            published_path,
            working_image,
            prepared_export,
        })
    }
}

#[cfg(feature = "gui")]
struct PreparedReprocessJob {
    action: ReprocessAction,
    cancel: Arc<AtomicBool>,
    pending_pdf_password: Option<String>,
    token: CancellationToken,
    published_path: PathBuf,
    working_image: Option<TemporaryOutput>,
    prepared_export: Option<crate::workflows::publication::PreparedExportOptions>,
}

#[cfg(feature = "gui")]
fn execute_reprocess_action(
    action: &ReprocessAction,
    published_path: &Path,
    prepared_export: Option<&crate::workflows::publication::PreparedExportOptions>,
    token: &CancellationToken,
) -> crate::error::Result<PathBuf> {
    let process_export = prepared_export
        .map(|_| crate::domain::export::ExportOptions::default())
        .unwrap_or_else(|| action.export.clone());
    let raster = process::process_image_file(
        &action.options,
        ProcessRunOptions {
            export: process_export,
            cancellation: Some(token.clone()),
        },
    )?;
    let Some(prepared) = prepared_export else {
        return Ok(raster);
    };
    let image = apply_export_profile(&load_image(&raster)?, prepared)?;
    save_final_image_with_cancellation(
        published_path,
        &image,
        Some(action.dpi),
        None,
        prepared,
        Some(token),
    )
}

#[cfg(feature = "gui")]
fn reprocess_job_event(
    result: crate::error::Result<PathBuf>,
    mut working_image: Option<TemporaryOutput>,
) -> GuiJobEvent {
    match result {
        Ok(path) => GuiJobEvent::ScanFinished {
            path,
            working_image: working_image.take(),
            advance_frame: false,
        },
        Err(ScanError::Cancelled(_)) => GuiJobEvent::Cancelled,
        Err(error) => GuiJobEvent::Failed(error.to_string()),
    }
}
