//! Runtime facilities shared by command-backed integrations.

mod artifact_watch;
pub(crate) mod atomic_publish;
mod command;
mod output_capture;
pub(crate) mod platform;
mod seams;
mod session;
mod supervision;
mod temporary_output;

pub use seams::{ArtifactQuota, CommandOutput, CommandRunner, CommandSpec, ImageDecoder};

pub(crate) use artifact_watch::{
    artifact_quota_for_request, validate_artifact_quota, ArtifactWatch,
};
pub(crate) use command::{
    availability_probe_succeeds, document_batch_timeout, document_batch_timeout_per_side,
    run_contained_command, run_contained_command_with_artifact_quota,
};
pub(crate) use session::CommandSession;

/// Held around every child spawn in the process. On Windows, the supervised
/// spawn creates inheritable pipe ends; without this lock a concurrent spawn
/// could inherit them and keep the pipes open.
pub(crate) fn lock_process_spawn() -> std::sync::MutexGuard<'static, ()> {
    static SPAWN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    SPAWN_LOCK.lock().unwrap_or_else(|error| error.into_inner())
}
pub(crate) use temporary_output::TemporaryOutput;
