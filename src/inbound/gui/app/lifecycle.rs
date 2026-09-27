#[cfg(feature = "gui")]
use super::*;

enum JobReceive {
    Event(GuiJobEvent),
    Empty,
    Disconnected,
}

fn receive_job_event(job: &GuiJob) -> JobReceive {
    match job.receiver.try_recv() {
        Ok(event) => JobReceive::Event(event),
        Err(mpsc::TryRecvError::Empty) => JobReceive::Empty,
        Err(mpsc::TryRecvError::Disconnected) => JobReceive::Disconnected,
    }
}

#[cfg(feature = "gui")]
impl OpenScanlineApp {
    fn finish_job(&mut self, terminal: Option<GuiJobEvent>) {
        let Some(mut job) = self.job.take() else {
            return;
        };
        let pending_pdf_password = job.pending_pdf_password.take();
        let join_result = Self::join_job(job);
        self.state.scanning = false;
        self.state.cancel_requested = Arc::new(AtomicBool::new(false));
        if let Err(error) = join_result {
            self.update_job_report(|report| report.fail(error.clone()));
            self.state.restore_export_password(pending_pdf_password);
            self.state.set_error(error);
        } else {
            self.finish_terminal(terminal, pending_pdf_password);
        }
        self.working_images
            .retain(|output| self.state.references_working_source(output.path()));
    }

    pub(in crate::inbound::gui) fn drain_job_events(&mut self, ctx: &egui::Context) {
        let mut terminal = None;
        let mut received_event = false;
        let mut disconnected = false;
        let mut progress_messages = Vec::new();
        loop {
            let Some(job) = &self.job else {
                return;
            };
            match receive_job_event(job) {
                JobReceive::Event(GuiJobEvent::Progress(progress)) => {
                    received_event = true;
                    self.update_job_report(|report| report.apply_progress(&progress.phase));
                    progress_messages.push(progress.message);
                }
                JobReceive::Event(GuiJobEvent::BatchWorkflow(event)) => {
                    received_event = true;
                    self.update_job_report(|report| report.apply_batch_event(event));
                }
                JobReceive::Event(GuiJobEvent::ScanRawPublished(path)) => {
                    received_event = true;
                    self.update_job_report(|report| report.add_raw_output(path));
                }
                JobReceive::Event(event) => {
                    received_event = true;
                    terminal = Some(event);
                    break;
                }
                JobReceive::Empty => break,
                JobReceive::Disconnected => {
                    disconnected = true;
                    break;
                }
            }
        }
        if let Some(message) = progress_messages.last() {
            self.state.status = message.clone();
        }
        let terminal_received = terminal.is_some();
        if terminal_received || disconnected {
            self.finish_job(terminal);
        }
        if self.job_active() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if received_event || terminal_received {
            ctx.request_repaint();
        }
    }

    pub(in crate::inbound::gui) fn request_close(&mut self, ctx: &egui::Context) {
        self.stop_discovery();
        if !self.job_active() && self.discovery_finished() {
            self.closing = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        self.closing = true;
        self.cancel_job();
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }

    pub(in crate::inbound::gui) fn finish_deferred_close(&mut self, ctx: &egui::Context) {
        if self.closing && !self.job_active() && self.discovery_finished() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if self.closing {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    pub(in crate::inbound::gui) fn cancel_and_join_job(&mut self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::SeqCst);
        }
        if self.job.is_some() {
            self.finish_job(Some(GuiJobEvent::Cancelled));
        }
    }

    fn finish_terminal(&mut self, terminal: Option<GuiJobEvent>, password: Option<String>) {
        match terminal {
            Some(GuiJobEvent::ScanFinished {
                path,
                working_image,
                advance_frame,
            }) => self.finish_scan(path, working_image, advance_frame),
            Some(GuiJobEvent::BatchFinished { paths, end }) => self.finish_batch(paths, end),
            Some(GuiJobEvent::SaveFinished {
                path,
                retained_source,
                candidate,
                advance_frame,
            }) => self.finish_save(path, retained_source, candidate, advance_frame),
            Some(GuiJobEvent::OcrFinished { engine, text }) => {
                self.state.status =
                    format!("{} ({}): {}", self.state.translator.t("ocr"), engine, text)
            }
            Some(GuiJobEvent::Cancelled) => {
                self.update_job_report(report::JobReport::cancel);
                self.state.restore_export_password(password);
                self.state.status = self.state.translator.t("status.cancelled");
            }
            Some(GuiJobEvent::Failed(error)) => {
                self.update_job_report(|report| report.fail(error.clone()));
                self.state.restore_export_password(password);
                self.state.set_error(error);
            }
            Some(GuiJobEvent::Progress(_)) => unreachable!("progress is not terminal"),
            Some(GuiJobEvent::BatchWorkflow(_)) => unreachable!("batch workflow is not terminal"),
            Some(GuiJobEvent::ScanRawPublished(_)) => {
                unreachable!("raw publication is not terminal")
            }
            None => {
                self.update_job_report(|report| {
                    report.fail("background scan worker stopped without a result".into())
                });
                self.state.restore_export_password(password);
                self.state
                    .set_error("background scan worker stopped without a result");
            }
        }
    }

    fn finish_scan(
        &mut self,
        path: PathBuf,
        working_image: Option<TemporaryOutput>,
        advance_frame: bool,
    ) {
        self.update_job_report(|report| report.finish_scan(path.clone()));
        if let Some(working_image) = working_image {
            let raster_path = working_image.path().to_path_buf();
            self.working_images.push(working_image);
            self.state.last_image = Some(raster_path);
            self.state.status = format!(
                "{} {}",
                self.state.translator.t("status.done"),
                path.display()
            );
            self.state.update_histogram();
        } else {
            self.state.set_image_success(path);
        }
        if advance_frame {
            self.state.frame_index += 1;
        }
    }

    fn finish_batch(
        &mut self,
        paths: Vec<PathBuf>,
        end: crate::infrastructure::acquisition::ScanPagesEnd,
    ) {
        self.update_job_report(|report| report.finish_batch(&paths, end));
        if let Some(last) = paths.last() {
            self.state.last_image = Some(last.clone());
            self.state.update_histogram();
        }
        self.state.status = format!("{} {}", self.state.translator.t("batch.done"), paths.len());
    }

    fn finish_save(
        &mut self,
        path: PathBuf,
        retained_source: Option<PathBuf>,
        candidate: Option<super::super::state::MultipageSaveCandidate>,
        advance_frame: bool,
    ) {
        if let Some(candidate) = candidate {
            self.state.commit_multipage_save(candidate);
        } else if let Some(source) = retained_source {
            self.state.last_image = Some(source);
            self.state.update_histogram();
        } else {
            self.state.last_image = Some(path.clone());
        }
        if advance_frame {
            self.state.frame_index += 1;
            self.state.status = format!(
                "{} {} ({})",
                self.state.translator.t("status.done"),
                path.display(),
                self.state.frame_index
            );
        } else {
            self.state.status = format!(
                "{} {}",
                self.state.translator.t("status.done"),
                path.display()
            );
        }
    }
}
