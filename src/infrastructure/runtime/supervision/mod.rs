//! Cancellation-aware command process-tree supervision.

mod platform;

use super::artifact_watch::validate_artifact_quota;
use super::command::CommandOutputLimits;
use super::output_capture::{drain_command_stream, join_command_stream, CommandStreamReader};
use super::seams::{ArtifactQuota, CommandOutput, CommandSpec};
use crate::error::{Result, ScanError};
use crate::workflows::operation::CancellationToken;
use platform::SupervisedChild;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(super) fn run_command_with_capture_limit(
    spec: &CommandSpec,
    timeout: Duration,
    cancelled: &Mutex<bool>,
    cancellation: Option<&CancellationToken>,
    command_name: &str,
    cancellation_message: &str,
    output_limits: CommandOutputLimits<'_>,
) -> Result<CommandOutput> {
    let (child, containment) = platform::spawn_contained_command(spec).map_err(|error| {
        ScanError::Unsupported(format!("{command_name} failed to start: {error}"))
    })?;
    supervise_command_child(
        Supervision {
            child,
            containment,
            timeout,
            cancelled,
            cancellation,
            command_name,
            cancellation_message,
            capture_limit: output_limits.capture_bytes,
            artifact_watch: output_limits
                .artifacts
                .map(|watch| (watch.directory, watch.quota)),
        },
        |child| child.poll(),
    )
}

struct Supervision<'a, C: platform::SupervisedChild> {
    child: C,
    containment: platform::ProcessContainment,
    timeout: Duration,
    cancelled: &'a Mutex<bool>,
    cancellation: Option<&'a CancellationToken>,
    command_name: &'a str,
    cancellation_message: &'a str,
    capture_limit: usize,
    artifact_watch: Option<(&'a Path, ArtifactQuota)>,
}

fn supervise_command_child<C, F>(
    supervision: Supervision<'_, C>,
    mut poll: F,
) -> Result<CommandOutput>
where
    C: platform::SupervisedChild,
    F: FnMut(&mut C) -> std::io::Result<Option<std::process::ExitStatus>>,
{
    let Supervision {
        mut child,
        containment,
        timeout,
        cancelled,
        cancellation,
        command_name,
        cancellation_message,
        capture_limit,
        artifact_watch,
    } = supervision;
    let stdout = child.take_stdout().expect("stdout was configured as piped");
    let stderr = child.take_stderr().expect("stderr was configured as piped");
    let stdout_reader = drain_command_stream(stdout, capture_limit);
    let stderr_reader = drain_command_stream(stderr, capture_limit);
    let deadline = Instant::now() + timeout;
    loop {
        if *cancelled.lock().unwrap_or_else(|error| error.into_inner())
            || cancellation.is_some_and(CancellationToken::is_cancelled)
        {
            terminate_child_and_readers(&mut child, &containment, stdout_reader, stderr_reader);
            return Err(ScanError::Cancelled(cancellation_message.into()));
        }
        if let Some((directory, quota)) = artifact_watch {
            if let Err(error) = validate_artifact_quota(directory, quota) {
                terminate_child_and_readers(&mut child, &containment, stdout_reader, stderr_reader);
                return Err(error);
            }
        }
        match poll(&mut child) {
            Ok(Some(_)) => {
                let status = child.wait().map_err(|error| {
                    ScanError::Unsupported(format!("{command_name} output failed: {error}"))
                })?;
                if let Some((directory, quota)) = artifact_watch {
                    if let Err(error) = validate_artifact_quota(directory, quota) {
                        containment.terminate(&mut child);
                        return Err(error);
                    }
                }
                let stdout = match join_command_stream(stdout_reader) {
                    Ok(stdout) => stdout,
                    Err(error) => {
                        containment.terminate(&mut child);
                        return Err(error);
                    }
                };
                let stderr = match join_command_stream(stderr_reader) {
                    Ok(stderr) => stderr,
                    Err(error) => {
                        containment.terminate(&mut child);
                        return Err(error);
                    }
                };
                if stdout.overflowed || stderr.overflowed {
                    return Err(ScanError::Unsupported(format!(
                        "{command_name} output exceeded the {capture_limit} byte capture limit"
                    )));
                }
                return Ok(CommandOutput {
                    success: status.success(),
                    stdout: stdout.bytes,
                    stderr: stderr.bytes,
                });
            }
            Ok(None) if Instant::now() >= deadline => {
                terminate_child_and_readers(&mut child, &containment, stdout_reader, stderr_reader);
                return Err(ScanError::Unsupported(format!("{command_name} timed out")));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                terminate_child_and_readers(&mut child, &containment, stdout_reader, stderr_reader);
                return Err(ScanError::Unsupported(format!(
                    "{command_name} failed: {error}"
                )));
            }
        }
    }
}

fn terminate_child_and_readers(
    child: &mut impl platform::SupervisedChild,
    containment: &platform::ProcessContainment,
    stdout_reader: CommandStreamReader,
    stderr_reader: CommandStreamReader,
) {
    containment.terminate(child);
    let _ = child.wait();
    // Do not join here. A deliberately daemonized Unix descendant can retain
    // the inherited pipe after escaping its process group; dropping a handle
    // detaches the reader instead of making cancellation wait indefinitely.
    drop(stdout_reader);
    drop(stderr_reader);
}
