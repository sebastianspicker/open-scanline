use super::*;

mod document;
pub(crate) use document::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Endpoint {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) secure: bool,
}

impl Endpoint {
    pub(crate) fn url(&self, path: &str) -> String {
        let scheme = if self.secure { "https" } else { "http" };
        let host = if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        format!("{scheme}://{host}:{}{path}", self.port)
    }

    pub(crate) fn device_id(&self) -> String {
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
pub(crate) fn env_hosts() -> Vec<Endpoint> {
    let Ok(raw) = std::env::var("OPEN_SCANLINE_ESCL_HOSTS") else {
        return Vec::new();
    };
    raw.split(',').filter_map(parse_endpoint).collect()
}

pub(crate) fn parse_endpoint(value: &str) -> Option<Endpoint> {
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

pub(crate) fn endpoint_authority(value: &str) -> (bool, &str) {
    if let Some(rest) = value.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = value.strip_prefix("http://") {
        (false, rest)
    } else {
        (false, value)
    }
}

pub(crate) fn endpoint_host_port(authority: &str, secure: bool) -> Option<(String, u16)> {
    let default_port = if secure { 443 } else { 80 };
    if let Some(rest) = authority.strip_prefix('[') {
        return bracketed_host_port(rest, default_port);
    }
    unbracketed_host_port(authority, default_port)
}

fn bracketed_host_port(rest: &str, default_port: u16) -> Option<(String, u16)> {
    let (host, suffix) = rest.split_once(']')?;
    if host.is_empty() || suffix.contains(']') {
        return None;
    }
    Some((
        host.parse::<Ipv6Addr>().ok()?.to_string(),
        parse_endpoint_port(suffix, default_port)?,
    ))
}

fn unbracketed_host_port(authority: &str, default_port: u16) -> Option<(String, u16)> {
    if authority.contains(['/', '?', '#', '@', '[', ']', '\\']) {
        return None;
    }
    let (host, port) = split_host_port(authority, default_port)?;
    Some((normalize_host(host)?, port))
}

fn split_host_port(authority: &str, default_port: u16) -> Option<(&str, u16)> {
    match authority.split_once(':') {
        Some((host, port)) => Some((
            host,
            parse_endpoint_port(&format!(":{port}"), default_port)?,
        )),
        None => Some((authority, default_port)),
    }
}

pub(crate) fn parse_endpoint_port(suffix: &str, default_port: u16) -> Option<u16> {
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

pub(crate) fn normalize_host(host: &str) -> Option<String> {
    if host.is_empty() {
        return None;
    }
    normalized_ip_address(host).or_else(|| normalized_hostname(host))
}

fn normalized_ip_address(host: &str) -> Option<String> {
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => Some(address.to_string()),
        Ok(IpAddr::V6(_)) => None,
        Err(_) if numeric_ipv4_candidate(host) => host
            .parse::<Ipv4Addr>()
            .ok()
            .map(|address| address.to_string()),
        Err(_) => None,
    }
}

fn numeric_ipv4_candidate(host: &str) -> bool {
    host.contains('.')
        && host
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
}

fn normalized_hostname(host: &str) -> Option<String> {
    (host.len() <= 253 && host.split('.').all(valid_hostname_label))
        .then(|| host.to_ascii_lowercase())
}

fn valid_hostname_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

pub(crate) struct HttpExchangeOptions<'a> {
    pub(crate) body: Option<&'a [u8]>,
    pub(crate) content_type: Option<&'a str>,
    pub(crate) global_timeout: Duration,
    pub(crate) setup_timeout: Duration,
    pub(crate) response_header_timeout: Duration,
    pub(crate) response_limit: u64,
    pub(crate) cancellation: Option<CancellationToken>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HttpExchangeError {
    Cancelled,
    Failed,
}

pub(crate) fn map_http_exchange_error(error: &ureq::Error) -> HttpExchangeError {
    match error {
        ureq::Error::Io(error) if error.kind() == std::io::ErrorKind::Interrupted => {
            HttpExchangeError::Cancelled
        }
        _ => HttpExchangeError::Failed,
    }
}

pub(crate) fn http_exchange_with_cancellation_result(
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
    let response = send_http_exchange(&agent, method, &url, body, content_type)?;
    read_http_exchange_response(response, response_limit)
}

fn send_http_exchange(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    content_type: Option<&str>,
) -> std::result::Result<ureq::http::Response<ureq::Body>, HttpExchangeError> {
    match (method, body) {
        ("GET", None) => agent
            .get(url)
            .call()
            .map_err(|error| map_http_exchange_error(&error)),
        ("POST", Some(bytes)) => {
            let request = agent.post(url).header(
                "Content-Type",
                content_type.unwrap_or("application/octet-stream"),
            );
            request
                .send(bytes)
                .map_err(|error| map_http_exchange_error(&error))
        }
        ("DELETE", None) => agent
            .delete(url)
            .call()
            .map_err(|error| map_http_exchange_error(&error)),
        _ => Err(HttpExchangeError::Failed),
    }
}

fn read_http_exchange_response(
    response: ureq::http::Response<ureq::Body>,
    response_limit: u64,
) -> std::result::Result<(u16, Vec<u8>, String), HttpExchangeError> {
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
pub(crate) struct MdnsLocalResolver {
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

pub(crate) fn filter_mdns_resolved_addresses(
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

pub(crate) fn scanner_agent_with_phase_timeout(
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
pub(crate) struct CancellationConnector {
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

pub(crate) struct CancellableTransport {
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
