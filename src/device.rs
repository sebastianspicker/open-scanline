//! Device backends (OSL-DEVICE). Mock always available; OS backends empty-safe.

use crate::core::{ImageBuffer, PixelFormat, Result, ScanError, ScanRequest};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

mod file;
mod mock;

use file::parse_file_device_id;
pub use file::{FileBackend, FileDeviceSession};
pub use mock::{MockDevice, MockDeviceSession};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub manufacturer_id: Option<String>,
    pub manufacturer_name: Option<String>,
    pub support_profile: Option<String>,
}

impl DeviceInfo {
    pub fn new(id: impl Into<String>, name: impl Into<String>, kind: impl Into<String>) -> Self {
        let name = name.into();
        let mut device = Self {
            id: id.into(),
            name,
            kind: kind.into(),
            manufacturer_id: None,
            manufacturer_name: None,
            support_profile: None,
        };
        if let Some(manufacturer) =
            crate::manufacturers::resolve_manufacturer(&device.name).manufacturer
        {
            device.manufacturer_id = Some(manufacturer.id);
            device.manufacturer_name = Some(manufacturer.canonical_name);
        }
        device
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendInfo {
    pub id: String,
    pub name: String,
    pub available: bool,
}

/// A cloneable, operation-wide cancellation signal shared by orchestration and
/// the opened device session.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_arc(flag: Arc<AtomicBool>) -> Self {
        Self(flag)
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn as_arc(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.0)
    }
}

/// Controls opt-in device-opening exceptions. Defaults remain conservative.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeviceOpenPolicy {
    pub allow_unlisted_escl: bool,
}

/// Why a bounded multi-page acquisition stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanPagesEnd {
    /// The caller's requested page limit was reached.
    LimitReached,
    /// The document feeder reported that no additional side is available.
    FeederExhausted,
}

/// Summary returned by a streaming multi-page acquisition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanPagesResult {
    pub emitted: u32,
    pub end: ScanPagesEnd,
}

/// Upper bound for one acquisition job. This keeps command arguments,
/// temporary-file fan-out, timeouts, and caller-side collections finite even
/// when the streaming API is called directly rather than through the CLI.
pub const MAX_SCAN_PAGES: u32 = 1_000;

impl ScanPagesResult {
    pub const fn limit_reached(emitted: u32) -> Self {
        Self {
            emitted,
            end: ScanPagesEnd::LimitReached,
        }
    }

    pub const fn feeder_exhausted(emitted: u32) -> Self {
        Self {
            emitted,
            end: ScanPagesEnd::FeederExhausted,
        }
    }
}

pub(crate) fn reject_single_page_duplex(request: &ScanRequest) -> Result<()> {
    if request.duplex {
        return Err(ScanError::Unsupported(
            "duplex acquisition produces multiple sides; use the batch API with an even page limit"
                .into(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_page_limit(max_pages: u32) -> Result<()> {
    if max_pages == 0 {
        return Err(ScanError::Invalid("page limit must be positive".into()));
    }
    if max_pages > MAX_SCAN_PAGES {
        return Err(ScanError::Invalid(format!(
            "page limit {max_pages} exceeds the supported maximum of {MAX_SCAN_PAGES} logical sides"
        )));
    }
    Ok(())
}

pub trait DeviceSession: Send {
    fn scan(&self, request: &ScanRequest) -> Result<ImageBuffer>;
    /// Stream at most `max_pages` images through one open device session.
    ///
    /// Backends with a native feeder/job override this method. The default is
    /// appropriate for synthetic, file, and flatbed sources and increments the
    /// deterministic seed for each emitted page.
    fn scan_pages(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<ScanPagesResult> {
        validate_page_limit(max_pages)?;
        if request.duplex {
            return Err(ScanError::Unsupported(
                "this backend does not implement streaming duplex acquisition".into(),
            ));
        }
        for index in 0..max_pages {
            let mut page_request = request.clone();
            page_request.seed = request.seed.saturating_add(index);
            emit(self.scan(&page_request)?)?;
        }
        Ok(ScanPagesResult::limit_reached(max_pages))
    }
    fn preview(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        self.scan(request)
    }
    fn set_params(&self, _request: &ScanRequest) -> Result<()> {
        Ok(())
    }
    /// Bind the operation-wide cancellation signal after opening the session.
    /// Implementations that do not block in native acquisition can ignore it.
    fn bind_cancellation(&self, _token: CancellationToken) {}
    fn cancel(&self) {}
    fn close(&self) {}
    /// Productive calibrate hook; mock stores flat/dark tables applied on scan.
    fn calibrate(&self) -> serde_json::Value {
        serde_json::json!({"ok": false, "status": "unsupported"})
    }
    /// Productive focus hook at fractional platen point.
    fn focus(&self, x: f64, y: f64) -> serde_json::Value {
        serde_json::json!({
            "ok": false,
            "status": "unsupported",
            "x_frac": x,
            "y_frac": y,
        })
    }
}

/// Column-wise dark subtract + flat-field multiply (`apply_flat_dark_cal`).
pub fn apply_flat_dark_cal(img: &ImageBuffer, dark: &[i32], flat: &[f64]) -> Result<ImageBuffer> {
    if flat.is_empty() {
        return Ok(img.clone());
    }
    let ntab = flat.len();
    let w = img.width as usize;
    let h = img.height as usize;
    let bpp = img.bpp();
    let mut out = img.data.clone();
    let mut i = 0usize;
    for _y in 0..h {
        for x in 0..w {
            let idx = if w > 1 { (x * (ntab - 1)) / (w - 1) } else { 0 };
            let d = dark.get(idx).copied().unwrap_or(0);
            let f = flat.get(idx).copied().unwrap_or(1.0);
            let chans = if img.pixel_format == PixelFormat::Gray8 {
                1
            } else {
                3.min(bpp)
            };
            for c in 0..chans {
                let v = out[i + c] as i32;
                let nv = (((v - d).max(0) as f64) * f + d as f64 * 0.25).round() as i32;
                out[i + c] = nv.clamp(0, 255) as u8;
            }
            i += bpp;
        }
    }
    ImageBuffer::new(img.width, img.height, img.pixel_format, out)
}

pub fn synthetic_cal_tables() -> (Vec<i32>, Vec<f64>) {
    let n = 64;
    let dark = vec![2i32; n];
    let mut flat = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64 / (n - 1).max(1) as f64;
        let g = 1.0 + 0.04 * (t - 0.5).abs() * 2.0;
        flat.push((g * 10000.0).round() / 10000.0);
    }
    (dark, flat)
}

/// Concrete session enum (avoids dyn lifetime issues in callers).
pub enum AnySession {
    Mock(MockDeviceSession),
    File(FileDeviceSession),
    Wia(crate::wia::WiaDeviceSession),
    Sane(crate::sane::SaneDeviceSession),
    Escl(crate::escl::EsclDeviceSession),
}

impl DeviceSession for AnySession {
    fn scan(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        match self {
            AnySession::Mock(s) => s.scan(request),
            AnySession::File(s) => s.scan(request),
            AnySession::Wia(s) => s.scan(request),
            AnySession::Sane(s) => s.scan(request),
            AnySession::Escl(s) => s.scan(request),
        }
    }
    fn scan_pages(
        &self,
        request: &ScanRequest,
        max_pages: u32,
        emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    ) -> Result<ScanPagesResult> {
        match self {
            AnySession::Mock(session) => session.scan_pages(request, max_pages, emit),
            AnySession::File(session) => session.scan_pages(request, max_pages, emit),
            AnySession::Wia(session) => session.scan_pages(request, max_pages, emit),
            AnySession::Sane(session) => session.scan_pages(request, max_pages, emit),
            AnySession::Escl(session) => session.scan_pages(request, max_pages, emit),
        }
    }
    fn preview(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        match self {
            AnySession::Mock(s) => s.preview(request),
            AnySession::File(s) => s.preview(request),
            AnySession::Wia(s) => s.preview(request),
            AnySession::Sane(s) => s.preview(request),
            AnySession::Escl(s) => s.preview(request),
        }
    }
    fn set_params(&self, request: &ScanRequest) -> Result<()> {
        match self {
            AnySession::Mock(s) => s.set_params(request),
            AnySession::File(s) => s.set_params(request),
            AnySession::Wia(s) => s.set_params(request),
            AnySession::Sane(s) => s.set_params(request),
            AnySession::Escl(s) => s.set_params(request),
        }
    }
    fn bind_cancellation(&self, token: CancellationToken) {
        match self {
            AnySession::Mock(s) => s.bind_cancellation(token),
            AnySession::File(s) => s.bind_cancellation(token),
            AnySession::Wia(s) => s.bind_cancellation(token),
            AnySession::Sane(s) => s.bind_cancellation(token),
            AnySession::Escl(s) => s.bind_cancellation(token),
        }
    }
    fn cancel(&self) {
        match self {
            AnySession::Mock(s) => s.cancel(),
            AnySession::File(s) => s.cancel(),
            AnySession::Wia(s) => s.cancel(),
            AnySession::Sane(s) => s.cancel(),
            AnySession::Escl(s) => s.cancel(),
        }
    }
    fn close(&self) {
        match self {
            AnySession::Mock(s) => s.close(),
            AnySession::File(s) => s.close(),
            AnySession::Wia(s) => s.close(),
            AnySession::Sane(s) => s.close(),
            AnySession::Escl(s) => s.close(),
        }
    }
    fn calibrate(&self) -> serde_json::Value {
        match self {
            AnySession::Mock(s) => s.calibrate(),
            AnySession::File(s) => s.calibrate(),
            AnySession::Wia(s) => s.calibrate(),
            AnySession::Sane(s) => s.calibrate(),
            AnySession::Escl(s) => s.calibrate(),
        }
    }
    fn focus(&self, x: f64, y: f64) -> serde_json::Value {
        match self {
            AnySession::Mock(s) => s.focus(x, y),
            AnySession::File(s) => s.focus(x, y),
            AnySession::Wia(s) => s.focus(x, y),
            AnySession::Sane(s) => s.focus(x, y),
            AnySession::Escl(s) => s.focus(x, y),
        }
    }
}

/// Open a session for `device_id` using the correct backend.
pub fn open_device(device_id: &str) -> Result<AnySession> {
    open_device_with_policy(device_id, DeviceOpenPolicy::default())
}

/// Open a device session with explicit, narrowly-scoped opening policy.
pub fn open_device_with_policy(device_id: &str, policy: DeviceOpenPolicy) -> Result<AnySession> {
    let id = resolve_device_id(Some(device_id));
    if let Some(session) = open_prefixed_device(&id, policy)? {
        return Ok(session);
    }
    if let Some(session) = open_enumerated_device(&id)? {
        return Ok(session);
    }
    Err(ScanError::DeviceNotFound(id))
}

/// Route ids with an explicit backend prefix before matching enumerated ids.
fn open_prefixed_device(id: &str, policy: DeviceOpenPolicy) -> Result<Option<AnySession>> {
    if id == "mock" || id.starts_with("mock") {
        return Ok(Some(AnySession::Mock(MockDeviceSession::new())));
    }
    if id.starts_with("file:") || id.starts_with("file://") {
        let path = parse_file_device_id(id);
        return if path.is_file() {
            Ok(Some(AnySession::File(FileDeviceSession::new(path))))
        } else {
            Err(ScanError::DeviceNotFound(format!(
                "unknown file device: {id}"
            )))
        };
    }
    open_backend_prefix(id, policy)
}

fn open_backend_prefix(id: &str, policy: DeviceOpenPolicy) -> Result<Option<AnySession>> {
    let session = match () {
        _ if id.starts_with("wia:") || id == "wia" => Some(AnySession::Wia(crate::wia::open(id)?)),
        _ if id.starts_with("sane:") || id == "sane" => {
            Some(AnySession::Sane(crate::sane::open(id)?))
        }
        _ if id.starts_with("escl:") || id == "escl" => {
            let session = if policy.allow_unlisted_escl && id.starts_with("escl:") {
                crate::escl::open_explicit_id(id)?
            } else {
                crate::escl::open(id)?
            };
            Some(AnySession::Escl(session))
        }
        _ => None,
    };
    Ok(session)
}

/// Match a listed id and dispatch according to its advertised backend kind.
fn open_enumerated_device(id: &str) -> Result<Option<AnySession>> {
    let Some(device) = list_all_devices().into_iter().find(|device| {
        device.id == id
            && matches!(
                device.kind.as_str(),
                "mock" | "file" | "wia" | "sane" | "escl"
            )
    }) else {
        return Ok(None);
    };
    let session = match device.kind.as_str() {
        "mock" => Some(AnySession::Mock(MockDeviceSession::new())),
        "file" => Some(AnySession::File(FileDeviceSession::new(
            parse_file_device_id(&device.id),
        ))),
        "wia" => Some(AnySession::Wia(crate::wia::open(&device.id)?)),
        "sane" => Some(AnySession::Sane(crate::sane::open(&device.id)?)),
        "escl" => Some(AnySession::Escl(crate::escl::open(&device.id)?)),
        _ => None,
    };
    Ok(session)
}

/// Pick a usable device id; prefer explicit, else first from list_all_devices.
pub fn resolve_device_id(device: Option<&str>) -> String {
    if let Some(d) = device {
        if !d.trim().is_empty() {
            return d.trim().to_string();
        }
    }
    let devices = list_all_devices();
    if devices.is_empty() {
        return MockDevice::DEVICE_ID.to_string();
    }
    devices[0].id.clone()
}

/// Registered backends with availability flags.
pub fn list_backends() -> Vec<BackendInfo> {
    list_backends_with_cancellation(None)
}

/// Registered backends with cancellation for command-backed availability probes.
pub fn list_backends_with_cancellation(
    cancellation: Option<&CancellationToken>,
) -> Vec<BackendInfo> {
    let mut backends = vec![
        BackendInfo {
            id: "mock".into(),
            name: "open-scanline Synthetic Scanner".into(),
            available: true,
        },
        BackendInfo {
            id: "file".into(),
            name: "File image source (file:path)".into(),
            available: true,
        },
    ];
    backends.push(crate::wia::backend_info_with_cancellation(cancellation));
    backends.push(crate::sane::backend_info_with_cancellation(cancellation));
    backends.push(crate::escl::backend_info());
    backends
}

/// Multi-backend enumeration. Always includes mock; never panics.
pub fn list_all_devices() -> Vec<DeviceInfo> {
    list_all_devices_with_cancellation(None)
}

/// Multi-backend enumeration with cancellation for command-backed discovery.
pub fn list_all_devices_with_cancellation(
    cancellation: Option<&CancellationToken>,
) -> Vec<DeviceInfo> {
    let escl = std::panic::catch_unwind(|| {
        crate::escl::list_escl_devices_safe_with_cancellation(cancellation)
    })
    .unwrap_or_default();
    list_all_devices_with_escl_and_cancellation(escl, cancellation)
}

fn list_all_devices_with_escl(escl: Vec<DeviceInfo>) -> Vec<DeviceInfo> {
    list_all_devices_with_escl_and_cancellation(escl, None)
}

fn list_all_devices_with_escl_and_cancellation(
    escl: Vec<DeviceInfo>,
    cancellation: Option<&CancellationToken>,
) -> Vec<DeviceInfo> {
    let mut devices = MockDevice::list_devices();

    let file_devs = std::panic::catch_unwind(FileBackend::list_devices).unwrap_or_default();
    devices.extend(file_devs);

    let wia = std::panic::catch_unwind(|| {
        crate::wia::list_wia_devices_safe_with_cancellation(cancellation)
    })
    .unwrap_or_default();
    devices.extend(wia);

    let sane = std::panic::catch_unwind(|| {
        crate::sane::list_sane_devices_safe_with_cancellation(cancellation)
    })
    .unwrap_or_default();
    devices.extend(sane);

    devices.extend(escl);

    devices
}

/// Re-list local and network scanners. A refresh performs exactly one bounded
/// eSCL discovery; the following aggregate listing reuses that result.
pub fn find_scanners(refresh: bool) -> Vec<DeviceInfo> {
    if refresh {
        return list_all_devices_with_escl(crate::escl::refresh_devices());
    }
    list_all_devices()
}

/// Open session, run calibrate hook, close. Returns session calibrate result.
pub fn calibrate_device(device_id: &str) -> serde_json::Value {
    use serde_json::json;
    run_session_action(device_id, DeviceSession::calibrate, |error| {
        json!({
            "ok": false,
            "status": "error",
            "device_id": device_id,
            "error": error.to_string(),
        })
    })
}

/// Open session, run focus at fractional point, close.
pub fn focus_device(device_id: &str, x: f64, y: f64) -> serde_json::Value {
    use serde_json::json;
    run_session_action(
        device_id,
        |session| session.focus(x, y),
        |error| {
            json!({
                "ok": false,
                "status": "error",
                "device_id": device_id,
                "x_frac": x,
                "y_frac": y,
                "error": error.to_string(),
            })
        },
    )
}

fn run_session_action(
    device_id: &str,
    action: impl FnOnce(&AnySession) -> serde_json::Value,
    on_open_error: impl FnOnce(ScanError) -> serde_json::Value,
) -> serde_json::Value {
    match open_device(device_id) {
        Ok(session) => {
            let mut result = action(&session);
            session.close();
            if let Some(object) = result.as_object_mut() {
                object
                    .entry("device_id")
                    .or_insert_with(|| serde_json::json!(device_id));
            }
            result
        }
        Err(error) => on_open_error(error),
    }
}

/// Compute exposure gains from a preview buffer (mean-channel inverse).
pub fn exposure_gains_from_buffer(image: &ImageBuffer) -> serde_json::Value {
    use serde_json::json;
    let bpp = image.bpp();
    let n = (image.width * image.height).max(1) as f64;
    let mut sr = 0.0f64;
    let mut sg = 0.0f64;
    let mut sb = 0.0f64;
    if bpp == 1 {
        let sum: f64 = image.data.iter().map(|&v| v as f64).sum();
        let mean = (sum / n).max(1.0);
        let g = 128.0 / mean;
        return json!({
            "ok": true,
            "gain_r": g,
            "gain_g": g,
            "gain_b": g,
            "method": "gray-mean",
        });
    }
    for i in (0..image.data.len()).step_by(bpp) {
        sr += image.data[i] as f64;
        sg += image.data[i + 1] as f64;
        sb += image.data[i + 2] as f64;
    }
    let mr = (sr / n).max(1.0);
    let mg = (sg / n).max(1.0);
    let mb = (sb / n).max(1.0);
    let target = 128.0;
    json!({
        "ok": true,
        "gain_r": target / mr,
        "gain_g": target / mg,
        "gain_b": target / mb,
        "method": "rgb-mean",
    })
}

/// Open session, acquire a small preview, compute exposure gains, close.
pub fn exposure_from_preview(device_id: &str, request: &ScanRequest) -> serde_json::Value {
    use serde_json::json;
    match open_device(device_id) {
        Ok(session) => preview_exposure_result(device_id, request, &session),
        Err(e) => json!({
            "ok": false,
            "status": "error",
            "device_id": device_id,
            "error": e.to_string(),
        }),
    }
}

fn preview_exposure_result(
    device_id: &str,
    request: &ScanRequest,
    session: &AnySession,
) -> serde_json::Value {
    use serde_json::json;
    let mut preview_request = request.clone();
    preview_request.width = preview_request.width.clamp(16, 256);
    preview_request.height = preview_request.height.clamp(16, 256);
    let preview = session.preview(&preview_request);
    session.close();

    match preview {
        Ok(image) => {
            let mut gains = exposure_gains_from_buffer(&image);
            if let Some(object) = gains.as_object_mut() {
                object.insert("device_id".into(), json!(device_id));
                object.insert("status".into(), json!("ok"));
            }
            gains
        }
        Err(error) => json!({
            "ok": false,
            "status": "preview_failed",
            "device_id": device_id,
            "error": error.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::PipelinePrefs;
    use std::sync::Mutex;

    struct DefaultPageSession {
        seeds: Mutex<Vec<u32>>,
    }

    impl DeviceSession for DefaultPageSession {
        fn scan(&self, request: &ScanRequest) -> Result<ImageBuffer> {
            self.seeds.lock().unwrap().push(request.seed);
            ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![request.seed as u8, 0, 0])
        }
    }

    #[test]
    fn mock_gradient_corners() {
        let session = open_device("mock").unwrap();
        let req = ScanRequest {
            device_id: "mock".into(),
            width: 10,
            height: 5,
            dpi_x: 150,
            dpi_y: 150,
            seed: 99,
            pixel_format: PixelFormat::Rgb8,
            pipeline: PipelinePrefs::default(),
            ..ScanRequest::default()
        };
        let img = session.scan(&req).unwrap();
        assert_eq!(img.data[0], 0);
        assert_eq!(img.data[1], 0);
        assert_eq!(img.data[2], 99);
        let last = (4 * 10 + 9) * 3;
        assert_eq!(img.data[last], 255);
        assert_eq!(img.data[last + 1], 255);
        assert_eq!(img.data[last + 2], 99);
    }

    #[test]
    fn default_page_stream_is_bounded_seeded_and_stops_on_callback_error() {
        let session = DefaultPageSession {
            seeds: Mutex::new(Vec::new()),
        };
        let mut emitted = Vec::new();
        let summary = session
            .scan_pages(
                &ScanRequest {
                    seed: 7,
                    ..ScanRequest::default()
                },
                3,
                &mut |image| {
                    emitted.push(image.data[0]);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(summary, ScanPagesResult::limit_reached(3));
        assert_eq!(emitted, [7, 8, 9]);
        assert_eq!(*session.seeds.lock().unwrap(), [7, 8, 9]);

        let error = session
            .scan_pages(&ScanRequest::default(), 3, &mut |_image| {
                Err(ScanError::Other("stop callback".into()))
            })
            .unwrap_err();
        assert!(matches!(error, ScanError::Other(message) if message == "stop callback"));
    }

    #[test]
    fn default_page_stream_rejects_duplex_instead_of_losing_back_sides() {
        let session = DefaultPageSession {
            seeds: Mutex::new(Vec::new()),
        };
        let error = session
            .scan_pages(
                &ScanRequest {
                    mode: crate::core::ScanMode::Document,
                    duplex: true,
                    ..ScanRequest::default()
                },
                2,
                &mut |_image| Ok(()),
            )
            .unwrap_err();
        assert!(matches!(error, ScanError::Unsupported(message) if message.contains("duplex")));
        assert!(session.seeds.lock().unwrap().is_empty());
    }

    #[test]
    fn default_page_stream_rejects_zero_and_excessive_limits_before_scanning() {
        let session = DefaultPageSession {
            seeds: Mutex::new(Vec::new()),
        };
        for limit in [0, MAX_SCAN_PAGES + 1] {
            let error = session
                .scan_pages(&ScanRequest::default(), limit, &mut |_image| Ok(()))
                .unwrap_err();
            assert!(matches!(error, ScanError::Invalid(_)));
        }
        assert!(session.seeds.lock().unwrap().is_empty());
    }

    #[test]
    fn list_always_includes_mock() {
        let backends = list_backends();
        assert!(backends.iter().any(|b| b.id == "mock" && b.available));
        let devices = list_all_devices();
        assert!(devices.iter().any(|d| d.id == "mock"));
    }

    #[test]
    fn enumerated_names_are_enriched_with_catalog_manufacturer_identity() {
        let known = DeviceInfo::new("sane:usb:fixture", "SANE: Epson Perfection V600", "sane");
        assert_eq!(known.manufacturer_id.as_deref(), Some("epson"));
        assert_eq!(known.manufacturer_name.as_deref(), Some("Epson"));
        assert!(
            known.support_profile.is_none(),
            "catalog feature profiles are not device capability evidence"
        );

        let unknown = DeviceInfo::new("sane:unknown", "SANE: Generic Scanner", "sane");
        assert!(unknown.manufacturer_id.is_none());
        assert!(unknown.manufacturer_name.is_none());
    }

    #[test]
    fn resolve_defaults_to_mock() {
        assert_eq!(resolve_device_id(None), "mock");
        assert_eq!(resolve_device_id(Some("wia:foo")), "wia:foo");
    }

    #[test]
    fn prefix_routing_preserves_mock_and_file_not_found_behavior() {
        assert!(matches!(
            open_device("mock-with-file-looking-suffix"),
            Ok(AnySession::Mock(_))
        ));
        assert!(matches!(
            open_device("file:/open-scanline-device-does-not-exist"),
            Err(ScanError::DeviceNotFound(message))
                if message == "unknown file device: file:/open-scanline-device-does-not-exist"
        ));
    }

    #[test]
    fn find_calibrate_focus_exposure() {
        let devices = find_scanners(true);
        assert!(devices.iter().any(|d| d.id == "mock"));
        let cal = calibrate_device("mock");
        assert_eq!(cal["ok"], true);
        assert_eq!(cal["status"], "calibrated");
        let foc = focus_device("mock", 0.5, 0.5);
        assert_eq!(foc["ok"], true);
        assert!(foc.get("focus").is_some());
        let req = ScanRequest {
            device_id: "mock".into(),
            width: 64,
            height: 48,
            ..ScanRequest::default()
        };
        let exp = exposure_from_preview("mock", &req);
        assert_eq!(exp["ok"], true);
        assert!(exp.get("gain_r").is_some());
    }

    #[test]
    fn device_action_open_errors_preserve_operation_specific_fields() {
        let device_id = "missing-device";
        let calibration = calibrate_device(device_id);
        assert_eq!(calibration["status"], "error");
        assert_eq!(calibration["device_id"], device_id);

        let focus = focus_device(device_id, 0.25, 0.75);
        assert_eq!(focus["status"], "error");
        assert_eq!(focus["x_frac"], 0.25);
        assert_eq!(focus["y_frac"], 0.75);

        let exposure = exposure_from_preview(device_id, &ScanRequest::default());
        assert_eq!(exposure["status"], "error");
        assert_eq!(exposure["device_id"], device_id);
    }

    #[test]
    fn mock_calibrate_applies_on_scan() {
        let session = open_device("mock").unwrap();
        let req = ScanRequest {
            width: 32,
            height: 16,
            seed: 50,
            pixel_format: PixelFormat::Rgb8,
            ..ScanRequest::default()
        };
        let before = session.scan(&req).unwrap();
        let cal = session.calibrate();
        assert_eq!(cal["ok"], true);
        let after = session.scan(&req).unwrap();
        assert_eq!(after.width, before.width);
        // Flat/dark tables must alter at least some pixels
        assert_ne!(before.data, after.data);
        session.close();
    }

    #[test]
    fn backend_sim_open_scan_all() {
        for id in ["wia:sim", "sane:sim", "escl:sim"] {
            let session = open_device(id).unwrap_or_else(|e| panic!("open {id}: {e}"));
            let req = ScanRequest {
                width: 8,
                height: 6,
                seed: 1,
                pixel_format: PixelFormat::Rgb8,
                ..ScanRequest::default()
            };
            let img = session
                .scan(&req)
                .unwrap_or_else(|e| panic!("scan {id}: {e}"));
            assert_eq!(img.width, 8, "{id}");
            session.close();
        }
    }

    #[test]
    fn any_session_binds_the_shared_token_to_command_backends() {
        let token = CancellationToken::new();
        let session = AnySession::Sane(crate::sane::SaneDeviceSession::new(
            "sane:sim".into(),
            "sim".into(),
            true,
        ));
        session.bind_cancellation(token.clone());
        token.cancel();

        assert!(matches!(
            session.scan(&ScanRequest::default()),
            Err(ScanError::Cancelled(message)) if message == "scan cancelled"
        ));
    }

    #[test]
    fn explicit_escl_policy_opens_a_strict_unlisted_id_without_weakening_default_open() {
        let id = "escl:127.0.0.1:9";
        assert!(matches!(
            open_device_with_policy(
                id,
                DeviceOpenPolicy {
                    allow_unlisted_escl: true
                }
            ),
            Ok(AnySession::Escl(_))
        ));
        assert!(matches!(
            open_device_with_policy("escl:not a valid endpoint", DeviceOpenPolicy::default()),
            Err(ScanError::DeviceNotFound(_))
        ));
    }
}
