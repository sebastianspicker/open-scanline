use crate::domain::acquisition::ScanRequest;
use crate::domain::acquisition::{apply_flat_dark_cal, synthetic_cal_tables};
use crate::domain::image::{ImageBuffer, PixelFormat};
use crate::error::{Result, ScanError};
use crate::workflows::ports::acquisition::{
    reject_single_page_duplex, DeviceInfo, DeviceMaintenanceCapabilities, DeviceSession,
};
use std::sync::Mutex;

/// Complete synthetic device backend — always available for CI/offline.
pub struct MockDevice;

impl MockDevice {
    pub const DEVICE_ID: &'static str = "mock";
    pub const NAME: &'static str = "open-scanline Synthetic Scanner";

    pub fn list_devices() -> Vec<DeviceInfo> {
        vec![DeviceInfo::new(Self::DEVICE_ID, Self::NAME, "mock")]
    }
}

pub struct MockDeviceSession {
    closed: Mutex<bool>,
    cancelled: Mutex<bool>,
    cal: Mutex<Option<(Vec<i32>, Vec<f64>)>>,
    focus_pt: Mutex<Option<(f64, f64)>>,
}

impl MockDeviceSession {
    pub fn new() -> Self {
        Self {
            closed: Mutex::new(false),
            cancelled: Mutex::new(false),
            cal: Mutex::new(None),
            focus_pt: Mutex::new(None),
        }
    }

    /// Deterministic RGB8 gradient matching the product contract:
    /// r = x*255/max(w-1,1), g = y*255/max(h-1,1), b = seed & 0xFF
    pub fn gradient(request: &ScanRequest) -> Result<ImageBuffer> {
        validate_gradient_request(request)?;
        let image = gradient_image(request)?;
        match request.region {
            Some(region) => crate::domain::processing::crop(&image, region),
            None => Ok(image),
        }
    }
}

impl Default for MockDeviceSession {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceSession for MockDeviceSession {
    fn maintenance_capabilities(&self) -> DeviceMaintenanceCapabilities {
        DeviceMaintenanceCapabilities::simulated_point_focus()
    }

    fn scan(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        reject_single_page_duplex(request)?;
        super::batch::ensure_session_ready(&self.closed, &self.cancelled)?;
        let mut image = Self::gradient(request)?;
        if let Some((ref dark, ref flat)) =
            *self.cal.lock().unwrap_or_else(|error| error.into_inner())
        {
            image = apply_flat_dark_cal(&image, dark, flat)?;
        }
        Ok(image)
    }

    fn cancel(&self) {
        if let Ok(mut cancelled) = self.cancelled.lock() {
            *cancelled = true;
        }
    }

    fn close(&self) {
        if let Ok(mut closed) = self.closed.lock() {
            *closed = true;
        }
    }

    fn calibrate(&self) -> serde_json::Value {
        use serde_json::json;
        if *self
            .closed
            .lock()
            .unwrap_or_else(|error| error.into_inner())
        {
            return json!({"ok": false, "status": "closed", "backend": "mock"});
        }
        let (dark, flat) = synthetic_cal_tables();
        if let Ok(mut calibration) = self.cal.lock() {
            *calibration = Some((dark.clone(), flat.clone()));
        }
        json!({
            "ok": true,
            "status": "calibrated",
            "backend": "mock",
            "dark_len": dark.len(),
            "flat_len": flat.len(),
        })
    }

    fn focus(&self, x: f64, y: f64) -> serde_json::Value {
        use serde_json::json;
        if *self
            .closed
            .lock()
            .unwrap_or_else(|error| error.into_inner())
        {
            return json!({"ok": false, "status": "closed", "backend": "mock"});
        }
        let x_fraction = x.clamp(0.0, 1.0);
        let y_fraction = y.clamp(0.0, 1.0);
        if let Ok(mut focus_point) = self.focus_pt.lock() {
            *focus_point = Some((x_fraction, y_fraction));
        }
        let distance = ((x_fraction - 0.5).powi(2) + (y_fraction - 0.5).powi(2)).sqrt();
        let value = ((1.0 - (distance * 1.4).min(1.0)) * 10000.0).round() / 10000.0;
        json!({
            "ok": true,
            "status": "focused",
            "x_frac": x_fraction,
            "y_frac": y_fraction,
            "focus": value,
            "backend": "mock",
        })
    }
}

fn validate_gradient_request(request: &ScanRequest) -> Result<()> {
    if request.pixel_format != PixelFormat::Rgb8 {
        return Err(ScanError::Unsupported(
            "MockDevice only supports Rgb8".into(),
        ));
    }
    validate_gradient_dimensions(request.width, request.height)
}

fn validate_gradient_dimensions(width: u32, height: u32) -> Result<()> {
    if width < 1 || height < 1 {
        return Err(ScanError::Invalid("invalid dimensions".into()));
    }
    if width > 8192 || height > 8192 {
        return Err(ScanError::Invalid(
            "dimensions exceed mock max 8192x8192".into(),
        ));
    }
    Ok(())
}

fn gradient_image(request: &ScanRequest) -> Result<ImageBuffer> {
    let (width, height) = (request.width, request.height);
    let mut buffer = vec![0_u8; (width as usize) * (height as usize) * 3];
    let seed = (request.seed & 0xFF) as u8;
    for (index, pixel) in buffer.chunks_exact_mut(3).enumerate() {
        let x = index as u32 % width;
        let y = index as u32 / width;
        pixel.copy_from_slice(&gradient_pixel(x, y, width, height, seed));
    }
    ImageBuffer::new(width, height, PixelFormat::Rgb8, buffer)
}

fn gradient_pixel(x: u32, y: u32, width: u32, height: u32, seed: u8) -> [u8; 3] {
    let red = ((x * 255) / width.saturating_sub(1).max(1)) as u8;
    let green = ((y * 255) / height.saturating_sub(1).max(1)) as u8;
    [red, green, seed]
}
