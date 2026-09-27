use super::OpenScanlineApp;
use crate::domain::acquisition::ScanMode;
use crate::inbound::api::batch::BatchScanArgs;
use crate::inbound::api::scan::ScanToFileArgs;
use crate::infrastructure::acquisition::ScanPagesEnd;
use crate::workflows::capture::batch::{
    BatchPublishedOutput, BatchPublishedOutputKind, BatchWorkflowEvent,
};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::inbound::gui) enum JobPhase {
    Preparing,
    Acquiring,
    Processing,
    Publishing,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::inbound::gui) enum BatchStopReason {
    LimitReached,
    FeederExhausted,
}

impl From<ScanPagesEnd> for BatchStopReason {
    fn from(value: ScanPagesEnd) -> Self {
        match value {
            ScanPagesEnd::LimitReached => Self::LimitReached,
            ScanPagesEnd::FeederExhausted => Self::FeederExhausted,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::inbound::gui) enum JobTerminal {
    Completed(Option<BatchStopReason>),
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::inbound::gui) enum OutputKind {
    Image,
    RawImage,
    Page,
    Document,
    ContactSheet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::inbound::gui) struct RequestedOutput {
    pub(in crate::inbound::gui) kind: OutputKind,
    pub(in crate::inbound::gui) path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::inbound::gui) struct PublishedFile {
    pub(in crate::inbound::gui) kind: OutputKind,
    pub(in crate::inbound::gui) path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::inbound::gui) struct JobReport {
    pub(in crate::inbound::gui) title: String,
    pub(in crate::inbound::gui) device: String,
    pub(in crate::inbound::gui) source: String,
    pub(in crate::inbound::gui) mode: ScanMode,
    pub(in crate::inbound::gui) width: u32,
    pub(in crate::inbound::gui) height: u32,
    pub(in crate::inbound::gui) dpi: u32,
    pub(in crate::inbound::gui) duplex: bool,
    pub(in crate::inbound::gui) requested_outputs: Vec<RequestedOutput>,
    pub(in crate::inbound::gui) published_files: Vec<PublishedFile>,
    pub(in crate::inbound::gui) sides_complete: u32,
    pub(in crate::inbound::gui) side_limit: u32,
    pub(in crate::inbound::gui) phase: JobPhase,
    pub(in crate::inbound::gui) terminal: Option<JobTerminal>,
    pub(in crate::inbound::gui) error: Option<String>,
}

impl JobReport {
    pub(super) fn scan(title: String, args: &ScanToFileArgs, published_path: PathBuf) -> Self {
        let mut requested_outputs = Vec::with_capacity(2);
        if let Some(raw_path) = &args.raw_out {
            requested_outputs.push(RequestedOutput {
                kind: OutputKind::RawImage,
                path: raw_path.clone(),
            });
        }
        requested_outputs.push(RequestedOutput {
            kind: output_kind(&published_path),
            path: published_path,
        });
        Self::new(
            JobSnapshot {
                title,
                device: args.device.clone().unwrap_or_else(|| "default".into()),
                mode: args.mode,
                width: args.width,
                height: args.height,
                dpi: args.dpi,
                duplex: false,
            },
            requested_outputs,
            1,
        )
    }

    pub(super) fn batch(title: String, args: &BatchScanArgs) -> Self {
        let mut requested_outputs = Vec::with_capacity(args.pages as usize + 4);
        let extension = args.format.trim_start_matches('.');
        for index in 1..=args.pages {
            requested_outputs.push(RequestedOutput {
                kind: OutputKind::Page,
                path: args.out_dir.join(format!("page_{index:03}.{extension}")),
            });
        }
        for path in [
            args.multipage_out.as_ref(),
            args.multipage_tiff.as_ref(),
            args.multipage_pdf.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            requested_outputs.push(RequestedOutput {
                kind: OutputKind::Document,
                path: effective_document_path(path),
            });
        }
        if let Some(path) = &args.contact_sheet {
            requested_outputs.push(RequestedOutput {
                kind: OutputKind::ContactSheet,
                path: path.clone(),
            });
        }
        Self::new(
            JobSnapshot {
                title,
                device: args.device.clone(),
                mode: args.mode,
                width: args.width,
                height: args.height,
                dpi: args.dpi,
                duplex: args.duplex,
            },
            requested_outputs,
            args.pages,
        )
    }

    fn new(
        snapshot: JobSnapshot,
        requested_outputs: Vec<RequestedOutput>,
        side_limit: u32,
    ) -> Self {
        let JobSnapshot {
            title,
            device,
            mode,
            width,
            height,
            dpi,
            duplex,
        } = snapshot;
        Self {
            title,
            device,
            source: source_label(mode),
            mode,
            width,
            height,
            dpi,
            duplex,
            requested_outputs,
            published_files: Vec::new(),
            sides_complete: 0,
            side_limit,
            phase: JobPhase::Preparing,
            terminal: None,
            error: None,
        }
    }

    pub(super) fn apply_progress(&mut self, phase: &str) {
        self.phase = match phase {
            "open" | "acquire" | "batch" => JobPhase::Acquiring,
            "pipeline" => JobPhase::Processing,
            "save" | "done" => JobPhase::Publishing,
            _ => self.phase,
        };
    }

    pub(super) fn apply_batch_event(&mut self, event: BatchWorkflowEvent) {
        match event {
            BatchWorkflowEvent::Published(output) => self.add_batch_output(output),
            BatchWorkflowEvent::PublishingOutputs => self.phase = JobPhase::Publishing,
        }
    }

    pub(super) fn finish_scan(&mut self, path: PathBuf) {
        self.add_published(PublishedFile {
            kind: output_kind(&path),
            path,
        });
        self.sides_complete = 1;
        self.finish(JobTerminal::Completed(None), None);
    }

    pub(super) fn add_raw_output(&mut self, path: PathBuf) {
        self.add_published(PublishedFile {
            kind: OutputKind::RawImage,
            path,
        });
    }

    pub(super) fn finish_batch(&mut self, paths: &[PathBuf], end: ScanPagesEnd) {
        for path in paths {
            self.add_published(PublishedFile {
                kind: OutputKind::Page,
                path: path.clone(),
            });
        }
        self.sides_complete = paths.len().try_into().unwrap_or(u32::MAX);
        self.finish(JobTerminal::Completed(Some(end.into())), None);
    }

    pub(super) fn cancel(&mut self) {
        self.finish(JobTerminal::Cancelled, None);
    }

    pub(super) fn fail(&mut self, error: String) {
        self.finish(JobTerminal::Failed, Some(error));
    }

    fn add_batch_output(&mut self, output: BatchPublishedOutput) {
        let kind = match output.kind {
            BatchPublishedOutputKind::Page => OutputKind::Page,
            BatchPublishedOutputKind::Document => OutputKind::Document,
            BatchPublishedOutputKind::ContactSheet => OutputKind::ContactSheet,
        };
        if kind == OutputKind::Page {
            self.sides_complete = self.sides_complete.saturating_add(1);
        }
        self.add_published(PublishedFile {
            kind,
            path: output.path,
        });
    }

    fn add_published(&mut self, file: PublishedFile) {
        if !self
            .published_files
            .iter()
            .any(|published| published.path == file.path)
        {
            self.published_files.push(file);
        }
    }

    fn finish(&mut self, terminal: JobTerminal, error: Option<String>) {
        self.phase = JobPhase::Finished;
        self.terminal = Some(terminal);
        self.error = error;
    }
}

struct JobSnapshot {
    title: String,
    device: String,
    mode: ScanMode,
    width: u32,
    height: u32,
    dpi: u32,
    duplex: bool,
}

impl OpenScanlineApp {
    pub(in crate::inbound::gui) fn job_report(&self) -> Option<&JobReport> {
        self.job_report.as_ref()
    }

    pub(in crate::inbound::gui) fn clear_job_report(&mut self) {
        if !self.job_active() {
            self.job_report = None;
        }
    }

    pub(super) fn update_job_report(&mut self, update: impl FnOnce(&mut JobReport)) {
        if let Some(report) = &mut self.job_report {
            update(report);
        }
    }

    pub(super) fn replace_job_report(&mut self, report: JobReport) {
        self.job_report = Some(report);
    }
}

fn effective_document_path(path: &Path) -> PathBuf {
    if path.extension().is_none() {
        path.with_extension("tif")
    } else {
        path.to_path_buf()
    }
}

fn output_kind(path: &Path) -> OutputKind {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("pdf" | "tif" | "tiff") => OutputKind::Document,
        _ => OutputKind::Image,
    }
}

fn source_label(mode: ScanMode) -> String {
    match mode {
        ScanMode::Reflective => "Flatbed".into(),
        ScanMode::Film => "Film".into(),
        ScanMode::Document => "Document feeder".into(),
    }
}
