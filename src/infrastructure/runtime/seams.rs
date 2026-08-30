//! Stable command, decoding, and test-adapter seams.

use crate::domain::image::ImageBuffer;
use crate::error::Result;
use crate::workflows::operation::CancellationToken;
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
    fn run(
        &self,
        spec: &CommandSpec,
        timeout: Duration,
        cancelled: &Mutex<bool>,
    ) -> Result<CommandOutput>;

    /// Token-aware extension point. Existing injected runners only need to
    /// implement [`CommandRunner::run`]; production overrides this to interrupt
    /// a child blocked inside native acquisition.
    fn run_with_cancellation(
        &self,
        spec: &CommandSpec,
        timeout: Duration,
        cancelled: &Mutex<bool>,
        cancellation: Option<&CancellationToken>,
    ) -> Result<CommandOutput> {
        let _ = cancellation;
        self.run(spec, timeout, cancelled)
    }

    /// Artifact-aware extension point. The default keeps injected test and
    /// integration runners source-compatible; native runners override it to
    /// supervise direct-to-file scanner output while the command is running.
    fn run_with_cancellation_and_artifact_quota(
        &self,
        spec: &CommandSpec,
        timeout: Duration,
        cancelled: &Mutex<bool>,
        cancellation: Option<&CancellationToken>,
        artifact_directory: &Path,
        artifact_quota: ArtifactQuota,
    ) -> Result<CommandOutput> {
        let _ = (artifact_directory, artifact_quota);
        self.run_with_cancellation(spec, timeout, cancelled, cancellation)
    }
}

pub trait ImageDecoder: Send + Sync {
    fn decode(&self, path: &Path) -> Result<ImageBuffer>;
}

pub struct NativeImageDecoder;

impl ImageDecoder for NativeImageDecoder {
    fn decode(&self, path: &Path) -> Result<ImageBuffer> {
        crate::infrastructure::media::load_image(path)
    }
}
