#[cfg(feature = "gui")]
use super::actions::SaveAction;
#[cfg(feature = "gui")]
use super::state::GuiState;
#[cfg(feature = "gui")]
use super::{preview_file_fingerprint, preview_texture_needs_reload};
#[cfg(feature = "gui")]
use crate::infrastructure::media::image_buffer_to_rgba;
#[cfg(feature = "gui")]
use crate::infrastructure::media::load_image;
#[cfg(feature = "gui")]
use crate::infrastructure::runtime::TemporaryOutput;
#[cfg(feature = "gui")]
use crate::{
    batch::run_batch_scan_with_export_options_and_token,
    core::{ScanError, ScanProgress},
    device::CancellationToken,
    scan::run_scan_to_file_with_export_options_and_token,
};
#[cfg(feature = "gui")]
use crate::{
    inbound::api::publication::{
        apply_export_profile, prepare_export_options, save_final_image_with_cancellation,
        save_final_multipage_from_paths_with_cancellation,
    },
    ocr::ocr_image_with_cancellation,
    process::process_image_file_with_export_options_and_token,
};
#[cfg(feature = "gui")]
use std::any::Any;
use std::path::Path;
#[cfg(feature = "gui")]
use std::path::PathBuf;
#[cfg(feature = "gui")]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender},
    Arc,
};
#[cfg(feature = "gui")]
use std::thread::JoinHandle;

#[cfg(feature = "gui")]
enum GuiJobEvent {
    Progress(ScanProgress),
    ScanFinished {
        path: PathBuf,
        working_image: Option<TemporaryOutput>,
        advance_frame: bool,
    },
    BatchFinished(Vec<PathBuf>),
    SaveFinished {
        path: PathBuf,
        retained_source: Option<PathBuf>,
        candidate: Option<super::state::MultipageSaveCandidate>,
        advance_frame: bool,
    },
    OcrFinished {
        engine: String,
        text: String,
    },
    Cancelled,
    Failed(String),
}

#[cfg(feature = "gui")]
struct GuiJob {
    receiver: Receiver<GuiJobEvent>,
    handle: Option<JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    pending_pdf_password: Option<String>,
}

#[cfg(feature = "gui")]
fn panic_message(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).into()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".into()
    }
}

#[cfg(feature = "gui")]
fn destination_needs_working_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "pdf" | "jxl"))
}

#[cfg(feature = "gui")]
pub(super) struct OpenScanlineApp {
    pub(super) state: GuiState,
    pub(super) preview_tex: Option<egui::TextureHandle>,
    pub(super) preview_tex_path: Option<PathBuf>,
    pub(super) preview_tex_fp: Option<(u64, u64)>,
    job: Option<GuiJob>,
    working_images: Vec<TemporaryOutput>,
    closing: bool,
}

#[cfg(feature = "gui")]
impl OpenScanlineApp {
    pub(super) fn new(config_path: Option<&Path>) -> Self {
        Self {
            state: GuiState::new(config_path),
            preview_tex: None,
            preview_tex_path: None,
            preview_tex_fp: None,
            job: None,
            working_images: Vec::new(),
            closing: false,
        }
    }

    pub(super) fn job_active(&self) -> bool {
        self.job.is_some()
    }

    pub(super) fn start_scan(&mut self, preview: bool, advance_frame: bool) {
        if self.job_active() || self.closing {
            return;
        }
        if let Err(error) = self.state.validate_color_controls() {
            self.state
                .set_error(format!("invalid color controls: {error}"));
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let mut args = match self.state.scan_args(preview) {
            Ok(args) => args,
            Err(error) => {
                self.state.set_error(error);
                return;
            }
        };
        let cancel_check = Arc::clone(&cancel);
        args.cancel_check = Some(Box::new(move || cancel_check.load(Ordering::SeqCst)));
        let token = CancellationToken::from_arc(Arc::clone(&cancel));
        let mut export = if preview {
            let mut export = self.state.export_options();
            export.pdf_password = None;
            export.searchable_pdf = false;
            export
        } else {
            self.state.take_export_options()
        };
        let pending_pdf_password = (!preview).then(|| export.pdf_password.clone()).flatten();
        let published_path = args.out.clone();
        let export_dpi = args.dpi;
        let mut working_image = if !preview && destination_needs_working_image(&published_path) {
            match TemporaryOutput::new("gui-working", "png") {
                Ok(output) => Some(output),
                Err(error) => {
                    self.state
                        .restore_export_password(pending_pdf_password.clone());
                    self.state.set_error(error);
                    return;
                }
            }
        } else {
            None
        };
        let prepared_export = if working_image.is_some() {
            match prepare_export_options(&published_path, &export) {
                Ok(prepared) => Some(prepared),
                Err(error) => {
                    self.state
                        .restore_export_password(pending_pdf_password.clone());
                    self.state.set_error(error);
                    return;
                }
            }
        } else {
            None
        };
        if let Some(output) = working_image.as_ref() {
            args.out = output.path().to_path_buf();
            // Pipeline into a lossless raster first. The requested profile and
            // PDF/JXL export are applied exactly once below.
            export = crate::ExportOptions::default();
        }
        self.state.cancel_requested = Arc::clone(&cancel);
        self.state.scanning = true;
        self.state.status = self.state.translator.t("status.scanning");
        self.job = Some(Self::spawn_job(
            cancel,
            pending_pdf_password,
            move |sender| {
                let progress_sender = sender.clone();
                args.on_progress = Some(Box::new(move |progress| {
                    let _ = progress_sender.send(GuiJobEvent::Progress(progress));
                }));
                let result: crate::error::Result<PathBuf> = (|| {
                    let raster = run_scan_to_file_with_export_options_and_token(
                        args,
                        &export,
                        token.clone(),
                    )?;
                    let Some(prepared) = prepared_export.as_ref() else {
                        return Ok(raster);
                    };
                    let image = apply_export_profile(&load_image(&raster)?, prepared)?;
                    save_final_image_with_cancellation(
                        &published_path,
                        &image,
                        Some(export_dpi),
                        None,
                        prepared,
                        Some(&token),
                    )
                })();
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

    pub(super) fn start_batch(&mut self) {
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
        let cancel = Arc::new(AtomicBool::new(false));
        let token = CancellationToken::from_arc(Arc::clone(&cancel));
        let export = self.state.take_export_options();
        let pending_pdf_password = export.pdf_password.clone();
        self.state.cancel_requested = Arc::clone(&cancel);
        self.state.scanning = true;
        self.state.status = self.state.translator.t("status.scanning");
        self.job = Some(Self::spawn_job(
            cancel,
            pending_pdf_password,
            move |sender| {
                let progress_sender = sender.clone();
                let mut args = args;
                args.on_progress = Some(Box::new(move |progress| {
                    let _ = progress_sender.send(GuiJobEvent::Progress(progress));
                }));
                match run_batch_scan_with_export_options_and_token(args, &export, token) {
                    Ok(paths) => GuiJobEvent::BatchFinished(paths),
                    Err(ScanError::Cancelled(_)) => GuiJobEvent::Cancelled,
                    Err(error) => GuiJobEvent::Failed(error.to_string()),
                }
            },
        ));
    }

    pub(super) fn cancel_job(&mut self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::SeqCst);
            self.state.do_cancel();
        }
    }

    pub(super) fn start_save(&mut self) {
        if self.job_active() || self.closing {
            return;
        }
        let Some(action) = self.state.prepare_save() else {
            return;
        };
        self.start_save_action(action);
    }

    pub(super) fn start_save_plus(&mut self) {
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
            let result: crate::error::Result<PathBuf> = (|| match action.candidate.as_ref() {
                Some(candidate) => save_final_multipage_from_paths_with_cancellation(
                    &action.dest,
                    &candidate.sources,
                    action.dpi,
                    &action.export,
                    Some(&token),
                ),
                None => {
                    let prepared = prepare_export_options(&action.dest, &action.export)?;
                    let image = apply_export_profile(&load_image(&action.src)?, &prepared)?;
                    save_final_image_with_cancellation(
                        &action.dest,
                        &image,
                        Some(action.dpi),
                        None,
                        &prepared,
                        Some(&token),
                    )
                }
            })();
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
        });
    }

    pub(super) fn start_reprocess(&mut self) {
        if self.job_active() || self.closing {
            return;
        }
        let Some(mut action) = self.state.prepare_reprocess() else {
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let token = CancellationToken::from_arc(Arc::clone(&cancel));
        let pending_pdf_password = action.pending_pdf_password.clone();
        let published_path = action.options.dst.clone();
        let mut working_image = if destination_needs_working_image(&published_path) {
            match TemporaryOutput::new("gui-reprocess", "png") {
                Ok(output) => Some(output),
                Err(error) => {
                    self.state
                        .restore_export_password(pending_pdf_password.clone());
                    self.state.set_error(error);
                    return;
                }
            }
        } else {
            None
        };
        let prepared_export = if working_image.is_some() {
            match prepare_export_options(&published_path, &action.export) {
                Ok(prepared) => Some(prepared),
                Err(error) => {
                    self.state
                        .restore_export_password(pending_pdf_password.clone());
                    self.state.set_error(error);
                    return;
                }
            }
        } else {
            None
        };
        if let Some(output) = working_image.as_ref() {
            action.options.dst = output.path().to_path_buf();
        }
        self.start_file_job(
            cancel,
            pending_pdf_password,
            "status.processing",
            move || match (|| {
                let process_export = if prepared_export.is_some() {
                    crate::ExportOptions::default()
                } else {
                    action.export.clone()
                };
                let raster = process_image_file_with_export_options_and_token(
                    &action.options,
                    &process_export,
                    token.clone(),
                )?;
                let Some(prepared) = prepared_export.as_ref() else {
                    return Ok(raster);
                };
                let image = apply_export_profile(&load_image(&raster)?, prepared)?;
                save_final_image_with_cancellation(
                    &published_path,
                    &image,
                    Some(action.dpi),
                    None,
                    prepared,
                    Some(&token),
                )
            })() {
                Ok(path) => GuiJobEvent::ScanFinished {
                    path,
                    working_image: working_image.take(),
                    advance_frame: false,
                },
                Err(ScanError::Cancelled(_)) => GuiJobEvent::Cancelled,
                Err(error) => GuiJobEvent::Failed(error.to_string()),
            },
        );
    }

    pub(super) fn start_ocr(&mut self) {
        if self.job_active() || self.closing {
            return;
        }
        let Some(action) = self.state.prepare_ocr() else {
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let token = CancellationToken::from_arc(Arc::clone(&cancel));
        self.start_file_job(cancel, None, "status.processing", move || {
            let result = load_image(&action.src).and_then(|image| {
                ocr_image_with_cancellation(&image, &action.language, action.offline, Some(&token))
            });
            match result {
                Ok(result) => GuiJobEvent::OcrFinished {
                    engine: result.engine,
                    text: result.text,
                },
                Err(ScanError::Cancelled(_)) => GuiJobEvent::Cancelled,
                Err(error) => GuiJobEvent::Failed(error.to_string()),
            }
        });
    }

    fn start_file_job<F>(
        &mut self,
        cancel: Arc<AtomicBool>,
        pending_pdf_password: Option<String>,
        status_key: &str,
        worker: F,
    ) where
        F: FnOnce() -> GuiJobEvent + Send + 'static,
    {
        self.state.cancel_requested = Arc::clone(&cancel);
        self.state.scanning = true;
        self.state.status = self.state.translator.t(status_key);
        self.job = Some(Self::spawn_job(cancel, pending_pdf_password, move |_| {
            worker()
        }));
    }

    fn spawn_job<F>(
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

    fn join_job(mut job: GuiJob) -> Result<(), String> {
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

    fn finish_job(&mut self, terminal: Option<GuiJobEvent>) {
        let Some(mut job) = self.job.take() else {
            return;
        };
        let pending_pdf_password = job.pending_pdf_password.take();
        let join_result = Self::join_job(job);
        self.state.scanning = false;
        self.state.cancel_requested = Arc::new(AtomicBool::new(false));
        match join_result {
            Err(error) => {
                self.state.restore_export_password(pending_pdf_password);
                self.state.set_error(error);
            }
            Ok(()) => match terminal {
                Some(GuiJobEvent::ScanFinished {
                    path,
                    working_image,
                    advance_frame,
                }) => {
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
                Some(GuiJobEvent::BatchFinished(paths)) => {
                    if let Some(last) = paths.last() {
                        self.state.last_image = Some(last.clone());
                        self.state.update_histogram();
                    }
                    self.state.status =
                        format!("{} {}", self.state.translator.t("batch.done"), paths.len());
                }
                Some(GuiJobEvent::SaveFinished {
                    path,
                    retained_source,
                    candidate,
                    advance_frame,
                }) => {
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
                Some(GuiJobEvent::OcrFinished { engine, text }) => {
                    self.state.status =
                        format!("{} ({}): {}", self.state.translator.t("ocr"), engine, text);
                }
                Some(GuiJobEvent::Cancelled) => {
                    self.state.restore_export_password(pending_pdf_password);
                    self.state.status = self.state.translator.t("status.cancelled");
                }
                Some(GuiJobEvent::Failed(error)) => {
                    self.state.restore_export_password(pending_pdf_password);
                    self.state.set_error(error);
                }
                Some(GuiJobEvent::Progress(_)) => unreachable!("progress is not terminal"),
                None => {
                    self.state.restore_export_password(pending_pdf_password);
                    self.state
                        .set_error("background scan worker stopped without a result");
                }
            },
        }
        self.working_images
            .retain(|output| self.state.references_working_source(output.path()));
    }

    pub(super) fn drain_job_events(&mut self, ctx: &egui::Context) {
        let mut terminal = None;
        let mut received_event = false;
        let mut disconnected = false;
        let mut progress_messages = Vec::new();
        if let Some(job) = &self.job {
            loop {
                let event = match job.receiver.try_recv() {
                    Ok(event) => event,
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                };
                received_event = true;
                match event {
                    GuiJobEvent::Progress(progress) => progress_messages.push(progress.message),
                    event => {
                        terminal = Some(event);
                        break;
                    }
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

    pub(super) fn request_close(&mut self, ctx: &egui::Context) {
        if !self.job_active() {
            self.closing = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        self.closing = true;
        self.cancel_job();
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }

    pub(super) fn finish_deferred_close(&mut self, ctx: &egui::Context) {
        if self.closing && !self.job_active() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    pub(super) fn cancel_and_join_job(&mut self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::SeqCst);
        }
        if self.job.is_some() {
            self.finish_job(Some(GuiJobEvent::Cancelled));
        }
    }

    /// Rebuild when path or file content changes; Preview reuses its path.
    pub(super) fn ensure_preview_texture(&mut self, ctx: &egui::Context) {
        let path = match &self.state.last_image {
            Some(path) => path.clone(),
            None => {
                self.preview_tex = None;
                self.preview_tex_path = None;
                self.preview_tex_fp = None;
                return;
            }
        };
        if !preview_texture_needs_reload(
            Some(path.as_path()),
            self.preview_tex_path.as_deref(),
            self.preview_tex_fp,
        ) && self.preview_tex.is_some()
        {
            return;
        }
        let Ok(image) = load_image(&path) else {
            self.preview_tex = None;
            self.preview_tex_path = None;
            self.preview_tex_fp = None;
            return;
        };
        let Ok((width, height, rgba)) = image_buffer_to_rgba(&image) else {
            return;
        };
        let fingerprint = preview_file_fingerprint(&path);
        let color =
            egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
        let name = format!(
            "preview-{}-{}-{}",
            path.display(),
            fingerprint.map(|value| value.0).unwrap_or(0),
            fingerprint.map(|value| value.1).unwrap_or(0)
        );
        self.preview_tex = Some(ctx.load_texture(name, color, egui::TextureOptions::LINEAR));
        self.preview_tex_path = Some(path);
        self.preview_tex_fp = fingerprint;
    }
}

#[cfg(feature = "gui")]
impl Drop for OpenScanlineApp {
    fn drop(&mut self) {
        self.cancel_and_join_job();
    }
}

#[cfg(all(test, feature = "gui"))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn install_test_job<F>(app: &mut OpenScanlineApp, worker: F) -> Arc<AtomicBool>
    where
        F: FnOnce(Arc<AtomicBool>, Sender<GuiJobEvent>) + Send + 'static,
    {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (sender, receiver) = mpsc::channel();
        let handle = std::thread::spawn(move || worker(worker_cancel, sender));
        app.state.cancel_requested = Arc::clone(&cancel);
        app.state.scanning = true;
        app.job = Some(GuiJob {
            receiver,
            handle: Some(handle),
            cancel: Arc::clone(&cancel),
            pending_pdf_password: None,
        });
        cancel
    }

    fn drain_until_idle(app: &mut OpenScanlineApp, context: &egui::Context) {
        for _ in 0..500 {
            app.drain_job_events(context);
            if !app.job_active() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("test job did not finish");
    }

    #[test]
    fn scan_plus_updates_state_only_after_the_worker_completes() {
        let directory = std::env::temp_dir().join(format!(
            "open_scanline_gui_async_scan_plus_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let context = egui::Context::default();
        let mut app = OpenScanlineApp::new(None);
        app.state.output_dir = directory.display().to_string();
        app.state.output_name = "async".into();

        app.start_scan(false, true);
        assert!(app.job_active());
        assert_eq!(app.state.frame_index, 0);
        for _ in 0..100 {
            app.drain_job_events(&context);
            if !app.job_active() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert!(!app.job_active());
        assert_eq!(app.state.frame_index, 1);
        assert!(app
            .state
            .last_image
            .as_ref()
            .is_some_and(|path| path.is_file()));
        assert!(app
            .state
            .status
            .contains(&app.state.translator.t("status.done")));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn pdf_scan_keeps_a_raster_working_image_for_follow_up_actions() {
        let directory = std::env::temp_dir().join(format!(
            "open_scanline_gui_pdf_working_image_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let context = egui::Context::default();
        let mut app = OpenScanlineApp::new(None);
        app.state.output_dir = directory.display().to_string();
        app.state.output_name = "document".into();
        app.state.output_fmt = "pdf".into();

        app.start_scan(false, false);
        drain_until_idle(&mut app, &context);

        let published = directory.join("document_scan_000.pdf");
        let working = app.state.last_image.clone().expect("working image path");
        assert!(published.is_file());
        assert_eq!(
            std::fs::read(&published).unwrap().get(..4),
            Some(b"%PDF".as_slice())
        );
        assert_eq!(
            working.extension().and_then(|value| value.to_str()),
            Some("png")
        );
        assert!(crate::infrastructure::media::load_image(&working).is_ok());

        app.state.ocr_engine = "offline".into();
        app.start_ocr();
        drain_until_idle(&mut app, &context);
        assert!(app
            .state
            .status
            .contains(crate::infrastructure::media::ocr::OFFLINE_OCR_ENGINE));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn multipage_pdf_session_retains_every_temporary_scan_source() {
        let directory = std::env::temp_dir().join(format!(
            "open_scanline_gui_pdf_multipage_sources_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let context = egui::Context::default();
        let mut app = OpenScanlineApp::new(None);
        app.state.output_dir = directory.display().to_string();
        app.state.output_name = "document".into();
        app.state.output_fmt = "pdf".into();
        app.state.multipage = true;
        app.state.multipage_format = "pdf".into();

        app.start_scan(false, false);
        drain_until_idle(&mut app, &context);
        let first = app.state.last_image.clone().expect("first raster source");
        app.start_save_plus();
        drain_until_idle(&mut app, &context);

        app.start_scan(false, false);
        drain_until_idle(&mut app, &context);
        let second = app.state.last_image.clone().expect("second raster source");
        assert_ne!(first, second);
        assert!(first.is_file(), "first raster must remain session-owned");
        assert!(second.is_file());

        app.start_save_plus();
        drain_until_idle(&mut app, &context);
        let document = lopdf::Document::load(directory.join("document_multipage.pdf")).unwrap();
        assert_eq!(document.get_pages().len(), 2);

        drop(app);
        assert!(!first.exists());
        assert!(!second.exists());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn exported_container_save_retains_the_processable_source() {
        let directory = std::env::temp_dir().join(format!(
            "open_scanline_gui_container_source_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.png");
        write_test_image(&source);
        let context = egui::Context::default();
        let mut app = OpenScanlineApp::new(None);
        app.state.output_dir = directory.display().to_string();
        app.state.output_name = "saved".into();
        app.state.output_fmt = "pdf".into();
        app.state.last_image = Some(source.clone());

        app.start_save_plus();
        drain_until_idle(&mut app, &context);

        assert_eq!(app.state.last_image.as_ref(), Some(&source));
        assert!(directory.join("saved_save_000.pdf").is_file());
        let _ = std::fs::remove_dir_all(directory);
    }

    fn write_test_image(path: &Path) {
        crate::infrastructure::media::save_image(
            path,
            &crate::domain::image::ImageBuffer::new(
                1,
                1,
                crate::domain::image::PixelFormat::Rgb8,
                vec![20, 30, 40],
            )
            .unwrap(),
            None,
            None,
        )
        .unwrap();
    }

    #[test]
    fn save_plus_starts_immediately_and_applies_the_worker_result() {
        let directory = std::env::temp_dir().join(format!(
            "open_scanline_gui_async_save_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.png");
        write_test_image(&source);
        let context = egui::Context::default();
        let mut app = OpenScanlineApp::new(None);
        app.state.output_dir = directory.display().to_string();
        app.state.output_name = "saved".into();
        app.state.last_image = Some(source.clone());

        app.start_save_plus();
        assert!(app.job_active());
        assert_eq!(app.state.last_image.as_ref(), Some(&source));
        assert_eq!(app.state.frame_index, 0);
        drain_until_idle(&mut app, &context);

        assert_eq!(app.state.frame_index, 1);
        assert!(app
            .state
            .last_image
            .as_ref()
            .is_some_and(|path| path.is_file()));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_file_export_restores_consumed_password() {
        let directory = std::env::temp_dir().join(format!(
            "open_scanline_gui_async_save_password_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.png");
        write_test_image(&source);
        let context = egui::Context::default();
        let mut app = OpenScanlineApp::new(None);
        app.state.output_dir = directory.display().to_string();
        app.state.output_name = "failed".into();
        app.state.output_fmt = "pdf".into();
        app.state.pdf_password = "save-secret".into();
        app.state.scanner_profile_path =
            directory.join("missing-profile.json").display().to_string();
        app.state.last_image = Some(source);

        app.start_save_plus();
        assert!(app.job_active());
        assert!(app.state.pdf_password.is_empty());
        drain_until_idle(&mut app, &context);

        assert_eq!(app.state.pdf_password, "save-secret");
        assert!(app.state.status.contains("Error"));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn offline_ocr_starts_in_a_job_and_applies_its_result() {
        let directory = std::env::temp_dir().join(format!(
            "open_scanline_gui_async_ocr_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.png");
        write_test_image(&source);
        let context = egui::Context::default();
        let mut app = OpenScanlineApp::new(None);
        app.state.last_image = Some(source);
        app.state.ocr_engine = "offline".into();

        app.start_ocr();
        assert!(app.job_active());
        drain_until_idle(&mut app, &context);

        assert!(app
            .state
            .status
            .contains(crate::infrastructure::media::ocr::OFFLINE_OCR_ENGINE));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn second_file_action_during_an_active_job_does_not_consume_its_password() {
        let directory = std::env::temp_dir().join(format!(
            "open_scanline_gui_async_action_guard_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.png");
        write_test_image(&source);
        let context = egui::Context::default();
        let mut app = OpenScanlineApp::new(None);
        app.state.output_dir = directory.display().to_string();
        app.state.output_name = "guard".into();
        app.state.output_fmt = "pdf".into();
        app.state.pdf_password = "unconsumed-secret".into();
        app.state.last_image = Some(source);
        install_test_job(&mut app, move |cancel, sender| {
            while !cancel.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            sender.send(GuiJobEvent::Cancelled).unwrap();
        });

        app.start_save_plus();

        assert!(app.job_active());
        assert_eq!(app.state.pdf_password, "unconsumed-secret");
        app.cancel_job();
        drain_until_idle(&mut app, &context);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn cancelled_job_resets_cancellation_for_the_next_reprocess() {
        let directory = std::env::temp_dir().join(format!(
            "open_scanline_gui_async_cancel_reset_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.png");
        write_test_image(&source);
        let context = egui::Context::default();
        let mut app = OpenScanlineApp::new(None);
        app.state.output_dir = directory.display().to_string();
        app.state.output_name = "reprocessed".into();
        app.state.last_image = Some(source);
        install_test_job(&mut app, move |cancel, sender| {
            while !cancel.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            sender.send(GuiJobEvent::Cancelled).unwrap();
        });

        app.cancel_job();
        drain_until_idle(&mut app, &context);
        assert!(!app.state.cancel_requested.load(Ordering::SeqCst));

        app.start_reprocess();
        assert!(app.job_active());
        drain_until_idle(&mut app, &context);
        assert!(app
            .state
            .last_image
            .as_ref()
            .is_some_and(|path| path.is_file()));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn terminal_drain_joins_before_clearing_the_job() {
        let context = egui::Context::default();
        let mut app = OpenScanlineApp::new(None);
        let exited = Arc::new(AtomicBool::new(false));
        let worker_exited = Arc::clone(&exited);
        install_test_job(&mut app, move |_cancel, sender| {
            sender.send(GuiJobEvent::Cancelled).unwrap();
            std::thread::sleep(Duration::from_millis(30));
            worker_exited.store(true, Ordering::SeqCst);
        });

        let started = Instant::now();
        drain_until_idle(&mut app, &context);

        assert!(started.elapsed() >= Duration::from_millis(25));
        assert!(exited.load(Ordering::SeqCst));
        assert!(!app.state.scanning);
        assert!(!app.job_active());
    }

    #[test]
    fn invalid_job_args_do_not_activate_or_replace_cancellation_state() {
        let mut app = OpenScanlineApp::new(None);
        app.state.output_name = "../outside".into();
        let original_cancel = Arc::clone(&app.state.cancel_requested);

        app.start_scan(false, false);
        assert!(!app.job_active());
        assert!(!app.state.scanning);
        assert!(Arc::ptr_eq(&app.state.cancel_requested, &original_cancel));

        app.start_batch();
        assert!(!app.job_active());
        assert!(!app.state.scanning);
        assert!(Arc::ptr_eq(&app.state.cancel_requested, &original_cancel));
    }

    #[test]
    fn failed_encrypted_scan_restores_password() {
        let directory = std::env::temp_dir().join(format!(
            "open_scanline_gui_failed_encrypted_scan_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let context = egui::Context::default();
        let mut app = OpenScanlineApp::new(None);
        app.state.output_dir = directory.display().to_string();
        app.state.output_name = "encrypted_scan".into();
        app.state.output_fmt = "pdf".into();
        app.state.pdf_password = "scan-secret".into();
        app.state.scanner_profile_path =
            directory.join("missing-profile.json").display().to_string();

        app.start_scan(false, false);
        drain_until_idle(&mut app, &context);

        assert_eq!(app.state.pdf_password, "scan-secret");
        assert!(app.state.status.contains("Error"));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_encrypted_batch_restores_password() {
        let directory = std::env::temp_dir().join(format!(
            "open_scanline_gui_failed_encrypted_batch_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let context = egui::Context::default();
        let mut app = OpenScanlineApp::new(None);
        app.state.output_dir = directory.display().to_string();
        app.state.output_name = "encrypted_batch".into();
        app.state.output_fmt = "pdf".into();
        app.state.pdf_password = "batch-secret".into();
        app.state.scanner_profile_path =
            directory.join("missing-profile.json").display().to_string();

        app.start_batch();
        drain_until_idle(&mut app, &context);

        assert_eq!(app.state.pdf_password, "batch-secret");
        assert!(app.state.status.contains("Error"));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn cancelled_encrypted_scan_restores_password_without_overwriting_replacement() {
        let context = egui::Context::default();
        let mut app = OpenScanlineApp::new(None);
        app.state.pdf_password = "scan-secret".into();
        let export = app.state.take_export_options();
        let pending_password = export.pdf_password.clone();
        let cancel = install_test_job(&mut app, move |cancel, sender| {
            while !cancel.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            sender.send(GuiJobEvent::Cancelled).unwrap();
        });
        app.job.as_mut().unwrap().pending_pdf_password = pending_password;
        app.state.pdf_password = "replacement-secret".into();

        app.cancel_job();
        assert!(cancel.load(Ordering::SeqCst));
        drain_until_idle(&mut app, &context);

        assert_eq!(app.state.pdf_password, "replacement-secret");
    }

    #[test]
    fn cancelled_encrypted_batch_restores_password() {
        let context = egui::Context::default();
        let mut app = OpenScanlineApp::new(None);
        app.state.pdf_password = "batch-secret".into();
        let export = app.state.take_export_options();
        let pending_password = export.pdf_password.clone();
        let cancel = install_test_job(&mut app, move |cancel, sender| {
            while !cancel.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            sender.send(GuiJobEvent::Cancelled).unwrap();
        });
        app.job.as_mut().unwrap().pending_pdf_password = pending_password;

        app.cancel_job();
        assert!(cancel.load(Ordering::SeqCst));
        drain_until_idle(&mut app, &context);

        assert_eq!(app.state.pdf_password, "batch-secret");
    }

    #[test]
    fn successful_encrypted_scan_consumes_password() {
        let directory = std::env::temp_dir().join(format!(
            "open_scanline_gui_successful_encrypted_scan_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        let context = egui::Context::default();
        let mut app = OpenScanlineApp::new(None);
        app.state.output_dir = directory.display().to_string();
        app.state.output_name = "encrypted_scan".into();
        app.state.output_fmt = "pdf".into();
        app.state.pdf_password = "success-secret".into();

        app.start_scan(false, false);
        drain_until_idle(&mut app, &context);

        assert!(app.state.pdf_password.is_empty());
        assert!(directory.join("encrypted_scan_scan_000.pdf").is_file());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn close_request_defers_close_until_cancelled_job_is_joined() {
        let context = egui::Context::default();
        let mut app = OpenScanlineApp::new(None);
        let cancel = install_test_job(&mut app, move |cancel, sender| {
            while !cancel.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            sender.send(GuiJobEvent::Cancelled).unwrap();
        });

        context.begin_pass(egui::RawInput::default());
        app.request_close(&context);
        let output = context.end_pass();
        assert!(output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::CancelClose));
        assert!(app.closing);
        assert!(app.job_active());
        assert!(cancel.load(Ordering::SeqCst));

        drain_until_idle(&mut app, &context);
        context.begin_pass(egui::RawInput::default());
        app.finish_deferred_close(&context);
        let output = context.end_pass();
        assert!(output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::Close));
    }

    #[test]
    fn worker_panic_becomes_a_visible_failure() {
        let context = egui::Context::default();
        let mut app = OpenScanlineApp::new(None);
        let cancel = Arc::new(AtomicBool::new(false));
        app.state.scanning = true;
        app.job = Some(OpenScanlineApp::spawn_job(cancel, None, |_| {
            panic!("test worker panic")
        }));

        drain_until_idle(&mut app, &context);

        assert!(app.state.status.contains("background scan worker panicked"));
        assert!(!app.state.scanning);
    }

    #[test]
    fn drop_signals_and_joins_before_late_worker_output() {
        let marker = std::env::temp_dir().join(format!(
            "open_scanline_gui_drop_marker_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&marker);
        let worker_marker = marker.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = Arc::clone(&cancelled);
        let mut app = OpenScanlineApp::new(None);
        install_test_job(&mut app, move |cancel, sender| {
            while !cancel.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            worker_cancelled.store(true, Ordering::SeqCst);
            sender.send(GuiJobEvent::Cancelled).unwrap();
            std::fs::write(worker_marker, "worker finished").unwrap();
        });

        drop(app);

        assert!(cancelled.load(Ordering::SeqCst));
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "worker finished");
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "worker finished");
        let _ = std::fs::remove_file(marker);
    }
}

/// Launch desktop GUI. Uses eframe when the `gui` feature is enabled.
pub(super) fn run(config_path: Option<&Path>) -> i32 {
    #[cfg(feature = "gui")]
    {
        let path_owned = config_path.map(Path::to_path_buf);
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1100.0, 720.0])
                .with_title(format!("{} {}", crate::APP_NAME, crate::VERSION)),
            ..Default::default()
        };
        match eframe::run_native(
            crate::APP_NAME,
            options,
            Box::new(move |_cc| {
                Ok(Box::new(OpenScanlineApp::new(path_owned.as_deref())) as Box<dyn eframe::App>)
            }),
        ) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("GUI launch failed: {error}");
                eprintln!("GUI toolkit could not open a window in this environment");
                1
            }
        }
    }
    #[cfg(not(feature = "gui"))]
    {
        let _ = config_path;
        eprintln!("GUI unavailable: rebuild with the 'gui' feature and use a desktop session");
        1
    }
}
