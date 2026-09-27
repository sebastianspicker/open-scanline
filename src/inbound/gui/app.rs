#[cfg(feature = "gui")]
use super::actions::SaveAction;
#[cfg(feature = "gui")]
use super::state::GuiState;
#[cfg(feature = "gui")]
use crate::infrastructure::media::image_buffer_to_rgba;
#[cfg(feature = "gui")]
use crate::infrastructure::media::load_image;
#[cfg(feature = "gui")]
use crate::infrastructure::runtime::TemporaryOutput;
#[cfg(feature = "gui")]
use crate::{
    domain::acquisition::ScanProgress, error::ScanError,
    inbound::api::batch::run_batch_scan_with_export_options_and_token_and_report,
    inbound::api::scan::run_scan_to_file_with_export_options_and_token,
    workflows::capture::batch::BatchWorkflowEvent, workflows::operation::CancellationToken,
};
#[cfg(feature = "gui")]
use crate::{
    inbound::api::process::process_image_file_with_export_options_and_token,
    inbound::api::publication::{
        apply_export_profile, prepare_export_options, save_final_image_with_cancellation,
        save_final_multipage_from_paths_with_cancellation,
    },
    infrastructure::media::ocr::ocr_image_with_engine_with_cancellation,
};
#[cfg(feature = "gui")]
use std::any::Any;
#[cfg(feature = "gui")]
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
pub(in crate::inbound::gui::app) enum GuiJobEvent {
    Progress(ScanProgress),
    ScanFinished {
        path: PathBuf,
        working_image: Option<TemporaryOutput>,
        advance_frame: bool,
    },
    ScanRawPublished(PathBuf),
    BatchFinished {
        paths: Vec<PathBuf>,
        end: crate::workflows::ports::acquisition::ScanPagesEnd,
    },
    BatchWorkflow(BatchWorkflowEvent),
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
pub(in crate::inbound::gui::app) struct GuiJob {
    receiver: Receiver<GuiJobEvent>,
    handle: Option<JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    pending_pdf_password: Option<String>,
}

#[cfg(feature = "gui")]
pub(in crate::inbound::gui) struct ScanJobPlan {
    args: crate::inbound::api::scan::ScanToFileArgs,
    export: crate::workflows::publication::ExportOptions,
    token: CancellationToken,
    pending_pdf_password: Option<String>,
    published_path: PathBuf,
    export_dpi: u32,
    working_image: Option<TemporaryOutput>,
    prepared_export: Option<crate::workflows::publication::PreparedExportOptions>,
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
type PreparedWorkingOutput = (
    Option<TemporaryOutput>,
    Option<crate::workflows::publication::PreparedExportOptions>,
);

#[cfg(feature = "gui")]
fn prepare_working_output(
    published_path: &Path,
    export: &crate::workflows::publication::ExportOptions,
    needed: bool,
    label: &str,
) -> crate::error::Result<PreparedWorkingOutput> {
    let working_image = needed
        .then(|| TemporaryOutput::new(label, "png"))
        .transpose()?;
    let prepared_export = working_image
        .as_ref()
        .map(|_| prepare_export_options(published_path, export))
        .transpose()?;
    Ok((working_image, prepared_export))
}

#[cfg(feature = "gui")]
pub(super) struct OpenScanlineApp {
    pub(super) state: GuiState,
    pub(super) preview_tex: Option<egui::TextureHandle>,
    pub(super) preview_tex_path: Option<PathBuf>,
    pub(super) preview_tex_fp: Option<(std::time::SystemTime, u64)>,
    preview_revision: u64,
    job: Option<GuiJob>,
    working_images: Vec<TemporaryOutput>,
    closing: bool,
    discovery: discovery::DiscoveryWorker,
    job_report: Option<report::JobReport>,
}

#[cfg(feature = "gui")]
mod acquire;
#[cfg(feature = "gui")]
mod batch;
#[cfg(feature = "gui")]
mod jobs;
#[cfg(feature = "gui")]
mod ocr;
#[cfg(feature = "gui")]
pub(in crate::inbound::gui) mod report;
#[cfg(feature = "gui")]
mod reprocess;
mod runner;
#[cfg(feature = "gui")]
mod save;
#[cfg(feature = "gui")]
mod scan;

#[cfg(feature = "gui")]
impl OpenScanlineApp {
    pub(super) fn new(config_path: Option<&Path>) -> Self {
        Self {
            state: GuiState::new(config_path),
            preview_tex: None,
            preview_tex_path: None,
            preview_tex_fp: None,
            preview_revision: u64::MAX,
            job: None,
            working_images: Vec::new(),
            closing: false,
            discovery: discovery::DiscoveryWorker::new(Arc::new(discovery::NativeDiscovery)),
            job_report: None,
        }
    }

    pub(super) fn job_active(&self) -> bool {
        self.job.is_some()
    }
}

#[cfg(feature = "gui")]
mod discovery;
#[cfg(feature = "gui")]
mod lifecycle;
#[cfg(feature = "gui")]
mod preview;

#[cfg(feature = "gui")]
impl Drop for OpenScanlineApp {
    fn drop(&mut self) {
        self.cancel_and_join_job();
    }
}

#[cfg(all(test, feature = "gui"))]
mod tests;

/// Launch desktop GUI. Uses eframe when the `gui` feature is enabled.
pub(super) use runner::run;
