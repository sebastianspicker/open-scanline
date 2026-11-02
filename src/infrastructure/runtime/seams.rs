//! Stable command, decoding, and test-adapter seams.

use super::artifact_watch::ArtifactWatch;
use crate::domain::image::ImageBuffer;
use crate::error::Result;
use crate::operation::CancellationToken;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    /// Process exit code; `None` when the child was ended by a signal.
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// The bounded amount of on-disk output a native acquisition is allowed to
/// materialize in one private artifact directory.
///
/// This is deliberately separate from stdout/stderr capture. Scanner drivers
/// often write images directly to files, so pipe limits do not bound their
/// disk use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactQuota {
    pub max_files: u64,
    pub max_bytes: u64,
}

pub trait CommandRunner: Send + Sync {
    /// Run `spec` to completion. `cancellation` interrupts a child blocked
    /// inside native acquisition; `artifacts`, when set, supervises
    /// direct-to-file scanner output while the command is running.
    fn run(
        &self,
        spec: &CommandSpec,
        timeout: Duration,
        cancelled: &Mutex<bool>,
        cancellation: Option<&CancellationToken>,
        artifacts: Option<ArtifactWatch<'_>>,
    ) -> Result<CommandOutput>;
}

pub trait ImageDecoder: Send + Sync {
    fn decode(&self, path: &Path) -> Result<ImageBuffer>;
}
