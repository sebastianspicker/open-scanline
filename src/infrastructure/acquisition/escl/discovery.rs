use super::*;

type DiscoveredDevices = Arc<Mutex<Vec<(usize, DeviceInfo)>>>;

fn probe_worker(
    endpoints: Arc<Vec<Endpoint>>,
    next: Arc<AtomicUsize>,
    found: DiscoveredDevices,
    deadline: Instant,
    cancellation: Option<CancellationToken>,
) {
    loop {
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
    }
}

pub(crate) fn probe_endpoint_until_with_cancellation(
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

pub(crate) fn probe_endpoints_with_cancellation(
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
        handles.push(std::thread::spawn(move || {
            probe_worker(endpoints, next, found, deadline, cancellation);
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

pub(crate) fn mdns_endpoints_with_cancellation(
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

pub(crate) fn resolved_mdns_endpoints_with_cancellation(
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

pub(crate) fn endpoints_for_service(
    info: &mdns_sd::ServiceInfo,
    secure_service: bool,
) -> Vec<Endpoint> {
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

pub(crate) fn mdns_address_is_local(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => ipv4_address_is_local(address),
        IpAddr::V6(address) => address.is_unique_local() || address.is_unicast_link_local(),
    }
}

pub(crate) fn ipv4_address_is_local(address: std::net::Ipv4Addr) -> bool {
    address.is_private() || address.is_link_local()
}

/// Returns a capped set of targets from `OPEN_SCANLINE_ESCL_SUBNETS`. A value may
/// be a host prefix (`192.168.1`) or an IPv4 CIDR. We intentionally never infer
/// a subnet or probe more than 64 addresses: discovery stays local and bounded.
pub(crate) fn subnet_endpoints(limit: usize) -> Vec<Endpoint> {
    let Ok(raw) = std::env::var("OPEN_SCANLINE_ESCL_SUBNETS") else {
        return Vec::new();
    };
    let cap = limit.min(64);
    raw.split(',')
        .flat_map(|entry| subnet_candidates(entry.trim(), cap))
        .take(cap)
        .collect()
}

pub(crate) fn subnet_candidates(value: &str, cap: usize) -> Vec<Endpoint> {
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

pub(crate) fn discover_devices_locked(max_hosts: usize) -> Vec<DeviceInfo> {
    discover_devices_locked_with_cancellation(max_hosts, None)
}

pub(crate) fn discover_devices_locked_with_cancellation(
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
    refresh_devices_with_cancellation(None)
}

pub fn refresh_devices_with_cancellation(
    cancellation: Option<&CancellationToken>,
) -> Vec<DeviceInfo> {
    let _discovery = ESCL_DISCOVERY_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let devices =
        std::panic::catch_unwind(|| discover_devices_locked_with_cancellation(32, cancellation))
            .unwrap_or_default();
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Vec::new();
    }
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
