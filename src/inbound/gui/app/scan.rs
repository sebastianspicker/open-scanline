#[cfg(feature = "gui")]
use super::*;

#[cfg(feature = "gui")]
impl OpenScanlineApp {
    pub(in crate::inbound::gui) fn scan_job_plan(
        &mut self,
        preview: bool,
        cancel: Arc<AtomicBool>,
    ) -> Option<ScanJobPlan> {
        let mut args = self
            .state
            .scan_args(preview)
            .map_err(|error| self.state.set_error(error))
            .ok()?;
        let cancel_check = Arc::clone(&cancel);
        args.cancel_check = Some(Box::new(move || cancel_check.load(Ordering::SeqCst)));
        let mut export = self.scan_export(preview);
        let pending_pdf_password = (!preview).then(|| export.pdf_password.clone()).flatten();
        let published_path = args.out.clone();
        let export_dpi = args.dpi;
        let (working_image, prepared_export) = prepare_working_output(
            &published_path,
            &export,
            !preview && destination_needs_working_image(&published_path),
            "gui-working",
        )
        .map_err(|error| self.scan_plan_error(pending_pdf_password.clone(), error))
        .ok()?;
        if let Some(output) = working_image.as_ref() {
            args.out = output.path().to_path_buf();
            export = crate::workflows::publication::ExportOptions::default();
        }
        Some(ScanJobPlan {
            args,
            export,
            token: CancellationToken::from_arc(cancel),
            pending_pdf_password,
            published_path,
            export_dpi,
            working_image,
            prepared_export,
        })
    }

    fn scan_export(&mut self, preview: bool) -> crate::workflows::publication::ExportOptions {
        if preview {
            let mut export = self.state.export_options();
            export.pdf_password = None;
            export.searchable_pdf = false;
            export
        } else {
            self.state.take_export_options()
        }
    }

    pub(in crate::inbound::gui) fn scan_plan_error(
        &mut self,
        password: Option<String>,
        error: impl ToString,
    ) {
        self.state.restore_export_password(password);
        self.state.set_error(error.to_string());
    }
}
