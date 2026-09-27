#[cfg(feature = "gui")]
use super::*;

#[cfg(feature = "gui")]
impl OpenScanlineApp {
    pub(in crate::inbound::gui) fn cancel_job(&mut self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::SeqCst);
            self.state.do_cancel();
        }
    }

    pub(in crate::inbound::gui::app) fn start_file_job<F>(
        &mut self,
        cancel: Arc<AtomicBool>,
        pending_pdf_password: Option<String>,
        status_key: &str,
        worker: F,
    ) where
        F: FnOnce() -> GuiJobEvent + Send + 'static,
    {
        self.job_report = None;
        self.state.cancel_requested = Arc::clone(&cancel);
        self.state.scanning = true;
        self.state.error_message = None;
        self.state.status = self.state.translator.t(status_key);
        self.job = Some(Self::spawn_job(cancel, pending_pdf_password, move |_| {
            worker()
        }));
    }

    pub(in crate::inbound::gui::app) fn spawn_job<F>(
        cancel: Arc<AtomicBool>,
        pending_pdf_password: Option<String>,
        worker: F,
    ) -> GuiJob
    where
        F: FnOnce(Sender<GuiJobEvent>) -> GuiJobEvent + Send + 'static,
    {
        let (sender, receiver) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let terminal =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| worker(sender.clone())))
                    .unwrap_or_else(|payload| {
                        GuiJobEvent::Failed(format!(
                            "background scan worker panicked: {}",
                            panic_message(payload)
                        ))
                    });
            // Keep this worker's terminal event as its final GUI-visible action.
            let _ = sender.send(terminal);
        });
        GuiJob {
            receiver,
            handle: Some(handle),
            cancel,
            pending_pdf_password,
        }
    }

    pub(in crate::inbound::gui::app) fn join_job(mut job: GuiJob) -> Result<(), String> {
        match job.handle.take() {
            Some(handle) => handle.join().map_err(|payload| {
                format!(
                    "background scan worker panicked: {}",
                    panic_message(payload)
                )
            }),
            None => Ok(()),
        }
    }
}
