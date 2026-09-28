#[cfg(feature = "gui")]
use super::*;

#[cfg(feature = "gui")]
fn run_batch_job(
    mut args: crate::workflows::capture::batch::BatchScanArgs,
    export: crate::domain::export::ExportOptions,
    token: CancellationToken,
    sender: Sender<GuiJobEvent>,
) -> GuiJobEvent {
    let progress_sender = sender.clone();
    args.on_progress = Some(Box::new(move |progress| {
        let _ = progress_sender.send(GuiJobEvent::Progress(progress));
    }));
    let observer = move |event| {
        let _ = sender.send(GuiJobEvent::BatchWorkflow(event));
    };
    match crate::workflows::capture::batch::run_batch_scan_with_report(
        args,
        BatchCaptureOptions {
            export,
            token: Some(token),
            observer: Some(&observer),
            ..BatchCaptureOptions::default()
        },
    ) {
        Ok(report) => GuiJobEvent::BatchFinished {
            paths: report.page_paths,
            end: report.end,
        },
        Err(ScanError::Cancelled(_)) => GuiJobEvent::Cancelled,
        Err(error) => GuiJobEvent::Failed(error.to_string()),
    }
}

#[cfg(feature = "gui")]
impl OpenScanlineApp {
    pub(in crate::inbound::gui) fn start_batch(&mut self) {
        if self.job_active() || self.closing {
            return;
        }
        if let Err(error) = self.state.validate_color_controls() {
            self.state
                .set_error(format!("invalid color controls: {error}"));
            return;
        }
        let pages = self.state.batch_pages.max(1);
        let out_dir = PathBuf::from(&self.state.output_dir).join("batch");
        let args = match self.state.batch_args(out_dir, pages) {
            Ok(args) => args,
            Err(error) => {
                self.state.set_error(error);
                return;
            }
        };
        self.replace_job_report(report::JobReport::batch(
            self.state.output_name.clone(),
            &args,
        ));
        let cancel = Arc::new(AtomicBool::new(false));
        let token = CancellationToken::from_arc(Arc::clone(&cancel));
        let export = self.state.take_export_options();
        let pending_pdf_password = export.pdf_password.clone();
        self.state.cancel_requested = Arc::clone(&cancel);
        self.state.scanning = true;
        self.state.error_message = None;
        self.state.status = self.state.translator.t("status.scanning");
        self.job = Some(Self::spawn_job(
            cancel,
            pending_pdf_password,
            move |sender| run_batch_job(args, export, token, sender),
        ));
    }
}
