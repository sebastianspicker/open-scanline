//! Shared command-line-tool [`CommandRunner`] used by the SANE and WIA
//! adapters, which differ only in the program label and cancellation message
//! they attach to failures.

use crate::error::{Result, ScanError};
use crate::infrastructure::acquisition::ScanPagesResult;
use crate::infrastructure::runtime::{
    run_command, run_command_with_cancellation, run_contained_command_with_artifact_quota,
    ArtifactQuota, ArtifactWatch, CommandOutput, CommandRunner, CommandSpec,
};
use crate::operation::CancellationToken;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

/// Runs a command-line acquisition tool as a real OS process.
///
/// `program_label` names the tool in "not found"/failure messages;
/// `cancelled_message` is the [`crate::error::ScanError::Cancelled`] text used
/// when the operation-wide token or the per-call flag is already set.
pub struct SystemCommandRunner {
    program_label: &'static str,
    cancelled_message: &'static str,
}

impl SystemCommandRunner {
    pub const fn new(program_label: &'static str, cancelled_message: &'static str) -> Self {
        Self {
            program_label,
            cancelled_message,
        }
    }
}

impl CommandRunner for SystemCommandRunner {
    fn run(
        &self,
        spec: &CommandSpec,
        timeout: Duration,
        cancelled: &Mutex<bool>,
    ) -> Result<CommandOutput> {
        run_command(
            spec,
            timeout,
            cancelled,
            self.program_label,
            self.cancelled_message,
        )
    }

    fn run_with_cancellation(
        &self,
        spec: &CommandSpec,
        timeout: Duration,
        cancelled: &Mutex<bool>,
        cancellation: Option<&CancellationToken>,
    ) -> Result<CommandOutput> {
        run_command_with_cancellation(
            spec,
            timeout,
            cancelled,
            cancellation,
            self.program_label,
            self.cancelled_message,
        )
    }

    fn run_with_cancellation_and_artifact_quota(
        &self,
        spec: &CommandSpec,
        timeout: Duration,
        cancelled: &Mutex<bool>,
        cancellation: Option<&CancellationToken>,
        artifact_directory: &Path,
        artifact_quota: ArtifactQuota,
    ) -> Result<CommandOutput> {
        run_contained_command_with_artifact_quota(
            spec,
            timeout,
            cancelled,
            cancellation,
            self.program_label,
            self.cancelled_message,
            ArtifactWatch {
                directory: artifact_directory,
                quota: artifact_quota,
            },
        )
    }
}

/// `Ok` when a command-backed batch emitted no more than `maximum` pages;
/// otherwise a descriptive error naming `program_label` (e.g. `"scanimage"`
/// or `"WIA"`).
pub(crate) fn validate_batch_page_count(
    actual: usize,
    maximum: u32,
    program_label: &str,
) -> Result<()> {
    if actual <= maximum as usize {
        return Ok(());
    }
    Err(ScanError::Other(format!(
        "{program_label} emitted {actual} pages beyond the requested limit {maximum}"
    )))
}

/// [`ScanPagesResult::limit_reached`] when every requested page was emitted,
/// otherwise [`ScanPagesResult::feeder_exhausted`].
pub(crate) fn command_backed_pages_result(emitted: u32, maximum: u32) -> ScanPagesResult {
    if emitted == maximum {
        ScanPagesResult::limit_reached(emitted)
    } else {
        ScanPagesResult::feeder_exhausted(emitted)
    }
}
