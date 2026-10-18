use crate::domain::acquisition::validate_page_limit;
use crate::domain::acquisition::{ScanMode, ScanRequest};
use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use crate::infrastructure::runtime::{
    document_batch_timeout, validate_artifact_quota, ArtifactQuota, CommandRunner, CommandSession,
    CommandSpec, ImageDecoder, TemporaryOutput,
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
    failure_name: &str,
) -> Result<ImageBuffer> {
    let command_output = runner.run_with_cancellation_and_artifact_quota(
        spec,
        scan_timeout(),
        session.cancelled(),
        session.cancellation_token().as_ref(),
        output.directory(),
        quota,
    )?;
    validate_artifact_quota(output.directory(), quota)?;
    session.decode_materialized_output(&command_output, output.path(), decoder, failure_name)
}

const SCAN_TIMEOUT_ENV: &str = "OPEN_SCANLINE_SCAN_TIMEOUT_SECS";
const PROBE_TIMEOUT_ENV: &str = "OPEN_SCANLINE_PROBE_TIMEOUT_SECS";
const DEFAULT_SCAN_TIMEOUT_SECS: u64 = 90;
const BATCH_BASE_SECONDS: u64 = 60;
const BATCH_MAX_SECONDS: u64 = 12 * 60 * 60;

/// Parse a timeout override in whole seconds; `None` for absent, malformed or
/// out-of-range (outside 1..=3600) values so the caller keeps its default.
pub(crate) fn parse_timeout_override(value: Option<&str>) -> Option<u64> {
    value
        .and_then(|text| text.trim().parse::<u64>().ok())
        .filter(|seconds| (1..=3600).contains(seconds))
}

fn timeout_override(name: &str) -> Option<u64> {
    parse_timeout_override(std::env::var(name).ok().as_deref())
}

/// Budget for one single-page scan command (default 90 s).
pub(crate) fn scan_timeout() -> Duration {
    Duration::from_secs(timeout_override(SCAN_TIMEOUT_ENV).unwrap_or(DEFAULT_SCAN_TIMEOUT_SECS))
}

/// Budget for availability, listing and help-inspection commands; `default`
/// applies when `OPEN_SCANLINE_PROBE_TIMEOUT_SECS` is unset or invalid.
pub(crate) fn probe_timeout(default: Duration) -> Duration {
    timeout_override(PROBE_TIMEOUT_ENV).map_or(default, Duration::from_secs)
}

/// Document-feeder budget; the scan-timeout override replaces the per-side
/// allowance of the built-in schedule.
pub(crate) fn batch_timeout(max_pages: u32) -> Duration {
    batch_timeout_with(max_pages, timeout_override(SCAN_TIMEOUT_ENV))
}

fn batch_timeout_with(max_pages: u32, per_side: Option<u64>) -> Duration {
    match per_side {
        None => document_batch_timeout(max_pages),
        Some(seconds) => Duration::from_secs(
            BATCH_BASE_SECONDS
                .saturating_add(u64::from(max_pages).saturating_mul(seconds))
                .min(BATCH_MAX_SECONDS),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_override_is_bounded_to_one_through_3600() {
        assert_eq!(parse_timeout_override(None), None);
        assert_eq!(parse_timeout_override(Some("")), None);
        assert_eq!(parse_timeout_override(Some("abc")), None);
        assert_eq!(parse_timeout_override(Some("-5")), None);
        assert_eq!(parse_timeout_override(Some("0")), None);
        assert_eq!(parse_timeout_override(Some("1")), Some(1));
        assert_eq!(parse_timeout_override(Some(" 120 ")), Some(120));
        assert_eq!(parse_timeout_override(Some("3600")), Some(3600));
        assert_eq!(parse_timeout_override(Some("3601")), None);
    }

    #[test]
    fn batch_budget_uses_override_per_side_and_stays_capped() {
        assert_eq!(batch_timeout_with(4, None), document_batch_timeout(4));
        assert_eq!(batch_timeout_with(4, Some(10)), Duration::from_secs(100));
        assert_eq!(
            batch_timeout_with(1000, Some(3600)),
            Duration::from_secs(BATCH_MAX_SECONDS)
        );
    }
}
