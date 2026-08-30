//! Acquisition contracts shared by workflows and concrete scanner adapters.

use crate::domain::acquisition::{DeviceOpenPolicy, ScanRequest};
use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use crate::workflows::operation::CancellationToken;
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

/// Upper bound for one acquisition job.
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

/// Scanner-session boundary required by capture workflows.
///
/// Backend discovery and device maintenance are inbound concerns. Capture only
/// needs to resolve a requested device and open a session for it.
pub trait AcquisitionPort: Send + Sync {
    fn resolve_device_id(&self, device: Option<&str>) -> String;

    fn open_device_with_policy(
        &self,
        device_id: &str,
        policy: DeviceOpenPolicy,
    ) -> Result<Box<dyn DeviceSession>>;
}
