//! eSCL / AirScan network backend — list/open/scan with HTTP CreateScanJob.

use crate::domain::acquisition::{reject_single_page_duplex, validate_page_limit};
use crate::domain::acquisition::{ScanMode, ScanRequest};
use crate::domain::image::{ImageBuffer, PixelFormat};
use crate::error::{Result, ScanError};
use crate::infrastructure::acquisition::{
    simulate_backends, BackendInfo, DeviceInfo, DeviceSession, ScanPagesEnd, ScanPagesResult,
};
use crate::infrastructure::runtime::TemporaryOutput;
use crate::operation::CancellationToken;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use quick_xml::events::Event;
use quick_xml::Reader;
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
    crate::domain::image::MAX_IMAGE_BYTES as u64 + DOCUMENT_CONTAINER_OVERHEAD;
const MAX_DOCUMENT_DECODE_ALLOCATION: u64 = crate::domain::image::MAX_IMAGE_BYTES as u64;
const MIN_DOCUMENT_DECODE_ALLOCATION: u64 = 4 * 1024 * 1024;
const MIN_DOCUMENT_DIMENSION_LIMIT: u32 = 1_024;
const MAX_DOCUMENT_DIMENSION_LIMIT: u32 = crate::domain::image::MAX_IMAGE_DIMENSION;
const NEXT_DOCUMENT_DEADLINE: Duration = Duration::from_secs(60);
const NEXT_DOCUMENT_SETUP_TIMEOUT: Duration = Duration::from_secs(5);
const NEXT_DOCUMENT_RETRY_DELAY: Duration = Duration::from_millis(150);
const NEXT_DOCUMENT_CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);
const CONTROL_REQUEST_SETUP_TIMEOUT: Duration = Duration::from_secs(1);
const CANCEL_JOB_CLEANUP_TIMEOUT: Duration = Duration::from_millis(500);
static ESCL_DISCOVERY_LOCK: Mutex<()> = Mutex::new(());
static ESCL_DEVICE_CACHE: OnceLock<Mutex<Option<Vec<DeviceInfo>>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapabilitySource {
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
pub(crate) struct SourceCapabilities {
    source: Option<CapabilitySource>,
    color_modes: Vec<String>,
    document_formats: Vec<String>,
    resolutions: Vec<(u32, u32)>,
    max_width: Option<u32>,
    max_height: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ScannerCapabilities {
    make_and_model: Option<String>,
    root: String,
    sources: Vec<SourceCapabilities>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocumentRepresentation {
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
pub(crate) struct NegotiatedColorMode {
    name: String,
    representation: DocumentRepresentation,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DocumentLimits {
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
            merge_shared_capabilities(&mut selected, shared);
        }
        selected
    }
}

fn merge_shared_capabilities(selected: &mut SourceCapabilities, shared: &SourceCapabilities) {
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

/// Network discovery enabled unless OPEN_SCANLINE_NETWORK_DISCOVERY=0.
pub fn available() -> bool {
    !matches!(
        std::env::var("OPEN_SCANLINE_NETWORK_DISCOVERY"),
        Ok(value) if value.trim() == "0"
    )
}

pub fn backend_info() -> BackendInfo {
    BackendInfo {
        id: "escl".into(),
        name: "eSCL/AirScan network scanners".into(),
        available: available() || simulate_backends(),
    }
}

mod capabilities;
mod device_session;
mod discovery;
mod opening;
mod session;
mod settings;
mod transport;

pub(crate) use capabilities::*;
pub(crate) use discovery::*;
pub(crate) use opening::*;
pub(crate) use settings::*;
pub(crate) use transport::*;

pub use discovery::{
    discover_devices, list_devices, list_escl_devices_safe,
    list_escl_devices_safe_with_cancellation, refresh_devices, refresh_devices_with_cancellation,
};
pub use opening::{open, open_explicit_id, open_unlisted_endpoint, open_with_cancellation};
pub use session::EsclDeviceSession;

mod job_id;
pub use job_id::parse_job_id;

