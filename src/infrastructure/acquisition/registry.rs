//! Concrete multi-backend session registry.

use super::{escl, file, mock, sane, wia};
use crate::domain::acquisition::{DeviceOpenPolicy, ScanRequest};
use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use crate::workflows::operation::CancellationToken;
use crate::workflows::ports::acquisition::{
    AcquisitionPort, BackendInfo, DeviceInfo, DeviceSession, ScanPagesResult,
};

use file::parse_file_device_id;
pub use file::{FileBackend, FileDeviceSession};
pub use mock::{MockDevice, MockDeviceSession};

/// Native scanner registry adapter supplied by [`crate::composition::Runtime`].
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeAcquisition;

impl AcquisitionPort for NativeAcquisition {
    fn resolve_device_id(&self, device: Option<&str>) -> String {
        resolve_device_id(device)
    }

    fn open_device_with_policy(
        &self,
        device_id: &str,
        policy: DeviceOpenPolicy,
    ) -> Result<Box<dyn DeviceSession>> {
        Ok(Box::new(open_device_with_policy(device_id, policy)?))
    }
}

/// Concrete session enum (avoids dyn lifetime issues in callers).
pub enum AnySession {
    Mock(MockDeviceSession),
    File(FileDeviceSession),
    Wia(wia::WiaDeviceSession),
    Sane(sane::SaneDeviceSession),
    Escl(escl::EsclDeviceSession),
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
        _ if id.starts_with("wia:") || id == "wia" => Some(AnySession::Wia(wia::open(id)?)),
        _ if id.starts_with("sane:") || id == "sane" => Some(AnySession::Sane(sane::open(id)?)),
        _ if id.starts_with("escl:") || id == "escl" => {
            let session = if policy.allow_unlisted_escl && id.starts_with("escl:") {
                escl::open_explicit_id(id)?
            } else {
                escl::open(id)?
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
        "wia" => Some(AnySession::Wia(wia::open(&device.id)?)),
        "sane" => Some(AnySession::Sane(sane::open(&device.id)?)),
        "escl" => Some(AnySession::Escl(escl::open(&device.id)?)),
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
    backends.push(wia::backend_info_with_cancellation(cancellation));
    backends.push(sane::backend_info_with_cancellation(cancellation));
    backends.push(escl::backend_info());
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
    let escl =
        std::panic::catch_unwind(|| escl::list_escl_devices_safe_with_cancellation(cancellation))
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

    let wia =
        std::panic::catch_unwind(|| wia::list_wia_devices_safe_with_cancellation(cancellation))
            .unwrap_or_default();
    devices.extend(wia);

    let sane =
        std::panic::catch_unwind(|| sane::list_sane_devices_safe_with_cancellation(cancellation))
            .unwrap_or_default();
    devices.extend(sane);

    devices.extend(escl);

    devices
}

/// Re-list local and network scanners. A refresh performs exactly one bounded
/// eSCL discovery; the following aggregate listing reuses that result.
pub fn find_scanners(refresh: bool) -> Vec<DeviceInfo> {
    if refresh {
        return list_all_devices_with_escl(escl::refresh_devices());
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
