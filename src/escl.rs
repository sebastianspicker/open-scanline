//! eSCL / AirScan network backend — list/open/scan with HTTP CreateScanJob.

use crate::backend_process::{simulate_backends, TemporaryOutput};
use crate::core::{
    validate_scan_dpi, ImageBuffer, PixelFormat, Result, ScanError, ScanMode, ScanRequest,
};
use crate::device::{BackendInfo, CancellationToken, DeviceInfo};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::fs::File;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use ureq::config::Config;
use ureq::http::Uri;
use ureq::unversioned::resolver::{ArrayVec, DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, DefaultConnector, NextTimeout, Transport,
};

const ENDPOINT_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const DISCOVERY_PROBE_BUDGET: Duration = Duration::from_secs(5);
const MAX_PROBE_WORKERS: usize = 8;
const CAPABILITIES_RESPONSE_LIMIT: u64 = 2 * 1024 * 1024;
const JOB_RESPONSE_LIMIT: u64 = 2 * 1024 * 1024;
const MIN_DOCUMENT_RESPONSE_LIMIT: u64 = 2 * 1024 * 1024;
const DOCUMENT_CONTAINER_OVERHEAD: u64 = 16 * 1024 * 1024;
const MAX_DOCUMENT_RESPONSE_LIMIT: u64 =
    crate::core::MAX_IMAGE_BYTES as u64 + DOCUMENT_CONTAINER_OVERHEAD;
const MAX_DOCUMENT_DECODE_ALLOCATION: u64 = crate::core::MAX_IMAGE_BYTES as u64;
const MIN_DOCUMENT_DECODE_ALLOCATION: u64 = 4 * 1024 * 1024;
const MIN_DOCUMENT_DIMENSION_LIMIT: u32 = 1_024;
const MAX_DOCUMENT_DIMENSION_LIMIT: u32 = crate::core::MAX_IMAGE_DIMENSION;
const NEXT_DOCUMENT_DEADLINE: Duration = Duration::from_secs(60);
const NEXT_DOCUMENT_SETUP_TIMEOUT: Duration = Duration::from_secs(5);
const NEXT_DOCUMENT_RETRY_DELAY: Duration = Duration::from_millis(150);
const NEXT_DOCUMENT_CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);
const CONTROL_REQUEST_SETUP_TIMEOUT: Duration = Duration::from_secs(1);
const CANCEL_JOB_CLEANUP_TIMEOUT: Duration = Duration::from_millis(500);
static ESCL_DISCOVERY_LOCK: Mutex<()> = Mutex::new(());
static ESCL_DEVICE_CACHE: OnceLock<Mutex<Option<Vec<DeviceInfo>>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CapabilitySource {
    Platen,
    AdfSimplex,
    AdfDuplex,
    Film,
}

impl CapabilitySource {
    fn input_source(self) -> &'static str {
        match self {
            Self::Platen => "Platen",
            Self::AdfSimplex => "Feeder",
            Self::AdfDuplex => "Feeder",
            Self::Film => "Transparency",
        }
    }
}

#[derive(Debug, Clone, Default)]
struct SourceCapabilities {
    source: Option<CapabilitySource>,
    color_modes: Vec<String>,
    document_formats: Vec<String>,
    resolutions: Vec<(u32, u32)>,
    max_width: Option<u32>,
    max_height: Option<u32>,
}

#[derive(Debug, Clone, Default)]
struct ScannerCapabilities {
    make_and_model: Option<String>,
    root: String,
    sources: Vec<SourceCapabilities>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentRepresentation {
    BlackAndWhite1,
    Grayscale8,
    Rgb24,
    Rgb48,
}

impl DocumentRepresentation {
    fn decoded_bytes_per_pixel(self) -> u64 {
        match self {
            // The image decoder expands bilevel images to one byte per pixel.
            Self::BlackAndWhite1 | Self::Grayscale8 => 1,
            Self::Rgb24 => 3,
            Self::Rgb48 => 6,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::BlackAndWhite1 => "BlackAndWhite1",
            Self::Grayscale8 => "Grayscale8",
            Self::Rgb24 => "RGB24",
            Self::Rgb48 => "RGB48",
        }
    }
}

#[derive(Debug, Clone)]
struct NegotiatedColorMode {
    name: String,
    representation: DocumentRepresentation,
}

#[derive(Debug, Clone, Copy)]
struct DocumentLimits {
    response_limit: u64,
    max_width: u32,
    max_height: u32,
    max_allocation: u64,
}

impl DocumentLimits {
    fn for_request(request: &ScanRequest, representation: DocumentRepresentation) -> Result<Self> {
        let response_limit = document_response_limit(request, representation)?;
        let (max_width, max_height, max_allocation) =
            document_decode_limits(request, representation)?;
        Ok(Self {
            response_limit,
            max_width,
            max_height,
            max_allocation,
        })
    }
}

impl ScannerCapabilities {
    fn source(&self, source: CapabilitySource) -> Option<&SourceCapabilities> {
        self.sources
            .iter()
            .find(|capabilities| capabilities.source == Some(source))
    }

    fn source_or_default(&self, source: CapabilitySource) -> SourceCapabilities {
        let mut selected = self
            .source(source)
            .cloned()
            .unwrap_or_else(|| SourceCapabilities {
                source: Some(source),
                ..SourceCapabilities::default()
            });
        if let Some(shared) = self
            .sources
            .iter()
            .find(|capabilities| capabilities.source.is_none())
        {
            for color in &shared.color_modes {
                push_unique(&mut selected.color_modes, color);
            }
            for format in &shared.document_formats {
                push_unique(&mut selected.document_formats, format);
            }
            for resolution in &shared.resolutions {
                if !selected.resolutions.contains(resolution) {
                    selected.resolutions.push(*resolution);
                }
            }
            selected.max_width = selected.max_width.or(shared.max_width);
            selected.max_height = selected.max_height.or(shared.max_height);
        }
        selected
    }
}

mod job_id;
pub use job_id::parse_job_id;

#[cfg(test)]
mod tests;

/// Network discovery enabled unless OPEN_SCANLINE_NETWORK_DISCOVERY=0.
pub fn available() -> bool {
    !matches!(
        std::env::var("OPEN_SCANLINE_NETWORK_DISCOVERY"),
        Ok(v) if v.trim() == "0"
    )
}

pub fn backend_info() -> BackendInfo {
    BackendInfo {
        id: "escl".into(),
        name: "eSCL/AirScan network scanners".into(),
        available: available() || simulate_backends(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Endpoint {
    host: String,
    port: u16,
    secure: bool,
}

impl Endpoint {
    fn url(&self, path: &str) -> String {
        let scheme = if self.secure { "https" } else { "http" };
        let host = if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        format!("{scheme}://{host}:{}{path}", self.port)
    }

    fn device_id(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        if self.secure {
            format!("escl:https@{host}:{}", self.port)
        } else {
            format!("escl:{host}:{}", self.port)
        }
    }
}

/// Explicit scanner targets, separated by commas. Values may be hosts,
/// `host:port`, or `http(s)://host:port`.
fn env_hosts() -> Vec<Endpoint> {
    let Ok(raw) = std::env::var("OPEN_SCANLINE_ESCL_HOSTS") else {
        return Vec::new();
    };
    raw.split(',').filter_map(parse_endpoint).collect()
}

fn parse_endpoint(value: &str) -> Option<Endpoint> {
    if value.is_empty()
        || value
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return None;
    }
    let (secure, authority) = endpoint_authority(value);
    let (host, port) = endpoint_host_port(authority, secure)?;
    Some(Endpoint { host, port, secure })
}

fn endpoint_authority(value: &str) -> (bool, &str) {
    if let Some(rest) = value.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = value.strip_prefix("http://") {
        (false, rest)
    } else {
        (false, value)
    }
}

fn endpoint_host_port(authority: &str, secure: bool) -> Option<(String, u16)> {
    let default_port = if secure { 443 } else { 80 };
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, suffix) = rest.split_once(']')?;
        if host.is_empty() || suffix.contains(']') {
            return None;
        }
        let host = host.parse::<Ipv6Addr>().ok()?.to_string();
        let port = parse_endpoint_port(suffix, default_port)?;
        return Some((host, port));
    }
    if authority.contains(['/', '?', '#', '@', '[', ']', '\\']) {
        return None;
    }
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) => (
            host,
            parse_endpoint_port(&format!(":{port}"), default_port)?,
        ),
        None => (authority, default_port),
    };
    let host = normalize_host(host)?;
    Some((host, port))
}

fn parse_endpoint_port(suffix: &str, default_port: u16) -> Option<u16> {
    if suffix.is_empty() {
        return Some(default_port);
    }
    let port = suffix.strip_prefix(':')?;
    if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let port = port.parse::<u16>().ok()?;
    (port != 0).then_some(port)
}

fn normalize_host(host: &str) -> Option<String> {
    if host.is_empty() {
        return None;
    }
    if let Ok(address) = host.parse::<IpAddr>() {
        return match address {
            IpAddr::V4(address) => Some(address.to_string()),
            IpAddr::V6(_) => None,
        };
    }
    if host.contains('.')
        && host
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return host
            .parse::<Ipv4Addr>()
            .ok()
            .map(|address| address.to_string());
    }
    (host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        }))
    .then(|| host.to_ascii_lowercase())
}

#[cfg(test)]
fn http_exchange(
    endpoint: &Endpoint,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
    content_type: Option<&str>,
    timeout: Duration,
    response_limit: u64,
) -> Option<(u16, Vec<u8>, String)> {
    http_exchange_with_cancellation(
        endpoint,
        method,
        path,
        HttpExchangeOptions {
            body,
            content_type,
            global_timeout: timeout,
            setup_timeout: timeout,
            response_header_timeout: timeout,
            response_limit,
            cancellation: None,
        },
    )
}

struct HttpExchangeOptions<'a> {
    body: Option<&'a [u8]>,
    content_type: Option<&'a str>,
    global_timeout: Duration,
    setup_timeout: Duration,
    response_header_timeout: Duration,
    response_limit: u64,
    cancellation: Option<CancellationToken>,
}

#[cfg(test)]
fn http_exchange_with_cancellation(
    endpoint: &Endpoint,
    method: &str,
    path: &str,
    options: HttpExchangeOptions<'_>,
) -> Option<(u16, Vec<u8>, String)> {
    http_exchange_with_cancellation_result(endpoint, method, path, options).ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HttpExchangeError {
    Cancelled,
    Failed,
}

fn map_http_exchange_error(error: &ureq::Error) -> HttpExchangeError {
    match error {
        ureq::Error::Io(error) if error.kind() == std::io::ErrorKind::Interrupted => {
            HttpExchangeError::Cancelled
        }
        _ => HttpExchangeError::Failed,
    }
}

fn http_exchange_with_cancellation_result(
    endpoint: &Endpoint,
    method: &str,
    path: &str,
    options: HttpExchangeOptions<'_>,
) -> std::result::Result<(u16, Vec<u8>, String), HttpExchangeError> {
    let HttpExchangeOptions {
        body,
        content_type,
        global_timeout,
        setup_timeout,
        response_header_timeout,
        response_limit,
        cancellation,
    } = options;
    let agent = scanner_agent_with_phase_timeout(
        global_timeout,
        setup_timeout,
        response_header_timeout,
        cancellation,
    );
    let url = endpoint.url(path);
    let response = match (method, body) {
        ("GET", None) => agent
            .get(&url)
            .call()
            .map_err(|error| map_http_exchange_error(&error))?,
        ("POST", Some(bytes)) => {
            let request = agent.post(&url).header(
                "Content-Type",
                content_type.unwrap_or("application/octet-stream"),
            );
            request
                .send(bytes)
                .map_err(|error| map_http_exchange_error(&error))?
        }
        ("DELETE", None) => agent
            .delete(&url)
            .call()
            .map_err(|error| map_http_exchange_error(&error))?,
        _ => return Err(HttpExchangeError::Failed),
    };
    let (parts, body) = response.into_parts();
    if parts.status.is_redirection() {
        return Err(HttpExchangeError::Failed);
    }
    let headers = parts
        .headers
        .iter()
        .filter_map(|(name, value)| value.to_str().ok().map(|v| format!("{name}: {v}")))
        .collect::<Vec<_>>()
        .join("\r\n");
    let bytes = body
        .into_with_config()
        .limit(response_limit)
        .read_to_vec()
        .map_err(|error| map_http_exchange_error(&error))?;
    Ok((parts.status.as_u16(), bytes, headers))
}

#[derive(Debug, Default)]
struct MdnsLocalResolver {
    default: DefaultResolver,
}

impl Resolver for MdnsLocalResolver {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> std::result::Result<ResolvedSocketAddrs, ureq::Error> {
        let addresses = self.default.resolve(uri, config, timeout)?;
        filter_mdns_resolved_addresses(uri, addresses)
    }
}

fn filter_mdns_resolved_addresses(
    uri: &Uri,
    addresses: ResolvedSocketAddrs,
) -> std::result::Result<ResolvedSocketAddrs, ureq::Error> {
    let Some(host) = uri.host() else {
        return Ok(addresses);
    };
    if !host.to_ascii_lowercase().ends_with(".local") {
        return Ok(addresses);
    }

    let mut local_addresses = ArrayVec::from_fn(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
    for &address in &addresses {
        if mdns_address_is_local(address.ip()) {
            local_addresses.push(address);
        }
    }
    if local_addresses.is_empty() {
        Err(ureq::Error::HostNotFound)
    } else {
        Ok(local_addresses)
    }
}

fn scanner_agent_with_phase_timeout(
    global_timeout: Duration,
    setup_timeout: Duration,
    response_header_timeout: Duration,
    cancellation: Option<CancellationToken>,
) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(global_timeout))
        .timeout_resolve(Some(setup_timeout))
        .timeout_connect(Some(setup_timeout))
        // Sending is bounded by CancellableTransport. Leaving ureq's send
        // phase unconfigured prevents that short cap from becoming a
        // preceding RecvResponse deadline.
        .timeout_send_request(None)
        .timeout_send_body(None)
        // `RecvBody` also considers `RecvResponse` as a preceding deadline.
        // Keep both transport phases on the total allowance; the transport
        // wrapper below applies the independently configured response-header
        // allowance.
        .timeout_recv_response(Some(global_timeout))
        .timeout_recv_body(Some(global_timeout))
        .http_status_as_error(false)
        .max_redirects(0)
        .proxy(None)
        .build();
    ureq::Agent::with_parts(
        config,
        CancellationConnector {
            inner: DefaultConnector::new(),
            cancellation,
            send_timeout: setup_timeout,
            response_header_timeout,
        },
        MdnsLocalResolver::default(),
    )
}

#[derive(Debug)]
struct CancellationConnector {
    inner: DefaultConnector,
    cancellation: Option<CancellationToken>,
    send_timeout: Duration,
    response_header_timeout: Duration,
}

impl Connector for CancellationConnector {
    type Out = CancellableTransport;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<()>,
    ) -> std::result::Result<Option<Self::Out>, ureq::Error> {
        self.inner.connect(details, chained).map(|transport| {
            transport.map(|inner| CancellableTransport {
                inner,
                cancellation: self.cancellation.clone(),
                send_timeout: self.send_timeout,
                response_header_timeout: self.response_header_timeout,
                response_setup_deadline: None,
                send_deadline: None,
                send_phase_active: false,
            })
        })
    }
}

struct CancellableTransport {
    inner: Box<dyn Transport>,
    cancellation: Option<CancellationToken>,
    send_timeout: Duration,
    response_header_timeout: Duration,
    response_setup_deadline: Option<Instant>,
    send_deadline: Option<Instant>,
    send_phase_active: bool,
}

impl std::fmt::Debug for CancellableTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("CancellableTransport").finish()
    }
}

impl Transport for CancellableTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }

    fn transmit_output(
        &mut self,
        amount: usize,
        timeout: NextTimeout,
    ) -> std::result::Result<(), ureq::Error> {
        {
            // With ureq's built-in send phase deadline deliberately disabled
            // (so it cannot constrain RecvResponse), this arrives as the
            // global timeout. Every output write is consequently an HTTP send
            // phase from this wrapper's perspective.
            // A transport can be reused from the pool after a response without
            // a body. Treat the next request write as a new header phase.
            if !self.send_phase_active {
                self.response_setup_deadline = None;
                self.send_phase_active = true;
            }
            if self
                .cancellation
                .as_ref()
                .is_some_and(CancellationToken::is_cancelled)
            {
                return Err(ureq::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "eSCL request cancelled",
                )));
            }
            let now = Instant::now();
            let configured_deadline = now.checked_add(*timeout.after).unwrap_or(now);
            let send_deadline = *self.send_deadline.get_or_insert_with(|| {
                now.checked_add(self.send_timeout)
                    .unwrap_or(configured_deadline)
            });
            let remaining = configured_deadline
                .min(send_deadline)
                .saturating_duration_since(now);
            if remaining.is_zero() {
                return Err(ureq::Error::Timeout(timeout.reason));
            }
            self.inner.transmit_output(
                amount,
                NextTimeout {
                    after: remaining.into(),
                    reason: timeout.reason,
                },
            )
        }
    }

    fn await_input(&mut self, timeout: NextTimeout) -> std::result::Result<bool, ureq::Error> {
        let now = Instant::now();
        self.send_deadline = None;
        let deadline = now.checked_add(*timeout.after).unwrap_or(now);
        // ureq can report the global timeout as the reason when its global
        // and RecvResponse deadlines are equal. Track the request transition
        // ourselves so the response-start allowance remains independent from
        // the longer header/body deadline.
        if self.send_phase_active {
            self.response_setup_deadline.get_or_insert_with(|| {
                now.checked_add(self.response_header_timeout).unwrap_or(now)
            });
        }
        loop {
            if self
                .cancellation
                .as_ref()
                .is_some_and(CancellationToken::is_cancelled)
            {
                return Err(ureq::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "eSCL request cancelled",
                )));
            }
            let phase_deadline = self
                .response_setup_deadline
                .map_or(deadline, |header| deadline.min(header));
            let remaining = phase_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ureq::Error::Timeout(timeout.reason));
            }
            let poll_timeout = NextTimeout {
                after: remaining
                    .min(NEXT_DOCUMENT_CANCELLATION_POLL_INTERVAL)
                    .into(),
                reason: timeout.reason,
            };
            match self.inner.await_input(poll_timeout) {
                Ok(progress) => {
                    self.send_phase_active = false;
                    self.response_setup_deadline = None;
                    return Ok(progress);
                }
                Err(ureq::Error::Timeout(_)) => continue,
                Err(error) => return Err(error),
            }
        }
    }

    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }

    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}

#[cfg(test)]
fn http_get_to_temporary_output(
    endpoint: &Endpoint,
    path: &str,
    timeout: Duration,
    response_limit: u64,
) -> std::result::Result<(u16, Option<TemporaryOutput>), FetchDocumentError> {
    http_get_to_temporary_output_with_phase_timeout(
        endpoint,
        path,
        timeout,
        timeout,
        response_limit,
    )
}

#[cfg(test)]
fn http_get_to_temporary_output_with_phase_timeout(
    endpoint: &Endpoint,
    path: &str,
    global_timeout: Duration,
    setup_timeout: Duration,
    response_limit: u64,
) -> std::result::Result<(u16, Option<TemporaryOutput>), FetchDocumentError> {
    http_get_to_temporary_output_with_phase_timeout_and_cancellation(
        endpoint,
        path,
        global_timeout,
        setup_timeout,
        response_limit,
        None,
    )
}

fn http_get_to_temporary_output_with_phase_timeout_and_cancellation(
    endpoint: &Endpoint,
    path: &str,
    global_timeout: Duration,
    setup_timeout: Duration,
    response_limit: u64,
    cancellation: Option<&CancellationToken>,
) -> std::result::Result<(u16, Option<TemporaryOutput>), FetchDocumentError> {
    let response = scanner_agent_with_phase_timeout(
        global_timeout,
        setup_timeout,
        setup_timeout,
        cancellation.cloned(),
    )
    .get(&endpoint.url(path))
    .call()
    .map_err(|error| match error {
        ureq::Error::Io(error) if error.kind() == std::io::ErrorKind::Interrupted => {
            FetchDocumentError::Cancelled
        }
        ureq::Error::Timeout(_) => FetchDocumentError::RetryableTimeout,
        _ => FetchDocumentError::Failed(format!("NextDocument no response: {error}")),
    })?;
    let (parts, body) = response.into_parts();
    let status = parts.status.as_u16();
    if parts.status.is_redirection() {
        return Err(FetchDocumentError::Failed(
            "NextDocument redirect rejected".into(),
        ));
    }
    if status != 200 {
        return Ok((status, None));
    }
    if parts
        .headers
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > response_limit)
    {
        return Err(FetchDocumentError::Failed(
            "NextDocument response exceeds byte limit".into(),
        ));
    }
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

fn requested_document_dimensions(request: &ScanRequest) -> (u32, u32) {
    request
        .region
        .map(|region| (region.width, region.height))
        .unwrap_or((request.width, request.height))
}

fn requested_document_pixels(request: &ScanRequest) -> Result<u64> {
    let (width, height) = requested_document_dimensions(request);
    crate::core::checked_image_len(width, height, request.pixel_format.bpp())?;
    u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| ScanError::Invalid("eSCL document dimensions overflow".into()))
}

fn document_decoded_bytes(
    request: &ScanRequest,
    representation: DocumentRepresentation,
) -> Result<u64> {
    let bytes = requested_document_pixels(request)?
        .checked_mul(representation.decoded_bytes_per_pixel())
        .ok_or_else(|| ScanError::Invalid("eSCL document dimensions overflow".into()))?;
    if bytes > MAX_DOCUMENT_DECODE_ALLOCATION {
        return Err(ScanError::Invalid(format!(
            "eSCL {} document exceeds the {MAX_DOCUMENT_DECODE_ALLOCATION}-byte decoded image safety limit",
            representation.name()
        )));
    }
    Ok(bytes)
}

fn document_response_limit(
    request: &ScanRequest,
    representation: DocumentRepresentation,
) -> Result<u64> {
    let response_limit = document_decoded_bytes(request, representation)?
        .checked_add(DOCUMENT_CONTAINER_OVERHEAD)
        .ok_or_else(|| ScanError::Invalid("eSCL document response limit overflow".into()))?;
    if response_limit > MAX_DOCUMENT_RESPONSE_LIMIT {
        return Err(ScanError::Invalid(format!(
            "eSCL {} document exceeds the {MAX_DOCUMENT_RESPONSE_LIMIT}-byte response safety limit",
            representation.name()
        )));
    }
    Ok(response_limit.max(MIN_DOCUMENT_RESPONSE_LIMIT))
}

fn document_decode_limits(
    request: &ScanRequest,
    representation: DocumentRepresentation,
) -> Result<(u32, u32, u64)> {
    let (width, height) = requested_document_dimensions(request);
    let max_width = width
        .saturating_mul(4)
        .clamp(MIN_DOCUMENT_DIMENSION_LIMIT, MAX_DOCUMENT_DIMENSION_LIMIT);
    let max_height = height
        .saturating_mul(4)
        .clamp(MIN_DOCUMENT_DIMENSION_LIMIT, MAX_DOCUMENT_DIMENSION_LIMIT);
    let max_allocation =
        document_decoded_bytes(request, representation)?.max(MIN_DOCUMENT_DECODE_ALLOCATION);
    Ok((max_width, max_height, max_allocation))
}

fn local_name(name: &[u8]) -> String {
    let name = std::str::from_utf8(name).unwrap_or_default();
    name.rsplit(':')
        .next()
        .unwrap_or(name)
        .trim_matches(|character| character == '{' || character == '}')
        .to_ascii_lowercase()
}

fn source_for_path(path: &[String]) -> Option<CapabilitySource> {
    let path = path.join("/");
    if path.contains("adfduplex") || path.contains("duplexadf") {
        Some(CapabilitySource::AdfDuplex)
    } else if path.contains("adfsimplex")
        || path.contains("simplexadf")
        || path.contains("feeder")
        || path.contains("adf")
    {
        Some(CapabilitySource::AdfSimplex)
    } else if path.contains("film") || path.contains("transparen") {
        Some(CapabilitySource::Film)
    } else if path.contains("platen") || path.contains("flatbed") {
        Some(CapabilitySource::Platen)
    } else {
        None
    }
}

fn source_capabilities_mut(
    capabilities: &mut ScannerCapabilities,
    source: Option<CapabilitySource>,
) -> &mut SourceCapabilities {
    if let Some(index) = capabilities
        .sources
        .iter()
        .position(|candidate| candidate.source == source)
    {
        return &mut capabilities.sources[index];
    }
    capabilities.sources.push(SourceCapabilities {
        source,
        ..SourceCapabilities::default()
    });
    capabilities.sources.last_mut().expect("source inserted")
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if !value.is_empty()
        && !values
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(value))
    {
        values.push(value.to_string());
    }
}

fn parse_escl_capabilities(body: &[u8], root: &str) -> Option<ScannerCapabilities> {
    let mut reader = Reader::from_reader(body);
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut path = Vec::<String>::new();
    let mut values = Vec::<String>::new();
    let mut capabilities = ScannerCapabilities {
        root: root.to_string(),
        ..ScannerCapabilities::default()
    };
    let mut current_resolution: Option<(Option<u32>, Option<u32>)> = None;
    let mut saw_capabilities = false;

    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => {
                let name = local_name(event.name().as_ref());
                saw_capabilities |= name == "scannercapabilities";
                if name == "resolution" {
                    current_resolution = Some((None, None));
                }
                path.push(name);
                values.push(String::new());
            }
            Ok(Event::Text(event)) => {
                if let Some(value) = values.last_mut() {
                    *value = event.decode().ok()?.into_owned();
                }
            }
            Ok(Event::CData(event)) => {
                if let Some(value) = values.last_mut() {
                    *value = String::from_utf8_lossy(event.as_ref()).into_owned();
                }
            }
            Ok(Event::End(event)) => {
                let name = local_name(event.name().as_ref());
                let value = values.pop().unwrap_or_default();
                let source = source_for_path(&path);
                let source_capabilities = source_capabilities_mut(&mut capabilities, source);
                match name.as_str() {
                    "makeandmodel" => {
                        capabilities.make_and_model = (!value.is_empty()).then_some(value)
                    }
                    "colormode" => push_unique(&mut source_capabilities.color_modes, &value),
                    "documentformat" | "documentformatext" => {
                        push_unique(&mut source_capabilities.document_formats, &value)
                    }
                    "inputsource" => {
                        let value = value.to_ascii_lowercase();
                        let source = if value.contains("platen") || value.contains("flatbed") {
                            Some(CapabilitySource::Platen)
                        } else if value.contains("duplex") {
                            Some(CapabilitySource::AdfDuplex)
                        } else if value.contains("adf") || value.contains("feed") {
                            Some(CapabilitySource::AdfSimplex)
                        } else if value.contains("film") || value.contains("transparen") {
                            Some(CapabilitySource::Film)
                        } else {
                            None
                        };
                        if let Some(source) = source {
                            let _ = source_capabilities;
                            let _ = source_capabilities_mut(&mut capabilities, Some(source));
                        }
                    }
                    "maxwidth" | "maxscanwidth" => {
                        source_capabilities.max_width = value.trim().parse().ok()
                    }
                    "maxheight" | "maxscanheight" => {
                        source_capabilities.max_height = value.trim().parse().ok()
                    }
                    "xresolution" => {
                        if let Some((x, _)) = current_resolution.as_mut() {
                            *x = value.trim().parse().ok();
                        }
                    }
                    "yresolution" => {
                        if let Some((_, y)) = current_resolution.as_mut() {
                            *y = value.trim().parse().ok();
                        }
                    }
                    "resolution" => {
                        if let Some((x, y)) = current_resolution.take() {
                            if let (Some(x), Some(y)) = (x, y) {
                                source_capabilities.resolutions.push((x, y));
                            }
                        }
                    }
                    _ => {}
                }
                path.pop();
            }
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
        buffer.clear();
    }
    saw_capabilities.then_some(capabilities)
}

fn capabilities_for_endpoint_with_cancellation(
    endpoint: &Endpoint,
    deadline: Instant,
    cancellation: Option<CancellationToken>,
) -> Option<ScannerCapabilities> {
    capabilities_for_endpoint_with_cancellation_result(endpoint, deadline, cancellation)
        .ok()
        .flatten()
}

fn capabilities_for_endpoint_with_cancellation_result(
    endpoint: &Endpoint,
    deadline: Instant,
    cancellation: Option<CancellationToken>,
) -> std::result::Result<Option<ScannerCapabilities>, HttpExchangeError> {
    for (path, root) in [
        ("/eSCL/ScannerCapabilities", "eSCL"),
        ("/Scan/ScannerCapabilities", "Scan"),
    ] {
        if cancellation
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(HttpExchangeError::Cancelled);
        }
        let Some(timeout) = deadline.checked_duration_since(Instant::now()) else {
            return Ok(None);
        };
        if timeout.is_zero() {
            return Ok(None);
        }
        let response = http_exchange_with_cancellation_result(
            endpoint,
            "GET",
            path,
            HttpExchangeOptions {
                body: None,
                content_type: None,
                global_timeout: timeout,
                setup_timeout: timeout.min(CONTROL_REQUEST_SETUP_TIMEOUT),
                response_header_timeout: timeout,
                response_limit: CAPABILITIES_RESPONSE_LIMIT,
                cancellation: cancellation.clone(),
            },
        );
        let (status, body, _) = match response {
            Ok(response) => response,
            Err(HttpExchangeError::Cancelled) => return Err(HttpExchangeError::Cancelled),
            Err(HttpExchangeError::Failed) => continue,
        };
        if status == 200 {
            if let Some(capabilities) = parse_escl_capabilities(&body, root) {
                return Ok(Some(capabilities));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
fn probe_endpoint(endpoint: &Endpoint) -> Option<DeviceInfo> {
    probe_endpoint_until(endpoint, Instant::now() + ENDPOINT_PROBE_TIMEOUT)
}

#[cfg(test)]
fn probe_endpoint_until(endpoint: &Endpoint, deadline: Instant) -> Option<DeviceInfo> {
    probe_endpoint_until_with_cancellation(endpoint, deadline, None)
}

fn probe_endpoint_until_with_cancellation(
    endpoint: &Endpoint,
    deadline: Instant,
    cancellation: Option<CancellationToken>,
) -> Option<DeviceInfo> {
    let capabilities =
        capabilities_for_endpoint_with_cancellation(endpoint, deadline, cancellation)?;
    let name = capabilities
        .make_and_model
        .unwrap_or_else(|| format!("eSCL scanner @ {}:{}", endpoint.host, endpoint.port));
    Some(DeviceInfo::new(endpoint.device_id(), name, "escl"))
}

#[cfg(test)]
fn probe_endpoints(endpoints: &[Endpoint], budget: Duration) -> Vec<DeviceInfo> {
    probe_endpoints_with_cancellation(endpoints, budget, None)
}

fn probe_endpoints_with_cancellation(
    endpoints: &[Endpoint],
    budget: Duration,
    cancellation: Option<&CancellationToken>,
) -> Vec<DeviceInfo> {
    if endpoints.is_empty() {
        return Vec::new();
    }
    let now = Instant::now();
    let deadline = now.checked_add(budget).unwrap_or(now);
    let endpoints = Arc::new(endpoints.to_vec());
    let next = Arc::new(AtomicUsize::new(0));
    let found = Arc::new(Mutex::new(Vec::<(usize, DeviceInfo)>::new()));
    let workers = endpoints.len().min(MAX_PROBE_WORKERS);
    let mut handles = Vec::with_capacity(workers);
    for _ in 0..workers {
        let endpoints = Arc::clone(&endpoints);
        let next = Arc::clone(&next);
        let found = Arc::clone(&found);
        let cancellation = cancellation.cloned();
        handles.push(std::thread::spawn(move || loop {
            if Instant::now() >= deadline
                || cancellation
                    .as_ref()
                    .is_some_and(CancellationToken::is_cancelled)
            {
                break;
            }
            let index = next.fetch_add(1, Ordering::Relaxed);
            let Some(endpoint) = endpoints.get(index) else {
                break;
            };
            let now = Instant::now();
            let endpoint_deadline =
                deadline.min(now.checked_add(ENDPOINT_PROBE_TIMEOUT).unwrap_or(now));
            let device = std::panic::catch_unwind(|| {
                probe_endpoint_until_with_cancellation(
                    endpoint,
                    endpoint_deadline,
                    cancellation.clone(),
                )
            })
            .ok()
            .flatten();
            if let Some(device) = device {
                found
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push((index, device));
            }
        }));
    }
    for handle in handles {
        let _ = handle.join();
    }
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Vec::new();
    }
    let mut found = found.lock().unwrap_or_else(|error| error.into_inner());
    found.sort_by_key(|(index, _)| *index);
    found.drain(..).map(|(_, device)| device).collect()
}

fn mdns_endpoints_with_cancellation(
    wait: Duration,
    cancellation: Option<&CancellationToken>,
) -> Vec<Endpoint> {
    let Ok(daemon) = ServiceDaemon::new() else {
        return Vec::new();
    };
    let mut endpoints = Vec::new();
    for (service, secure_service) in [
        ("_uscan._tcp.local.", false),
        ("_uscans._tcp.local.", true),
        ("_scanner._tcp.local.", false),
    ] {
        let Ok(receiver) = daemon.browse(service) else {
            continue;
        };
        endpoints.extend(resolved_mdns_endpoints_with_cancellation(
            &receiver,
            wait,
            secure_service,
            cancellation,
        ));
        let _ = daemon.stop_browse(service);
    }
    if let Ok(shutdown) = daemon.shutdown() {
        let _ = shutdown.recv_timeout(Duration::from_secs(1));
    }
    endpoints
}

fn resolved_mdns_endpoints_with_cancellation(
    receiver: &mdns_sd::Receiver<ServiceEvent>,
    wait: Duration,
    secure_service: bool,
    cancellation: Option<&CancellationToken>,
) -> Vec<Endpoint> {
    let deadline = std::time::Instant::now() + wait;
    let mut endpoints = Vec::new();
    while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
        if cancellation.is_some_and(CancellationToken::is_cancelled) {
            break;
        }
        let poll = remaining.min(Duration::from_millis(25));
        let Ok(event) = receiver.recv_timeout(poll) else {
            if cancellation.is_some_and(CancellationToken::is_cancelled) {
                break;
            }
            continue;
        };
        if cancellation.is_some_and(CancellationToken::is_cancelled) {
            break;
        }
        if let ServiceEvent::ServiceResolved(info) = event {
            endpoints.extend(endpoints_for_service(&info, secure_service));
        }
    }
    endpoints
}

fn endpoints_for_service(info: &mdns_sd::ServiceInfo, secure_service: bool) -> Vec<Endpoint> {
    if !info
        .get_addresses()
        .iter()
        .any(|address| mdns_address_is_local(*address))
    {
        return Vec::new();
    }
    let secure = secure_service
        || info.get_port() == 443
        || matches!(info.get_property_val_str("https"), Some("1") | Some("true"));
    let service_host = info.get_hostname().trim_end_matches('.');
    let host = normalize_host(service_host)
        .filter(|host| host.ends_with(".local"))
        .or_else(|| {
            // An IP-literal service name is safe only because the resolved
            // addresses above were already restricted to local unicast.
            service_host
                .parse::<IpAddr>()
                .ok()
                .filter(|address| mdns_address_is_local(*address))
                .map(|address| address.to_string())
        });
    host.into_iter()
        .map(|host| Endpoint {
            host,
            port: info.get_port(),
            secure,
        })
        .collect()
}

fn mdns_address_is_local(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => ipv4_address_is_local(address),
        IpAddr::V6(address) => address.is_unique_local() || address.is_unicast_link_local(),
    }
}

fn ipv4_address_is_local(address: std::net::Ipv4Addr) -> bool {
    address.is_private() || address.is_link_local()
}

/// Returns a capped set of targets from `OPEN_SCANLINE_ESCL_SUBNETS`. A value may
/// be a host prefix (`192.168.1`) or an IPv4 CIDR. We intentionally never infer
/// a subnet or probe more than 64 addresses: discovery stays local and bounded.
fn subnet_endpoints(limit: usize) -> Vec<Endpoint> {
    let Ok(raw) = std::env::var("OPEN_SCANLINE_ESCL_SUBNETS") else {
        return Vec::new();
    };
    let cap = limit.min(64);
    raw.split(',')
        .flat_map(|entry| subnet_candidates(entry.trim(), cap))
        .take(cap)
        .collect()
}

fn subnet_candidates(value: &str, cap: usize) -> Vec<Endpoint> {
    let (address, prefix) = value.split_once('/').unwrap_or((value, "24"));
    let Ok(prefix) = prefix.parse::<u8>() else {
        return Vec::new();
    };
    let Ok(ip) = address.parse::<std::net::Ipv4Addr>() else {
        let parts: Vec<_> = address.trim_end_matches('.').split('.').collect();
        if parts.len() != 3 || parts.iter().any(|part| part.parse::<u8>().is_err()) {
            return Vec::new();
        }
        return (1..=cap.min(254))
            .filter_map(|last| {
                let candidate = format!("{}.{}.{}.{}", parts[0], parts[1], parts[2], last)
                    .parse::<std::net::Ipv4Addr>()
                    .ok()?;
                ipv4_address_is_local(candidate).then(|| Endpoint {
                    host: candidate.to_string(),
                    port: 80,
                    secure: false,
                })
            })
            .collect();
    };
    if !(16..=30).contains(&prefix) {
        return Vec::new();
    }
    let raw_ip = u32::from(ip);
    let mask = u32::MAX << (32 - prefix);
    let first = (raw_ip & mask).saturating_add(1);
    let last = (raw_ip | !mask).saturating_sub(1);
    (first..=last)
        .filter_map(|candidate| {
            let candidate = std::net::Ipv4Addr::from(candidate);
            ipv4_address_is_local(candidate).then(|| Endpoint {
                host: candidate.to_string(),
                port: 80,
                secure: false,
            })
        })
        .take(cap)
        .collect()
}

fn discover_devices_locked(max_hosts: usize) -> Vec<DeviceInfo> {
    discover_devices_locked_with_cancellation(max_hosts, None)
}

fn discover_devices_locked_with_cancellation(
    max_hosts: usize,
    cancellation: Option<&CancellationToken>,
) -> Vec<DeviceInfo> {
    if !available() && !simulate_backends() {
        return Vec::new();
    }
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Vec::new();
    }
    let cap = max_hosts.clamp(1, 64);
    let mut endpoints = env_hosts();
    endpoints.extend(mdns_endpoints_with_cancellation(
        Duration::from_millis(350),
        cancellation,
    ));
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Vec::new();
    }
    endpoints.extend(subnet_endpoints(cap.saturating_sub(endpoints.len())));
    endpoints.truncate(cap);
    let mut devices =
        probe_endpoints_with_cancellation(&endpoints, DISCOVERY_PROBE_BUDGET, cancellation);
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Vec::new();
    }
    if simulate_backends() && !devices.iter().any(|d| d.id == "escl:sim") {
        devices.push(DeviceInfo::new(
            "escl:sim",
            "eSCL Simulated Scanner",
            "escl",
        ));
    }
    devices
}

pub fn discover_devices(max_hosts: usize) -> Vec<DeviceInfo> {
    // A discovery daemon owns one socket per eligible network interface.
    // Serialize full discovery so concurrent UI/host queries cannot multiply
    // those sockets or competing probe pools past the process descriptor cap.
    let _discovery = ESCL_DISCOVERY_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    discover_devices_locked(max_hosts)
}

pub fn list_devices() -> Vec<DeviceInfo> {
    let _discovery = ESCL_DISCOVERY_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut cached = ESCL_DEVICE_CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(devices) = cached.as_ref() {
        return devices.clone();
    }
    let devices = std::panic::catch_unwind(|| discover_devices_locked(32)).unwrap_or_default();
    *cached = Some(devices);
    cached.as_ref().cloned().unwrap_or_default()
}

/// Perform one fresh bounded discovery and replace the process cache.
pub fn refresh_devices() -> Vec<DeviceInfo> {
    let _discovery = ESCL_DISCOVERY_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let devices = std::panic::catch_unwind(|| discover_devices_locked(32)).unwrap_or_default();
    let mut cached = ESCL_DEVICE_CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    *cached = Some(devices.clone());
    devices
}

pub fn list_escl_devices_safe() -> Vec<DeviceInfo> {
    list_devices()
}

/// eSCL discovery that observes an operation-wide cancellation signal before
/// cold discovery, while preserving the ordinary cached-listing behavior.
pub fn list_escl_devices_safe_with_cancellation(
    cancellation: Option<&CancellationToken>,
) -> Vec<DeviceInfo> {
    let Some(cancellation) = cancellation else {
        return list_escl_devices_safe();
    };
    if cancellation.is_cancelled() {
        return Vec::new();
    }
    let _discovery = ESCL_DISCOVERY_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if cancellation.is_cancelled() {
        return Vec::new();
    }
    let cached = ESCL_DEVICE_CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .cloned();
    if let Some(devices) = cached {
        return devices;
    }
    std::panic::catch_unwind(|| discover_devices_locked_with_cancellation(32, Some(cancellation)))
        .unwrap_or_default()
}

pub struct EsclDeviceSession {
    pub device_id: String,
    endpoint: Endpoint,
    simulate: bool,
    closed: Mutex<bool>,
    cancelled: Mutex<bool>,
    cancellation: Mutex<Option<CancellationToken>>,
}

impl EsclDeviceSession {
    fn new(device_id: String, endpoint: Endpoint, simulate: bool) -> Self {
        Self {
            device_id,
            endpoint,
            simulate,
            closed: Mutex::new(false),
            cancelled: Mutex::new(false),
            cancellation: Mutex::new(Some(CancellationToken::new())),
        }
    }

    fn is_closed(&self) -> bool {
        *self.closed.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn is_cancelled(&self) -> bool {
        *self.cancelled.lock().unwrap_or_else(|e| e.into_inner())
            || self
                .cancellation
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .is_some_and(CancellationToken::is_cancelled)
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .unwrap_or_default()
    }

    fn build_settings(
        &self,
        request: &ScanRequest,
        capabilities: &ScannerCapabilities,
    ) -> Result<(Vec<u8>, DocumentRepresentation)> {
        validate_scan_dpi(request.dpi_x, request.dpi_y)?;
        let source = select_source(request, capabilities)?;
        let color = select_color_mode(request, &source)?;
        let representation = color.representation;
        let color = color.name;
        let document_format = select_document_format(&source)?;
        let selected_source = source.source.expect("selected source");
        let duplex_setting = match selected_source {
            CapabilitySource::AdfSimplex => "  <scan:Duplex>false</scan:Duplex>\n",
            CapabilitySource::AdfDuplex => "  <scan:Duplex>true</scan:Duplex>\n",
            CapabilitySource::Platen | CapabilitySource::Film => "",
        };
        let (dpi_x, dpi_y) = select_resolution(request, &source);
        // ScanRequest coordinates are pixels at the requested DPI. The scanner
        // may acquire at a different advertised DPI and is resized below to the
        // requested output dimensions.
        let (x, y, width, height) =
            region_in_three_hundredths(request, request.dpi_x, request.dpi_y, &source);
        Ok((format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<scan:ScanSettings xmlns:scan="http://schemas.hp.com/imaging/escl/2011/05/03" xmlns:pwg="http://www.pwg.org/schemas/2010/12/sm" xmlns:escl="http://schemas.hp.com/imaging/escl/2011/05/03">
  <pwg:Version>2.0</pwg:Version>
  <scan:Intent>Document</scan:Intent>
  <pwg:ScanRegions>
    <pwg:ScanRegion>
      <pwg:ContentRegionUnits>escl:ThreeHundredthsOfInches</pwg:ContentRegionUnits>
      <pwg:XOffset>{x}</pwg:XOffset>
      <pwg:YOffset>{y}</pwg:YOffset>
      <pwg:Width>{width}</pwg:Width>
      <pwg:Height>{height}</pwg:Height>
    </pwg:ScanRegion>
  </pwg:ScanRegions>
  <pwg:InputSource>{input_source}</pwg:InputSource>
{duplex_setting}  <scan:XResolution>{dpi_x}</scan:XResolution>
  <scan:YResolution>{dpi_y}</scan:YResolution>
  <scan:ColorMode>{color}</scan:ColorMode>
  <pwg:DocumentFormat>{document_format}</pwg:DocumentFormat>
</scan:ScanSettings>
"#,
            input_source = selected_source.input_source(),
            color = color,
        )
        .into_bytes(), representation))
    }

    fn create_job(&self, root: &str, settings: &[u8]) -> Result<ScanJob> {
        let path = format!("/{root}/ScanJobs");
        let cancellation = self.cancellation_token();
        let (status, body, headers) = http_exchange_with_cancellation_result(
            &self.endpoint,
            "POST",
            &path,
            HttpExchangeOptions {
                body: Some(settings),
                content_type: Some("text/xml"),
                global_timeout: Duration::from_secs(15),
                setup_timeout: CONTROL_REQUEST_SETUP_TIMEOUT,
                response_header_timeout: Duration::from_secs(15),
                response_limit: JOB_RESPONSE_LIMIT,
                cancellation: Some(cancellation),
            },
        )
        .map_err(|error| match error {
            HttpExchangeError::Cancelled => ScanError::Cancelled("scan cancelled".into()),
            HttpExchangeError::Failed => ScanError::Unsupported(format!(
                "no response from {}:{}",
                self.endpoint.host, self.endpoint.port
            )),
        })?;
        if status >= 400 {
            return Err(ScanError::Unsupported(format!(
                "eSCL rejected job HTTP {status}"
            )));
        }
        canonical_job_path(&headers, &body, root, &self.endpoint)
            .map(|path| ScanJob { path })
            .ok_or_else(|| ScanError::Unsupported("eSCL did not return job id".into()))
    }

    fn fetch_document(
        &self,
        job: &ScanJob,
        response_limit: u64,
    ) -> std::result::Result<TemporaryOutput, FetchDocumentError> {
        let next = if job.path.ends_with("/NextDocument") {
            job.path.clone()
        } else {
            format!("{}/NextDocument", job.path)
        };
        let deadline = Instant::now() + NEXT_DOCUMENT_DEADLINE;
        let cancellation = self
            .cancellation
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .unwrap_or_default();
        loop {
            if self.is_cancelled() {
                return Err(FetchDocumentError::Cancelled);
            }
            let Some(timeout) = deadline.checked_duration_since(Instant::now()) else {
                return Err(FetchDocumentError::TimedOut);
            };
            let setup_timeout = timeout.min(NEXT_DOCUMENT_SETUP_TIMEOUT);
            let (status, output) =
                match http_get_to_temporary_output_with_phase_timeout_and_cancellation(
                    &self.endpoint,
                    &next,
                    timeout,
                    setup_timeout,
                    response_limit,
                    Some(&cancellation),
                ) {
                    Ok(result) => result,
                    Err(FetchDocumentError::RetryableTimeout) => continue,
                    Err(error) => return Err(error),
                };
            if let Some(output) = output {
                return Ok(output);
            }
            if matches!(status, 204 | 404 | 410) {
                return Err(FetchDocumentError::Exhausted);
            }
            if !matches!(status, 202 | 409 | 423 | 425 | 429 | 503) {
                return Err(FetchDocumentError::Failed(format!(
                    "NextDocument failed HTTP {status}"
                )));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let pause = NEXT_DOCUMENT_RETRY_DELAY.min(remaining);
            let pause_deadline = Instant::now() + pause;
            while Instant::now() < pause_deadline {
                if self.is_cancelled() {
                    return Err(FetchDocumentError::Cancelled);
                }
                std::thread::sleep(
                    Duration::from_millis(25)
                        .min(pause_deadline.saturating_duration_since(Instant::now())),
                );
            }
        }
    }

    fn cancel_job(&self, job: &ScanJob) {
        let _ = http_exchange_with_cancellation_result(
            &self.endpoint,
            "DELETE",
            &job.path,
            HttpExchangeOptions {
                body: None,
                content_type: None,
                global_timeout: CANCEL_JOB_CLEANUP_TIMEOUT,
                setup_timeout: CANCEL_JOB_CLEANUP_TIMEOUT,
                response_header_timeout: CANCEL_JOB_CLEANUP_TIMEOUT,
                response_limit: JOB_RESPONSE_LIMIT,
                // Cleanup must still be allowed to issue DELETE after the
                // operation token was cancelled. Its independent cap bounds a
                // stalled scanner without extending the cancelled operation.
                cancellation: Some(CancellationToken::new()),
            },
        );
    }

    fn decode_document(
        &self,
        request: &ScanRequest,
        output_file: &TemporaryOutput,
        limits: DocumentLimits,
    ) -> Result<Option<ImageBuffer>> {
        let decode_path = document_path(output_file.path())?;
        let decoded = crate::imaging::load_image_with_limits(
            &decode_path,
            limits.max_width,
            limits.max_height,
            limits.max_allocation,
        );
        let _ = std::fs::remove_file(&decode_path);
        let Ok(mut image) = decoded else {
            return Ok(None);
        };
        let (target_width, target_height) = requested_document_dimensions(request);
        if target_width > 0
            && target_height > 0
            && (image.width != target_width || image.height != target_height)
        {
            // The scanner can negotiate a different advertised acquisition DPI;
            // preserve ScanRequest's pixel-dimension output contract.
            image = crate::device::FileDeviceSession::resize_nearest(
                &image,
                target_width,
                target_height,
            )?;
        }
        Ok(Some(image))
    }

    fn acquire_http(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        let mut image = None;
        let summary = self.acquire_http_pages(request, 1, &mut |page| {
            image = Some(page);
            Ok(())
        })?;
        image.ok_or_else(|| {
            let message = match summary.end {
                crate::device::ScanPagesEnd::FeederExhausted => {
                    "eSCL feeder exhausted before returning an image"
                }
                crate::device::ScanPagesEnd::LimitReached => {
                    "eSCL stopped without returning the requested image"
                }
            };
            ScanError::Unsupported(message.into())
        })
    }

    fn acquire_http_pages(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<crate::device::ScanPagesResult> {
        if self.is_cancelled() {
            return Err(ScanError::Cancelled("scan cancelled".into()));
        }
        let cancellation = self.cancellation_token();
        let capabilities = capabilities_for_endpoint_with_cancellation_result(
            &self.endpoint,
            Instant::now() + ENDPOINT_PROBE_TIMEOUT,
            Some(cancellation),
        )
        .map_err(|error| match error {
            HttpExchangeError::Cancelled => ScanError::Cancelled("scan cancelled".into()),
            HttpExchangeError::Failed => {
                ScanError::Unsupported("eSCL ScannerCapabilities unavailable".into())
            }
        })?
        .ok_or_else(|| ScanError::Unsupported("eSCL ScannerCapabilities unavailable".into()))?;
        let (settings, representation) = self.build_settings(request, &capabilities)?;
        // Validate the negotiated source representation before creating a job.
        let document_limits = DocumentLimits::for_request(request, representation)?;
        let alternate_root = if capabilities.root.eq_ignore_ascii_case("escl") {
            "Scan"
        } else {
            "eSCL"
        };
        let mut last_error = String::new();
        for root in [capabilities.root.as_str(), alternate_root] {
            if self.is_cancelled() {
                return Err(ScanError::Cancelled("scan cancelled".into()));
            }
            let job = match self.create_job(root, &settings) {
                Ok(job) => job,
                Err(error @ ScanError::Cancelled(_)) => return Err(error),
                Err(error) => {
                    last_error = error.to_string();
                    continue;
                }
            };
            return self.stream_job_pages(request, max_pages, emit, &job, document_limits);
        }
        Err(ScanError::Unsupported(format!(
            "eSCL acquire failed: {last_error}"
        )))
    }

    fn stream_job_pages(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
        job: &ScanJob,
        document_limits: DocumentLimits,
    ) -> Result<crate::device::ScanPagesResult> {
        let mut emitted = 0;
        loop {
            if self.is_cancelled() {
                self.cancel_job(job);
                return Err(ScanError::Cancelled("scan cancelled".into()));
            }
            if emitted == max_pages {
                self.cancel_job(job);
                return Ok(crate::device::ScanPagesResult::limit_reached(emitted));
            }
            let output = match self.fetch_document(job, document_limits.response_limit) {
                Ok(output) => output,
                Err(FetchDocumentError::Exhausted) => {
                    return Ok(crate::device::ScanPagesResult::feeder_exhausted(emitted));
                }
                Err(FetchDocumentError::Cancelled) => {
                    self.cancel_job(job);
                    return Err(ScanError::Cancelled("scan cancelled".into()));
                }
                Err(FetchDocumentError::TimedOut | FetchDocumentError::RetryableTimeout) => {
                    self.cancel_job(job);
                    return Err(ScanError::Unsupported("eSCL NextDocument timed out".into()));
                }
                Err(FetchDocumentError::Failed(error)) => {
                    self.cancel_job(job);
                    return Err(ScanError::Unsupported(format!(
                        "eSCL acquire failed: {error}"
                    )));
                }
            };
            let image = match self.decode_document(request, &output, document_limits) {
                Ok(Some(image)) => image,
                Ok(None) => {
                    self.cancel_job(job);
                    return Err(ScanError::Unsupported(
                        "eSCL returned an undecodable raster image".into(),
                    ));
                }
                Err(error) => {
                    self.cancel_job(job);
                    return Err(error);
                }
            };
            if let Err(error) = emit(image) {
                self.cancel_job(job);
                return Err(error);
            }
            emitted += 1;
        }
    }
}

fn select_source(
    request: &ScanRequest,
    capabilities: &ScannerCapabilities,
) -> Result<SourceCapabilities> {
    let source = match (request.mode, request.duplex) {
        (ScanMode::Reflective, false) => CapabilitySource::Platen,
        (ScanMode::Reflective, true) => {
            return Err(ScanError::Unsupported(
                "duplex scanning is only supported for document ADF sources".into(),
            ));
        }
        (ScanMode::Document, false) => CapabilitySource::AdfSimplex,
        (ScanMode::Document, true) => CapabilitySource::AdfDuplex,
        (ScanMode::Film, true) => {
            return Err(ScanError::Unsupported(
                "duplex scanning is not supported for film sources".into(),
            ));
        }
        (ScanMode::Film, false) => CapabilitySource::Film,
    };
    if let Some(capabilities) = capabilities.source(source) {
        return Ok(capabilities.clone());
    }
    if source == CapabilitySource::Platen
        && capabilities
            .sources
            .iter()
            .all(|capabilities| capabilities.source.is_none())
    {
        return Ok(capabilities.source_or_default(source));
    }
    let description = match source {
        CapabilitySource::Platen => "platen",
        CapabilitySource::AdfSimplex => "ADF simplex",
        CapabilitySource::AdfDuplex => "ADF duplex",
        CapabilitySource::Film => "film/transparency",
    };
    Err(ScanError::Unsupported(format!(
        "scanner does not advertise a {description} source"
    )))
}

fn select_color_mode(
    request: &ScanRequest,
    source: &SourceCapabilities,
) -> Result<NegotiatedColorMode> {
    let preferred = if request.pixel_format == PixelFormat::Gray8 {
        ["Grayscale8", "BlackAndWhite1"]
    } else {
        ["RGB24", "RGB48"]
    };
    let color_mode = if source.color_modes.is_empty() {
        preferred[0].to_string()
    } else {
        preferred
            .iter()
            .find_map(|wanted| {
                source
                    .color_modes
                    .iter()
                    .find(|advertised| advertised.eq_ignore_ascii_case(wanted))
                    .cloned()
            })
            .ok_or_else(|| {
                ScanError::Unsupported(format!(
                    "scanner does not advertise a compatible color mode for {:?}",
                    request.pixel_format
                ))
            })?
    };
    let representation = match color_mode.as_str() {
        value if value.eq_ignore_ascii_case("BlackAndWhite1") => {
            DocumentRepresentation::BlackAndWhite1
        }
        value if value.eq_ignore_ascii_case("Grayscale8") => DocumentRepresentation::Grayscale8,
        value if value.eq_ignore_ascii_case("RGB24") => DocumentRepresentation::Rgb24,
        value if value.eq_ignore_ascii_case("RGB48") => DocumentRepresentation::Rgb48,
        _ => unreachable!("selected eSCL color mode is not in the preferred set"),
    };
    Ok(NegotiatedColorMode {
        name: color_mode,
        representation,
    })
}

fn select_document_format(source: &SourceCapabilities) -> Result<String> {
    if source.document_formats.is_empty() {
        return Ok("application/octet-stream".into());
    }
    const DECODABLE: [&str; 3] = ["image/png", "image/jpeg", "image/tiff"];
    DECODABLE
        .iter()
        .find_map(|wanted| {
            source
                .document_formats
                .iter()
                .find(|advertised| advertised.eq_ignore_ascii_case(wanted))
                .cloned()
        })
        .ok_or_else(|| {
            ScanError::Unsupported(
                "scanner advertises no decodable raster format (PDF is not a raster image)".into(),
            )
        })
}

fn select_resolution(request: &ScanRequest, source: &SourceCapabilities) -> (u32, u32) {
    let wanted = (request.dpi_x, request.dpi_y);
    source
        .resolutions
        .iter()
        .copied()
        .min_by_key(|(x, y)| x.abs_diff(wanted.0) as u64 + y.abs_diff(wanted.1) as u64)
        .unwrap_or(wanted)
}

fn pixels_to_three_hundredths(pixels: u32, dpi: u32) -> u32 {
    ((pixels as u64)
        .saturating_mul(300)
        .saturating_add((dpi.max(1) / 2) as u64)
        / dpi.max(1) as u64)
        .max(1)
        .min(u32::MAX as u64) as u32
}

fn offset_to_three_hundredths(pixels: u32, dpi: u32) -> u32 {
    if pixels == 0 {
        0
    } else {
        pixels_to_three_hundredths(pixels, dpi)
    }
}

fn region_in_three_hundredths(
    request: &ScanRequest,
    requested_dpi_x: u32,
    requested_dpi_y: u32,
    source: &SourceCapabilities,
) -> (u32, u32, u32, u32) {
    let region = request.region.unwrap_or(crate::core::Rect::new(
        0,
        0,
        request.width.max(1),
        request.height.max(1),
    ));
    let max_width = source.max_width.unwrap_or(u32::MAX).max(1);
    let max_height = source.max_height.unwrap_or(u32::MAX).max(1);
    let x = offset_to_three_hundredths(region.x.max(0) as u32, requested_dpi_x)
        .min(max_width.saturating_sub(1));
    let y = offset_to_three_hundredths(region.y.max(0) as u32, requested_dpi_y)
        .min(max_height.saturating_sub(1));
    let width = pixels_to_three_hundredths(region.width.max(1), requested_dpi_x)
        .min(max_width.saturating_sub(x).max(1));
    let height = pixels_to_three_hundredths(region.height.max(1), requested_dpi_y)
        .min(max_height.saturating_sub(y).max(1));
    (x, y, width, height)
}

#[derive(Debug)]
enum FetchDocumentError {
    Cancelled,
    Exhausted,
    TimedOut,
    RetryableTimeout,
    Failed(String),
}

#[derive(Debug, Clone)]
struct ScanJob {
    path: String,
}

fn canonical_job_path(
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

fn job_location(headers: &str, body: &[u8]) -> Option<String> {
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
                        return event.decode().ok().map(|v| v.into_owned())
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

fn normalize_job_location(location: &str, endpoint: &Endpoint) -> Option<String> {
    if location.is_empty()
        || location
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return None;
    }
    let path = if location.starts_with('/') {
        location.to_string()
    } else if let Some(authority_and_path) = location.strip_prefix("https://") {
        let (authority, path) = authority_and_path.split_once('/')?;
        (endpoint.secure && parse_endpoint(&format!("https://{authority}"))? == *endpoint)
            .then_some(path)
            .map(|path| format!("/{path}"))?
    } else if let Some(authority_and_path) = location.strip_prefix("http://") {
        let (authority, path) = authority_and_path.split_once('/')?;
        (!endpoint.secure && parse_endpoint(&format!("http://{authority}"))? == *endpoint)
            .then_some(path)
            .map(|path| format!("/{path}"))?
    } else {
        return None;
    };
    if path.contains(['?', '#'])
        || !path.contains("/ScanJobs/")
        || path.split('/').any(|segment| matches!(segment, "." | ".."))
    {
        return None;
    }
    Some(path.trim_end_matches('/').to_string())
}

fn document_path(path: &std::path::Path) -> Result<std::path::PathBuf> {
    let bytes = crate::imaging::read_magic(path)?;
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

impl crate::device::DeviceSession for EsclDeviceSession {
    fn scan(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        crate::device::reject_single_page_duplex(request)?;
        if self.is_closed() {
            return Err(ScanError::Other("session closed".into()));
        }
        if self.is_cancelled() {
            return Err(ScanError::Cancelled("scan cancelled".into()));
        }
        if self.simulate {
            return crate::device::MockDeviceSession::gradient(request);
        }
        self.acquire_http(request)
    }

    fn scan_pages(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<crate::device::ScanPagesResult> {
        crate::device::validate_page_limit(max_pages)?;
        if self.is_closed() {
            return Err(ScanError::Other("session closed".into()));
        }
        if self.is_cancelled() {
            return Err(ScanError::Cancelled("scan cancelled".into()));
        }
        if self.simulate {
            for index in 0..max_pages {
                if self.is_cancelled() {
                    return Err(ScanError::Cancelled("scan cancelled".into()));
                }
                let mut page_request = request.clone();
                page_request.seed = request.seed.saturating_add(index);
                emit(crate::device::MockDeviceSession::gradient(&page_request)?)?;
            }
            return Ok(crate::device::ScanPagesResult::limit_reached(max_pages));
        }
        self.acquire_http_pages(request, max_pages, emit)
    }

    fn cancel(&self) {
        if let Ok(mut g) = self.cancelled.lock() {
            *g = true;
        }
        if let Ok(token) = self.cancellation.lock() {
            if let Some(token) = token.as_ref() {
                token.cancel();
            }
        }
    }

    fn bind_cancellation(&self, token: CancellationToken) {
        if let Ok(mut cancellation) = self.cancellation.lock() {
            if self
                .cancelled
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .to_owned()
            {
                token.cancel();
            }
            *cancellation = Some(token);
        }
    }

    fn close(&self) {
        if let Ok(mut g) = self.closed.lock() {
            *g = true;
        }
    }

    fn calibrate(&self) -> serde_json::Value {
        serde_json::json!({
            "ok": false,
            "status": "unsupported",
            "backend": "escl",
            "device_id": self.device_id,
        })
    }

    fn focus(&self, x: f64, y: f64) -> serde_json::Value {
        serde_json::json!({
            "ok": false,
            "status": "unsupported",
            "x_frac": x.clamp(0.0, 1.0),
            "y_frac": y.clamp(0.0, 1.0),
            "focus": 0.0,
            "backend": "escl",
        })
    }
}

fn parse_escl_id(device_id: &str) -> Option<Endpoint> {
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
    let id = device_id;
    if !id.starts_with("escl:") && id != "escl" {
        return Err(ScanError::DeviceNotFound(format!(
            "not an eSCL device id: {id}"
        )));
    }
    if id == "escl:sim" {
        return Ok(EsclDeviceSession::new(
            id.into(),
            Endpoint {
                host: "sim".into(),
                port: 0,
                secure: false,
            },
            true,
        ));
    }
    let endpoint =
        parse_escl_id(id).ok_or_else(|| ScanError::DeviceNotFound(format!("bad eSCL id: {id}")))?;
    if endpoint.host == "sim" {
        return Ok(EsclDeviceSession::new(id.into(), endpoint, true));
    }
    let listed = list_devices();
    let explicitly_allowed = env_hosts().iter().any(|candidate| candidate == &endpoint);
    if !listed.iter().any(|device| device.id == id) && !explicitly_allowed {
        return Err(ScanError::DeviceNotFound(format!(
            "unknown eSCL device: {id}"
        )));
    }
    Ok(EsclDeviceSession::new(id.into(), endpoint, false))
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
