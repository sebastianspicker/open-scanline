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
        if let Some(path) = normalize_job_location(&location, endpoint) {
            return Some(path);
        }
        // A device found as `host.local` often answers with its IP address.
        // Never follow another host, but keep a well-formed job path on the
        // pinned endpoint.
        if let Some(path) = cross_origin_job_path(&location, root) {
            return Some(path);
        }
    }
    parse_job_id(headers, body).and_then(|id| job_path(&format!("/{root}/ScanJobs/{id}")))
}

/// Path of an absolute `http(s)` Location, accepted only as exactly
/// `/<root>/ScanJobs/<id>`; the authority is ignored.
pub(crate) fn cross_origin_job_path(location: &str, root: &str) -> Option<String> {
    if invalid_location(location) {
        return None;
    }
    let (_, authority_and_path) = absolute_location_parts(location)?;
    let (_, path) = authority_and_path.split_once('/')?;
    let path = job_path(&format!("/{path}"))?;
    path[1..]
        .split('/')
        .next()?
        .eq_ignore_ascii_case(root)
        .then_some(path)
}

/// The one rule for a job path: exactly `/<root>/ScanJobs/<id>` once a trailing
/// `/` is trimmed, so a collection path or a `..`/encoded segment never passes.
pub(crate) fn job_path(path: &str) -> Option<String> {
    let path = path.trim_end_matches('/');
    let mut segments = path.strip_prefix('/')?.split('/');
    let (root, jobs, id) = (segments.next()?, segments.next()?, segments.next()?);
    let well_formed = segments.next().is_none()
        && jobs == "ScanJobs"
        && ![root, id]
            .iter()
            .any(|segment| matches!(*segment, "" | "." | ".."))
        && !path.contains(['?', '#', '%', '\\'])
        && !path.chars().any(char::is_control);
    well_formed.then(|| path.to_string())
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
            let mut buffer = Vec::new();
            let mut in_job_uri = false;
            let mut uri = String::new();
            loop {
                match reader.read_event_into(&mut buffer) {
                    Ok(Event::Start(event)) => {
                        in_job_uri = local_name(event.name().as_ref()) == "joburi";
                        uri.clear();
                    }
                    Ok(Event::Text(event)) if in_job_uri => uri.push_str(&event.decode().ok()?),
                    Ok(Event::GeneralRef(event)) if in_job_uri => {
                        uri.push_str(&general_ref_text(&event)?)
                    }
                    Ok(Event::End(event)) if local_name(event.name().as_ref()) == "joburi" => {
                        if in_job_uri && !uri.trim().is_empty() {
                            return Some(uri.trim().to_string());
                        }
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
    job_path(&path)
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
    // Simulated ids open only on request, so a real host named `sim` works.
    let simulate = crate::infrastructure::acquisition::simulate_backends();
    if crate::infrastructure::acquisition::opens_simulated_session(id, "escl:sim", simulate) {
        return Ok(simulated_session(id));
    }
    let endpoint =
        parse_escl_id(id).ok_or_else(|| ScanError::DeviceNotFound(format!("bad eSCL id: {id}")))?;
    if simulate && endpoint.host == "sim" {
        return Ok(EsclDeviceSession::new(id.into(), endpoint, true));
    }
    authorize_listed_or_allowed(id, &endpoint, cancellation)?;
    Ok(EsclDeviceSession::new(id.into(), endpoint, false))
}

fn validate_device_id(id: &str) -> Result<()> {
    if id.starts_with("escl:") {
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
    open_explicit_id_with(
        device_id,
        crate::infrastructure::acquisition::simulate_backends(),
    )
}

fn open_explicit_id_with(device_id: &str, simulate: bool) -> Result<EsclDeviceSession> {
    // Simulated ids open only on request, as in [`open`].
    if crate::infrastructure::acquisition::opens_simulated_session(device_id, "escl:sim", simulate)
    {
        return Ok(simulated_session(device_id));
    }
    if device_id == "escl:sim" {
        return Err(ScanError::DeviceNotFound(format!(
            "{device_id} is the simulated eSCL scanner; set OPEN_SCANLINE_SIMULATE_BACKENDS=1 to use it"
        )));
    }
    let endpoint = parse_escl_id(device_id)
        .ok_or_else(|| ScanError::DeviceNotFound(format!("bad eSCL id: {device_id}")))?;
    let simulated = simulate && endpoint.host == "sim";
    Ok(EsclDeviceSession::new(
        device_id.into(),
        endpoint,
        simulated,
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

#[cfg(test)]
mod tests {
    #[test]
    fn explicit_sim_id_needs_the_simulation_opt_in() {
        assert!(open_explicit_id_with("escl:sim", true).is_ok());
        let error = open_explicit_id_with("escl:sim", false)
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("OPEN_SCANLINE_SIMULATE_BACKENDS=1"),
            "{error}"
        );
    }

    use super::*;

    fn endpoint() -> Endpoint {
        parse_endpoint("http://printer.local:80").unwrap()
    }

    fn location(value: &str) -> String {
        format!("HTTP/1.1 201 Created\r\nLocation: {value}")
    }

    #[test]
    fn same_origin_location_is_used() {
        let headers = location("http://printer.local/eSCL/ScanJobs/abc");
        assert_eq!(
            canonical_job_path(&headers, b"", "eSCL", &endpoint()).as_deref(),
            Some("/eSCL/ScanJobs/abc")
        );
    }

    #[test]
    fn ip_location_for_a_hostname_endpoint_keeps_only_the_path() {
        let headers = location("http://192.168.1.50:80/eSCL/ScanJobs/abc/");
        assert_eq!(
            canonical_job_path(&headers, b"", "eSCL", &endpoint()).as_deref(),
            Some("/eSCL/ScanJobs/abc")
        );
    }

    #[test]
    fn foreign_host_is_never_followed() {
        // Only the pinned endpoint is ever contacted, so the result is a bare path.
        let headers = location("https://evil.example/eSCL/ScanJobs/abc");
        assert_eq!(
            canonical_job_path(&headers, b"", "eSCL", &endpoint()).as_deref(),
            Some("/eSCL/ScanJobs/abc")
        );
        // Paths that are not a plain job path are not kept.
        for bad in [
            "http://evil.example/other/ScanJobs/abc",
            "http://evil.example/eSCL/ScanJobs/abc/extra",
            "http://evil.example/eSCL/ScanJobs/..",
            "http://evil.example/eSCL/ScanJobs/%2e%2e",
            "http://evil.example/eSCL/ScanJobs/abc?x=1",
            "http://evil.example/",
        ] {
            assert_eq!(cross_origin_job_path(bad, "eSCL"), None, "{bad}");
        }
    }

    #[test]
    fn job_location_decodes_entities() {
        let body = b"<scan:JobUri>/eSCL/ScanJobs/a&amp;b</scan:JobUri>";
        assert_eq!(
            job_location("", body).as_deref(),
            Some("/eSCL/ScanJobs/a&b")
        );
    }

    #[test]
    fn odd_locations_are_rejected_on_every_path() {
        let same = endpoint();
        for bad in [
            "/eSCL/ScanJobs/",
            "/eSCL/ScanJobs/%2e%2e/x",
            "/eSCL/ScanJobs/%2e%2e",
            "/eSCL/ScanJobs/..",
            "foo/..",
        ] {
            assert_eq!(normalize_job_location(bad, &same), None, "{bad}");
            let headers = location(bad);
            assert_eq!(
                canonical_job_path(&headers, b"", "eSCL", &same),
                None,
                "{bad}"
            );
        }
        for bad in ["http://h/eSCL/ScanJobs/", "http://h/eSCL/ScanJobs/%2e%2e/x"] {
            assert_eq!(cross_origin_job_path(bad, "eSCL"), None, "{bad}");
        }
        assert_eq!(
            normalize_job_location("http://printer.local:80/eSCL/ScanJobs/uuid", &same).as_deref(),
            Some("/eSCL/ScanJobs/uuid")
        );
    }

    #[test]
    fn unusable_location_falls_back_to_body_job_id() {
        let headers = location("http://evil.example/elsewhere");
        let body = b"<scan:JobUri>/eSCL/ScanJobs/from-body</scan:JobUri>";
        assert_eq!(
            canonical_job_path(&headers, body, "eSCL", &endpoint()).as_deref(),
            Some("/eSCL/ScanJobs/from-body")
        );
        assert_eq!(
            canonical_job_path("HTTP/1.1 201 Created", body, "eSCL", &endpoint()).as_deref(),
            Some("/eSCL/ScanJobs/from-body")
        );
    }
}
