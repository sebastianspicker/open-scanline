#[cfg(feature = "gui")]
use super::*;

#[cfg(feature = "gui")]
impl OpenScanlineApp {
    pub(in crate::inbound::gui) fn start_save(&mut self) {
        if self.job_active() || self.closing {
            return;
        }
        let Some(action) = self.state.prepare_save() else {
            return;
        };
        self.start_save_action(action);
    }

    pub(in crate::inbound::gui) fn start_save_plus(&mut self) {
        if self.job_active() || self.closing {
            return;
        }
        let Some(action) = self.state.prepare_save_plus() else {
            return;
        };
        self.start_save_action(action);
    }

    fn start_save_action(&mut self, action: SaveAction) {
        if self.job_active() || self.closing {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let token = CancellationToken::from_arc(Arc::clone(&cancel));
        let pending_pdf_password = action.pending_pdf_password.clone();
        let retained_source = (action.candidate.is_none()
            && destination_needs_working_image(&action.dest))
        .then(|| action.src.clone());
        self.start_file_job(cancel, pending_pdf_password, "status.saving", move || {
            let result = execute_save_action(&action, &token);
            save_job_event(result, action, retained_source)
        });
    }
}

#[cfg(feature = "gui")]
fn execute_save_action(
    action: &SaveAction,
    token: &CancellationToken,
) -> crate::error::Result<PathBuf> {
    match action.candidate.as_ref() {
        Some(candidate) => save_final_multipage_from_paths_with_cancellation(
            &action.dest,
            &candidate.sources,
            action.dpi,
            &action.export,
            Some(token),
        ),
        None => save_single_action(action, token),
    }
}

#[cfg(feature = "gui")]
fn save_single_action(
    action: &SaveAction,
    token: &CancellationToken,
) -> crate::error::Result<PathBuf> {
    let prepared = prepare_export_options(&action.dest, &action.export)?;
    let image = apply_export_profile(&load_image(&action.src)?, &prepared)?;
    save_final_image_with_cancellation(
        &action.dest,
        &image,
        Some(action.dpi),
        None,
        &prepared,
        Some(token),
    )
}

#[cfg(feature = "gui")]
fn save_job_event(
    result: crate::error::Result<PathBuf>,
    action: SaveAction,
    retained_source: Option<PathBuf>,
) -> GuiJobEvent {
    match result {
        Ok(path) => GuiJobEvent::SaveFinished {
            path,
            retained_source,
            candidate: action.candidate,
            advance_frame: action.advance_frame,
        },
        Err(ScanError::Cancelled(_)) => GuiJobEvent::Cancelled,
        Err(error) => GuiJobEvent::Failed(error.to_string()),
    }
}
