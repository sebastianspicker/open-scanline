//! Scanner-session contract implemented by concrete acquisition adapters.

use crate::domain::acquisition::{validate_page_limit, ScanRequest};
use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use crate::operation::CancellationToken;
use serde::{Deserialize, Serialize};

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
            crate::domain::manufacturers::resolve_manufacturer(&device.name).manufacturer
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

/// Whether a device can perform a maintenance operation without guessing at
/// undocumented driver behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaintenanceAvailability {
    Supported,
    Simulated,
    Unsupported { reason: String },
}

impl MaintenanceAvailability {
    pub fn is_available(&self) -> bool {
        !matches!(self, Self::Unsupported { .. })
    }

    pub fn explanation(&self) -> &str {
        match self {
            Self::Supported => "supported by this scanner",
            Self::Simulated => "simulated by this test backend",
            Self::Unsupported { reason } => reason,
        }
    }
}

/// Focus controls advertised by a scanner. Point focus is only exposed when
/// both device-coordinate ranges were explicitly reported by the backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FocusCapability {
    Unsupported {
        reason: String,
    },
    Center {
        availability: MaintenanceAvailability,
    },
    Point {
        availability: MaintenanceAvailability,
        x_range: (f64, f64),
        y_range: (f64, f64),
    },
}

impl FocusCapability {
    pub fn is_available(&self) -> bool {
        match self {
            Self::Unsupported { .. } => false,
            Self::Center { availability } | Self::Point { availability, .. } => {
                availability.is_available()
            }
        }
    }

    pub fn supports_point(&self) -> bool {
        matches!(self, Self::Point { availability, .. } if availability.is_available())
    }

    pub fn explanation(&self) -> &str {
        match self {
            Self::Unsupported { reason } => reason,
            Self::Center { availability } | Self::Point { availability, .. } => {
                availability.explanation()
            }
        }
    }
}

/// Device-specific maintenance capability snapshot. Discovery is deliberately
/// separate from execution so inbound adapters can disable unavailable
/// controls instead of attempting a best-effort operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceMaintenanceCapabilities {
    pub calibration: MaintenanceAvailability,
    pub focus: FocusCapability,
}

impl DeviceMaintenanceCapabilities {
    pub fn unsupported(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            calibration: MaintenanceAvailability::Unsupported {
                reason: reason.clone(),
            },
            focus: FocusCapability::Unsupported { reason },
        }
    }

    pub fn simulated_point_focus() -> Self {
        Self {
            calibration: MaintenanceAvailability::Simulated,
            focus: FocusCapability::Point {
                availability: MaintenanceAvailability::Simulated,
                x_range: (0.0, 1.0),
                y_range: (0.0, 1.0),
            },
        }
    }
}

/// Why a bounded multi-page acquisition stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScanPagesEnd {
    LimitReached,
    FeederExhausted,
}

/// Summary returned by a streaming multi-page acquisition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanPagesResult {
    pub emitted: u32,
    pub end: ScanPagesEnd,
}

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

pub trait DeviceSession: Send {
    fn scan(&self, request: &ScanRequest) -> Result<ImageBuffer>;

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
    fn bind_cancellation(&self, _token: CancellationToken) {}
    fn cancel(&self) {}
    fn close(&self) {}
    fn maintenance_capabilities(&self) -> DeviceMaintenanceCapabilities {
        DeviceMaintenanceCapabilities::unsupported("maintenance is not supported by this backend")
    }
    fn calibrate(&self) -> serde_json::Value {
        serde_json::json!({"ok": false, "status": "unsupported"})
    }
    fn focus(&self, x: f64, y: f64) -> serde_json::Value {
        serde_json::json!({
            "ok": false,
            "status": "unsupported",
            "x_frac": x,
            "y_frac": y,
        })
    }
}

/// The `{"ok": false, "status": "closed", "backend": ...}` envelope every
/// adapter's `calibrate`/`focus`/maintenance path returns for a closed session.
pub(crate) fn closed_session_envelope(backend: &str) -> serde_json::Value {
    serde_json::json!({"ok": false, "status": "closed", "backend": backend})
}

/// Deterministic simulated point-focus score shared by the mock backend and
/// WIA's simulated devices: clamp `x`/`y` to `[0, 1]` and score their distance
/// from the frame center.
///
/// Returns `(x_fraction, y_fraction, focus)`.
pub(crate) fn simulated_focus_score(x: f64, y: f64) -> (f64, f64, f64) {
    let x_fraction = x.clamp(0.0, 1.0);
    let y_fraction = y.clamp(0.0, 1.0);
    let distance = ((x_fraction - 0.5).powi(2) + (y_fraction - 0.5).powi(2)).sqrt();
    let focus = ((1.0 - (distance * 1.4).min(1.0)) * 10000.0).round() / 10000.0;
    (x_fraction, y_fraction, focus)
}
