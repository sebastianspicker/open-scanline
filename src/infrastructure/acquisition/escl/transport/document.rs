use super::{scanner_agent_with_phase_timeout, Endpoint};
use crate::infrastructure::runtime::TemporaryOutput;
use crate::operation::CancellationToken;
use std::fs::File;
use std::io::{Read, Write};
use std::time::Duration;

use super::super::FetchDocumentError;

pub(crate) fn http_get_to_temporary_output_with_phase_timeout_and_cancellation(
    endpoint: &Endpoint,
    path: &str,
    global_timeout: Duration,
    setup_timeout: Duration,
    response_limit: u64,
    cancellation: Option<&CancellationToken>,
) -> std::result::Result<(u16, Option<TemporaryOutput>), FetchDocumentError> {
    let response =
        fetch_next_document(endpoint, path, global_timeout, setup_timeout, cancellation)?;
    let (parts, body) = response.into_parts();
    let status = parts.status.as_u16();
    validate_next_document_response(&parts, status, response_limit)?;
    if status != 200 {
        return Ok((status, None));
    }
    write_next_document(body, status, response_limit)
}

fn fetch_next_document(
    endpoint: &Endpoint,
    path: &str,
    global_timeout: Duration,
    setup_timeout: Duration,
    cancellation: Option<&CancellationToken>,
) -> std::result::Result<ureq::http::Response<ureq::Body>, FetchDocumentError> {
    scanner_agent_with_phase_timeout(
        global_timeout,
        setup_timeout,
        setup_timeout,
        cancellation.cloned(),
    )
    .get(&endpoint.url(path))
    .call()
    .map_err(map_next_document_error)
}

fn map_next_document_error(error: ureq::Error) -> FetchDocumentError {
    match error {
        ureq::Error::Io(error) if error.kind() == std::io::ErrorKind::Interrupted => {
            FetchDocumentError::Cancelled
        }
        ureq::Error::Timeout(_) => FetchDocumentError::RetryableTimeout,
        _ => FetchDocumentError::Failed(format!("NextDocument no response: {error}")),
    }
}

fn validate_next_document_response(
    parts: &ureq::http::response::Parts,
    status: u16,
    response_limit: u64,
) -> std::result::Result<(), FetchDocumentError> {
    if parts.status.is_redirection() {
        return Err(FetchDocumentError::Failed(
            "NextDocument redirect rejected".into(),
        ));
    }
    if status == 200 && content_length_exceeds(parts, response_limit) {
        return Err(FetchDocumentError::Failed(
            "NextDocument response exceeds byte limit".into(),
        ));
    }
    Ok(())
}

fn content_length_exceeds(parts: &ureq::http::response::Parts, limit: u64) -> bool {
    parts
        .headers
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > limit)
}

fn write_next_document(
    body: ureq::Body,
    status: u16,
    response_limit: u64,
) -> std::result::Result<(u16, Option<TemporaryOutput>), FetchDocumentError> {
    let output = TemporaryOutput::new("escl", "bin")
        .map_err(|error| FetchDocumentError::Failed(error.to_string()))?;
    let mut file = File::create(output.path())
        .map_err(|error| FetchDocumentError::Failed(error.to_string()))?;
    let copied = copy_document_body(body.into_reader(), &mut file, response_limit)?;
    file.flush()
        .map_err(|error| FetchDocumentError::Failed(error.to_string()))?;
    (copied != 0)
        .then_some((status, Some(output)))
        .ok_or_else(|| FetchDocumentError::Failed("NextDocument returned an empty response".into()))
}

fn copy_document_body<R: Read>(
    mut reader: R,
    file: &mut File,
    response_limit: u64,
) -> std::result::Result<u64, FetchDocumentError> {
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(map_document_read_error)?;
        if read == 0 {
            return Ok(copied);
        }
        copied = write_document_chunk(file, &buffer[..read], copied, response_limit)?;
    }
}

fn write_document_chunk(
    file: &mut File,
    chunk: &[u8],
    copied: u64,
    response_limit: u64,
) -> std::result::Result<u64, FetchDocumentError> {
    let copied = copied.checked_add(chunk.len() as u64).ok_or_else(|| {
        FetchDocumentError::Failed("NextDocument response exceeds byte limit".into())
    })?;
    if copied > response_limit {
        return Err(FetchDocumentError::Failed(
            "NextDocument response exceeds byte limit".into(),
        ));
    }
    file.write_all(chunk)
        .map_err(|error| FetchDocumentError::Failed(error.to_string()))?;
    Ok(copied)
}

fn map_document_read_error(error: std::io::Error) -> FetchDocumentError {
    if error.kind() == std::io::ErrorKind::Interrupted {
        return FetchDocumentError::Cancelled;
    }
    if matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        FetchDocumentError::TimedOut
    } else {
        FetchDocumentError::Failed(error.to_string())
    }
}
