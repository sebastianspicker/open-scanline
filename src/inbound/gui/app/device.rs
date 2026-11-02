use super::*;

impl OpenScanlineApp {
    /// Start the maintenance action the UI queued, unless a job is running.
    pub(in crate::inbound::gui) fn start_pending_device_job(&mut self) {
        if self.job_active() || self.closing {
            return;
        }
        let Some(action) = self.state.device_job.take() else {
            return;
        };
        self.start_file_job(
            Arc::new(AtomicBool::new(false)),
            None,
            "status.processing",
            move || GuiJobEvent::DeviceFinished(action.run()),
        );
        if let Some(job) = &mut self.job {
            job.device = true;
        }
    }
}
