//! Command entry points and policy-level capture limits.

use super::artifact_watch::ArtifactWatch;
use super::seams::{CommandOutput, CommandSpec};
use super::supervision::run_command_with_capture_limit;
use crate::error::Result;
use crate::workflows::operation::CancellationToken;
use std::sync::Mutex;
use std::time::Duration;

/// Bound a document-feeder process while allowing the deadline to scale with
/// the requested logical side count. A large feeder job can legitimately run
/// for hours at high DPI, but it must never become unbounded.
pub(crate) fn document_batch_timeout(max_pages: u32) -> Duration {
    const BASE_SECONDS: u64 = 60;
    const SECONDS_PER_SIDE: u64 = 45;
    const MAX_SECONDS: u64 = 12 * 60 * 60;
    Duration::from_secs(
        BASE_SECONDS
            .saturating_add(u64::from(max_pages).saturating_mul(SECONDS_PER_SIDE))
            .min(MAX_SECONDS),
    )
}

const COMMAND_STREAM_CAPTURE_LIMIT: usize = 8 * 1024 * 1024;
const AVAILABILITY_PROBE_CAPTURE_LIMIT: usize = 64 * 1024;

#[derive(Debug, Clone, Copy)]
pub(super) struct CommandOutputLimits<'a> {
    pub(super) capture_bytes: usize,
    pub(super) artifacts: Option<ArtifactWatch<'a>>,
}

pub(crate) fn run_command(
    spec: &CommandSpec,
    timeout: Duration,
    cancelled: &Mutex<bool>,
    command_name: &str,
    cancellation_message: &str,
) -> Result<CommandOutput> {
    run_contained_command(
        spec,
        timeout,
        cancelled,
        None,
        command_name,
        cancellation_message,
    )
}

/// Run a short, low-output command used only to determine whether a backend is
/// usable. This deliberately shares the process-tree supervisor with scans so
/// a wedged driver probe cannot stall backend discovery.
pub(crate) fn availability_probe_succeeds(
    spec: &CommandSpec,
    timeout: Duration,
    command_name: &str,
) -> bool {
    let cancelled = Mutex::new(false);
    run_command_with_capture_limit(
        spec,
        timeout,
        &cancelled,
        None,
        command_name,
        "availability probe cancelled",
        CommandOutputLimits {
            capture_bytes: AVAILABILITY_PROBE_CAPTURE_LIMIT,
            artifacts: None,
        },
    )
    .is_ok_and(|output| output.success)
}

pub(crate) fn run_command_with_cancellation(
    spec: &CommandSpec,
    timeout: Duration,
    cancelled: &Mutex<bool>,
    cancellation: Option<&CancellationToken>,
    command_name: &str,
    cancellation_message: &str,
) -> Result<CommandOutput> {
    run_contained_command(
        spec,
        timeout,
        cancelled,
        cancellation,
        command_name,
        cancellation_message,
    )
}

/// Run a contained command with bounded pipes, cancellation, and process-tree
/// cleanup, but no direct-to-file artifact monitoring.
pub(crate) fn run_contained_command(
    spec: &CommandSpec,
    timeout: Duration,
    cancelled: &Mutex<bool>,
    cancellation: Option<&CancellationToken>,
    command_name: &str,
    cancellation_message: &str,
) -> Result<CommandOutput> {
    run_command_with_capture_limit(
        spec,
        timeout,
        cancelled,
        cancellation,
        command_name,
        cancellation_message,
        CommandOutputLimits {
            capture_bytes: COMMAND_STREAM_CAPTURE_LIMIT,
            artifacts: None,
        },
    )
}

/// Run a contained command while repeatedly checking its private artifact
/// directory. A quota breach terminates and reaps the whole contained tree
/// before returning, so callers never decode untrusted over-limit output.
pub(crate) fn run_contained_command_with_artifact_quota(
    spec: &CommandSpec,
    timeout: Duration,
    cancelled: &Mutex<bool>,
    cancellation: Option<&CancellationToken>,
    command_name: &str,
    cancellation_message: &str,
    artifact_watch: ArtifactWatch<'_>,
) -> Result<CommandOutput> {
    run_command_with_capture_limit(
        spec,
        timeout,
        cancelled,
        cancellation,
        command_name,
        cancellation_message,
        CommandOutputLimits {
            capture_bytes: COMMAND_STREAM_CAPTURE_LIMIT,
            artifacts: Some(artifact_watch),
        },
    )
}
