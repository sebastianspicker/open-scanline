use crate::domain::acquisition::validate_page_limit;
use crate::domain::acquisition::{ScanMode, ScanRequest};
use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use crate::infrastructure::runtime::{
    validate_artifact_quota, ArtifactQuota, CommandRunner, CommandSession, CommandSpec,
    ImageDecoder, TemporaryOutput,
};
use std::time::Duration;

pub(crate) fn validate_session_state(closed: bool, cancelled: bool) -> Result<()> {
    if closed {
        return Err(ScanError::Other("session closed".into()));
    }
    if cancelled {
        return Err(ScanError::Cancelled("scan cancelled".into()));
    }
    Ok(())
}

pub(crate) fn validate_batch_request(
    request: &ScanRequest,
    maximum: u32,
    closed: bool,
    cancelled: bool,
) -> Result<()> {
    validate_page_limit(maximum)?;
    validate_session_state(closed, cancelled)?;
    if request.duplex && request.mode != ScanMode::Document {
        return Err(ScanError::Invalid(
            "duplex is only valid with a document feeder source".into(),
        ));
    }
    Ok(())
}

pub(crate) fn run_materialized_scan_command(
    session: &CommandSession,
    runner: &dyn CommandRunner,
    decoder: &dyn ImageDecoder,
    spec: &CommandSpec,
    output: &TemporaryOutput,
    quota: ArtifactQuota,
    request: &ScanRequest,
    failure_name: &str,
) -> Result<ImageBuffer> {
    let command_output = runner.run_with_cancellation_and_artifact_quota(
        spec,
        Duration::from_secs(90),
        session.cancelled(),
        session.cancellation_token().as_ref(),
        output.directory(),
        quota,
    )?;
    validate_artifact_quota(output.directory(), quota)?;
    session.decode_materialized_output(
        &command_output,
        output.path(),
        decoder,
        request,
        failure_name,
    )
}
