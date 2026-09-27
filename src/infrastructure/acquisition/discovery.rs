//! Cancellation-aware maintenance inspection for background discovery.
use super::{escl, open_device, sane, wia, AnySession};
use crate::error::{Result, ScanError};
use crate::workflows::operation::CancellationToken;
use crate::workflows::ports::acquisition::{DeviceMaintenanceCapabilities, DeviceSession};

/// Bind cancellation before any maintenance command starts, including opening probes.
pub fn maintenance_capabilities_with_cancellation(
    device_id: &str,
    token: &CancellationToken,
) -> Result<DeviceMaintenanceCapabilities> {
    if token.is_cancelled() {
        return Err(ScanError::Cancelled("discovery cancelled".into()));
    }
    let session = open_for_inspection(device_id, token)?;
    session.bind_cancellation(token.clone());
    let capabilities = session.maintenance_capabilities();
    session.close();
    if token.is_cancelled() {
        return Err(ScanError::Cancelled("discovery cancelled".into()));
    }
    Ok(capabilities)
}

fn open_for_inspection(id: &str, token: &CancellationToken) -> Result<AnySession> {
    let session = match id.split(':').next().unwrap_or(id) {
        "sane" => AnySession::Sane(sane::open_with_cancellation(id, Some(token))?),
        "wia" => AnySession::Wia(wia::open_with_cancellation(id, Some(token))?),
        "escl" => AnySession::Escl(escl::open_with_cancellation(id, Some(token))?),
        "file" | "mock" => open_device(id)?,
        _ if id.starts_with("mock") => open_device(id)?,
        _ => return Err(ScanError::DeviceNotFound(id.into())),
    };
    Ok(session)
}
