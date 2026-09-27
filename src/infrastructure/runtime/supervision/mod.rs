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

struct StreamReaders {
    stdout: CommandStreamReader,
    stderr: CommandStreamReader,
}

enum SupervisionAction {
    Continue,
    Finish,
    Fail(ScanError),
}

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
    mut supervision: Supervision<'_, C>,
    mut poll: F,
) -> Result<CommandOutput>
where
    C: platform::SupervisedChild,
    F: FnMut(&mut C) -> std::io::Result<Option<std::process::ExitStatus>>,
{
    let readers = start_stream_readers(&mut supervision.child, supervision.capture_limit);
    supervise_child_loop(supervision, readers, &mut poll)
}

fn supervise_child_loop<C, F>(
    mut supervision: Supervision<'_, C>,
    readers: StreamReaders,
    poll: &mut F,
) -> Result<CommandOutput>
where
    C: platform::SupervisedChild,
    F: FnMut(&mut C) -> std::io::Result<Option<std::process::ExitStatus>>,
{
    let deadline = Instant::now() + supervision.timeout;
    loop {
        match next_supervision_action(&mut supervision, deadline, poll) {
            SupervisionAction::Finish => {
                return finish_child(
                    &mut supervision.child,
                    &supervision.containment,
                    readers,
                    supervision.artifact_watch,
                    supervision.command_name,
                    supervision.capture_limit,
                );
            }
            SupervisionAction::Fail(error) => {
                terminate_child_and_readers(
                    &mut supervision.child,
                    &supervision.containment,
                    readers,
                );
                return Err(error);
            }
            SupervisionAction::Continue => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}

fn next_supervision_action<C, F>(
    supervision: &mut Supervision<'_, C>,
    deadline: Instant,
    poll: &mut F,
) -> SupervisionAction
where
    C: platform::SupervisedChild,
    F: FnMut(&mut C) -> std::io::Result<Option<std::process::ExitStatus>>,
{
    if let Some(error) = supervision_interruption(
        supervision.cancelled,
        supervision.cancellation,
        supervision.cancellation_message,
        supervision.artifact_watch,
    ) {
        return SupervisionAction::Fail(error);
    }
    match poll(&mut supervision.child) {
        Ok(Some(_)) => SupervisionAction::Finish,
        Ok(None) if Instant::now() >= deadline => SupervisionAction::Fail(ScanError::Unsupported(
            format!("{} timed out", supervision.command_name),
        )),
        Ok(None) => SupervisionAction::Continue,
        Err(error) => SupervisionAction::Fail(ScanError::Unsupported(format!(
            "{} failed: {error}",
            supervision.command_name
        ))),
    }
}

fn start_stream_readers(
    child: &mut impl platform::SupervisedChild,
    capture_limit: usize,
) -> StreamReaders {
    StreamReaders {
        stdout: drain_command_stream(
            child.take_stdout().expect("stdout was configured as piped"),
            capture_limit,
        ),
        stderr: drain_command_stream(
            child.take_stderr().expect("stderr was configured as piped"),
            capture_limit,
        ),
    }
}

fn supervision_interruption(
    cancelled: &Mutex<bool>,
    cancellation: Option<&CancellationToken>,
    cancellation_message: &str,
    artifact_watch: Option<(&Path, ArtifactQuota)>,
) -> Option<ScanError> {
    if *cancelled.lock().unwrap_or_else(|error| error.into_inner())
        || cancellation.is_some_and(CancellationToken::is_cancelled)
    {
        return Some(ScanError::Cancelled(cancellation_message.into()));
    }
    artifact_watch.and_then(|(directory, quota)| validate_artifact_quota(directory, quota).err())
}

fn finish_child(
    child: &mut impl platform::SupervisedChild,
    containment: &platform::ProcessContainment,
    readers: StreamReaders,
    artifact_watch: Option<(&Path, ArtifactQuota)>,
    command_name: &str,
    capture_limit: usize,
) -> Result<CommandOutput> {
    let status = child.wait().map_err(|error| {
        ScanError::Unsupported(format!("{command_name} output failed: {error}"))
    })?;
    validate_finished_artifacts(child, containment, artifact_watch)?;
    let stdout = join_command_reader(child, containment, readers.stdout)?;
    let stderr = join_command_reader(child, containment, readers.stderr)?;
    if stdout.overflowed || stderr.overflowed {
        return Err(ScanError::Unsupported(format!(
            "{command_name} output exceeded the {capture_limit} byte capture limit"
        )));
    }
    Ok(CommandOutput {
        success: status.success(),
        stdout: stdout.bytes,
        stderr: stderr.bytes,
    })
}

fn join_command_reader(
    child: &mut impl platform::SupervisedChild,
    containment: &platform::ProcessContainment,
    reader: CommandStreamReader,
) -> Result<super::output_capture::DrainedCommandStream> {
    join_command_stream(reader).inspect_err(|_| containment.terminate(child))
}

fn validate_finished_artifacts(
    child: &mut impl platform::SupervisedChild,
    containment: &platform::ProcessContainment,
    artifact_watch: Option<(&Path, ArtifactQuota)>,
) -> Result<()> {
    let Some((directory, quota)) = artifact_watch else {
        return Ok(());
    };
    if let Err(error) = validate_artifact_quota(directory, quota) {
        containment.terminate(child);
        return Err(error);
    }
    Ok(())
}

fn terminate_child_and_readers(
    child: &mut impl platform::SupervisedChild,
    containment: &platform::ProcessContainment,
    readers: StreamReaders,
) {
    containment.terminate(child);
    let _ = child.wait();
    // Do not join here. A deliberately daemonized Unix descendant can retain
    // the inherited pipe after escaping its process group; dropping a handle
    // detaches the reader instead of making cancellation wait indefinitely.
    drop(readers);
}
