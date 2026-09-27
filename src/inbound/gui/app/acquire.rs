#[cfg(feature = "gui")]
use super::*;

#[cfg(feature = "gui")]
fn forward_scan_progress(
    sender: &Sender<GuiJobEvent>,
    raw_output: Option<&PathBuf>,
    progress: ScanProgress,
) {
    if let ("raw-saved", Some(path)) = (progress.phase.as_str(), raw_output) {
        let _ = sender.send(GuiJobEvent::ScanRawPublished(path.clone()));
    }
    let _ = sender.send(GuiJobEvent::Progress(progress));
}

#[cfg(feature = "gui")]
impl OpenScanlineApp {
    pub(in crate::inbound::gui) fn start_scan(&mut self, preview: bool, advance_frame: bool) {
        if self.job_active() || self.closing {
            return;
        }
        if let Err(error) = self.state.validate_color_controls() {
            self.state
                .set_error(format!("invalid color controls: {error}"));
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let Some(plan) = self.scan_job_plan(preview, Arc::clone(&cancel)) else {
            return;
        };
        if !preview {
            self.replace_job_report(report::JobReport::scan(
                self.state.output_name.clone(),
                &plan.args,
                plan.published_path.clone(),
            ));
        } else {
            self.job_report = None;
        }
        self.state.cancel_requested = Arc::clone(&cancel);
        self.state.scanning = true;
        self.state.error_message = None;
        self.state.status = self.state.translator.t("status.scanning");
        self.launch_scan_job(plan, cancel, advance_frame);
    }

    fn launch_scan_job(&mut self, plan: ScanJobPlan, cancel: Arc<AtomicBool>, advance_frame: bool) {
        let ScanJobPlan {
            mut args,
            export,
            token,
            pending_pdf_password,
            published_path,
            export_dpi,
            mut working_image,
            prepared_export,
        } = plan;
        self.job = Some(Self::spawn_job(
            cancel,
            pending_pdf_password,
            move |sender| {
                let progress_sender = sender.clone();
                let raw_output = args.raw_out.clone();
                args.on_progress = Some(Box::new(move |progress| {
                    forward_scan_progress(&progress_sender, raw_output.as_ref(), progress);
                }));
                let result =
                    run_scan_to_file_with_export_options_and_token(args, &export, token.clone())
                        .and_then(|raster| {
                            Self::publish_scan_result(
                                raster,
                                &published_path,
                                export_dpi,
                                prepared_export.as_ref(),
                                &token,
                            )
                        });
                match result {
                    Ok(path) => GuiJobEvent::ScanFinished {
                        path,
                        working_image: working_image.take(),
                        advance_frame,
                    },
                    Err(ScanError::Cancelled(_)) => GuiJobEvent::Cancelled,
                    Err(error) => GuiJobEvent::Failed(error.to_string()),
                }
            },
        ));
    }

    fn publish_scan_result(
        raster: PathBuf,
        published_path: &Path,
        export_dpi: u32,
        prepared_export: Option<&crate::workflows::publication::PreparedExportOptions>,
        token: &CancellationToken,
    ) -> crate::error::Result<PathBuf> {
        let Some(prepared) = prepared_export else {
            return Ok(raster);
        };
        let image = apply_export_profile(&load_image(&raster)?, prepared)?;
        save_final_image_with_cancellation(
            published_path,
            &image,
            Some(export_dpi),
            None,
            prepared,
            Some(token),
        )
    }
}
