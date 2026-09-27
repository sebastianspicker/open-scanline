use super::*;

#[derive(Debug)]
pub(crate) enum FetchDocumentError {
    Cancelled,
    Exhausted,
    TimedOut,
    RetryableTimeout,
    Failed(String),
}

pub(crate) fn next_document_path(job: &ScanJob) -> String {
    if job.path.ends_with("/NextDocument") {
        job.path.clone()
    } else {
        format!("{}/NextDocument", job.path)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ScanJob {
    pub(crate) path: String,
}

pub(crate) fn canonical_job_path(
    headers: &str,
    body: &[u8],
    root: &str,
    endpoint: &Endpoint,
) -> Option<String> {
    if let Some(location) = job_location(headers, body) {
        return normalize_job_location(&location, endpoint);
    }
    parse_job_id(headers, body).map(|id| format!("/{root}/ScanJobs/{id}"))
}

pub(crate) fn job_location(headers: &str, body: &[u8]) -> Option<String> {
    headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("location")
                .then(|| value.trim().to_string())
        })
        .or_else(|| {
            let mut reader = Reader::from_reader(body);
            reader.config_mut().trim_text(true);
            let mut buffer = Vec::new();
            let mut in_job_uri = false;
            loop {
                match reader.read_event_into(&mut buffer) {
                    Ok(Event::Start(event)) => {
                        in_job_uri = local_name(event.name().as_ref()) == "joburi"
                    }
                    Ok(Event::Text(event)) if in_job_uri => {
                        return event.decode().ok().map(|v| v.into_owned());
                    }
                    Ok(Event::End(event)) if local_name(event.name().as_ref()) == "joburi" => {
                        in_job_uri = false
                    }
                    Ok(Event::Eof) | Err(_) => return None,
                    _ => {}
                }
                buffer.clear();
            }
        })
}

pub(crate) fn normalize_job_location(location: &str, endpoint: &Endpoint) -> Option<String> {
    if invalid_location(location) {
        return None;
    }
    let path = job_location_path(location, endpoint)?;
    valid_job_path(&path).then(|| path.trim_end_matches('/').to_string())
}

pub(crate) fn invalid_location(location: &str) -> bool {
    location.is_empty()
        || location
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
}

pub(crate) fn job_location_path(location: &str, endpoint: &Endpoint) -> Option<String> {
    if location.starts_with('/') {
        return Some(location.to_string());
    }
    let (secure, authority_and_path) = absolute_location_parts(location)?;
    absolute_job_path(authority_and_path, secure, endpoint)
}

pub(crate) fn absolute_location_parts(location: &str) -> Option<(bool, &str)> {
    location
        .strip_prefix("https://")
        .map(|value| (true, value))
        .or_else(|| location.strip_prefix("http://").map(|value| (false, value)))
}

pub(crate) fn absolute_job_path(
    authority_and_path: &str,
    secure: bool,
    endpoint: &Endpoint,
) -> Option<String> {
    let (authority, path) = authority_and_path.split_once('/')?;
    let scheme = if secure { "https" } else { "http" };
    (endpoint.secure == secure && parse_endpoint(&format!("{scheme}://{authority}"))? == *endpoint)
        .then(|| format!("/{path}"))
}

pub(crate) fn valid_job_path(path: &str) -> bool {
    !path.contains(['?', '#'])
        && path.contains("/ScanJobs/")
        && !path.split('/').any(|segment| matches!(segment, "." | ".."))
}

pub(crate) fn document_path(path: &std::path::Path) -> Result<std::path::PathBuf> {
    let bytes = crate::infrastructure::media::read_magic(path)?;
    let extension = if bytes.starts_with(b"\x89PNG") {
        Some("png")
    } else if bytes.starts_with(b"\xff\xd8") {
        Some("jpg")
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        Some("tiff")
    } else {
        None
    };
    let Some(extension) = extension else {
        return Ok(path.to_path_buf());
    };
    let decode_path = path.with_extension(extension);
    if let Err(error) = std::fs::rename(path, &decode_path) {
        let _ = std::fs::remove_file(path);
        return Err(error.into());
    }
    Ok(decode_path)
}

pub(crate) fn parse_escl_id(device_id: &str) -> Option<Endpoint> {
    // escl:host:port, escl:https@host:port, or legacy escl:host
    let rest = device_id.strip_prefix("escl:")?;
    if rest == "sim" {
        return Some(Endpoint {
            host: "sim".into(),
            port: 0,
            secure: false,
        });
    }
    let (secure, rest) = rest
        .strip_prefix("https@")
        .map(|value| (true, value))
        .unwrap_or((false, rest));
    let endpoint = parse_endpoint(&format!(
        "{}://{rest}",
        if secure { "https" } else { "http" }
    ))?;
    Some(endpoint)
}

/// Open eSCL session for listed device or sim.
pub fn open(device_id: &str) -> Result<EsclDeviceSession> {
    open_with_cancellation(device_id, None)
}

pub fn open_with_cancellation(
    device_id: &str,
    cancellation: Option<&CancellationToken>,
) -> Result<EsclDeviceSession> {
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err(ScanError::Cancelled("discovery cancelled".into()));
    }
    let id = device_id;
    validate_device_id(id)?;
    if id == "escl:sim" {
        return Ok(simulated_session(id));
    }
    let endpoint =
        parse_escl_id(id).ok_or_else(|| ScanError::DeviceNotFound(format!("bad eSCL id: {id}")))?;
    if endpoint.host == "sim" {
        return Ok(EsclDeviceSession::new(id.into(), endpoint, true));
    }
    authorize_listed_or_allowed(id, &endpoint, cancellation)?;
    Ok(EsclDeviceSession::new(id.into(), endpoint, false))
}

fn validate_device_id(id: &str) -> Result<()> {
    if id.starts_with("escl:") || id == "escl" {
        return Ok(());
    }
    Err(ScanError::DeviceNotFound(format!(
        "not an eSCL device id: {id}"
    )))
}

fn simulated_session(id: &str) -> EsclDeviceSession {
    EsclDeviceSession::new(
        id.into(),
        Endpoint {
            host: "sim".into(),
            port: 0,
            secure: false,
        },
        true,
    )
}

fn authorize_listed_or_allowed(
    id: &str,
    endpoint: &Endpoint,
    cancellation: Option<&CancellationToken>,
) -> Result<()> {
    let listed = list_escl_devices_safe_with_cancellation(cancellation);
    let allowed = env_hosts().iter().any(|candidate| candidate == endpoint);
    if listed.iter().any(|device| device.id == id) || allowed {
        return Ok(());
    }
    Err(ScanError::DeviceNotFound(format!(
        "unknown eSCL device: {id}"
    )))
}

/// Open a syntactically valid explicit eSCL id without discovery authorization.
/// Callers must opt in through the device-opening policy; [`open`] remains
/// discovery/allow-list restricted.
pub fn open_explicit_id(device_id: &str) -> Result<EsclDeviceSession> {
    let endpoint = parse_escl_id(device_id)
        .ok_or_else(|| ScanError::DeviceNotFound(format!("bad eSCL id: {device_id}")))?;
    Ok(EsclDeviceSession::new(
        device_id.into(),
        endpoint.clone(),
        endpoint.host == "sim",
    ))
}

/// Open a strict explicit endpoint without requiring discovery or an allow-list entry.
///
/// This opt-in bypasses the normal discovery/`OPEN_SCANLINE_ESCL_HOSTS` authorization check.
pub fn open_unlisted_endpoint(endpoint: &str) -> Result<EsclDeviceSession> {
    let endpoint = parse_endpoint(endpoint)
        .ok_or_else(|| ScanError::DeviceNotFound("invalid explicit eSCL endpoint".into()))?;
    Ok(EsclDeviceSession::new(
        endpoint.device_id(),
        endpoint,
        false,
    ))
}
