//! Runtime facilities shared by command-backed integrations.

mod artifact_watch;
pub(crate) mod atomic_publish;
mod command;
mod device_listing;
mod output_capture;
pub(crate) mod platform;
mod seams;
mod session;
mod supervision;
mod temporary_output;

pub use seams::{
    ArtifactQuota, CommandOutput, CommandRunner, CommandSpec, ImageDecoder, NativeImageDecoder,
};

pub(crate) use artifact_watch::{
    artifact_quota_for_request, validate_artifact_quota, ArtifactWatch,
};
pub(crate) use command::{
    availability_probe_succeeds, document_batch_timeout, run_command,
    run_command_with_cancellation, run_contained_command_with_artifact_quota,
};
pub(crate) use device_listing::{parse_pipe_devices, simulate_backends};
pub(crate) use session::CommandSession;
pub(crate) use temporary_output::TemporaryOutput;
