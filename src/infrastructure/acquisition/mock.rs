use crate::domain::acquisition::ScanRequest;
use crate::domain::acquisition::{apply_flat_dark_cal, synthetic_cal_tables};
use crate::domain::image::{ImageBuffer, PixelFormat};
use crate::error::{Result, ScanError};
use crate::workflows::ports::acquisition::{reject_single_page_duplex, DeviceInfo, DeviceSession};
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
        if request.pixel_format != PixelFormat::Rgb8 {
            return Err(ScanError::Unsupported(
                "MockDevice only supports Rgb8".into(),
            ));
        }
        let width = request.width;
        let height = request.height;
        if width < 1 || height < 1 {
            return Err(ScanError::Invalid("invalid dimensions".into()));
        }
        if width > 8192 || height > 8192 {
            return Err(ScanError::Invalid(
                "dimensions exceed mock max 8192x8192".into(),
            ));
        }
        let seed = (request.seed & 0xFF) as u8;
        let mut buffer = vec![0_u8; (width as usize) * (height as usize) * 3];
        let x_denominator = width.saturating_sub(1).max(1);
        let y_denominator = height.saturating_sub(1).max(1);
        let mut index = 0_usize;
        for y in 0..height {
            for x in 0..width {
                buffer[index] = ((x * 255) / x_denominator) as u8;
                buffer[index + 1] = ((y * 255) / y_denominator) as u8;
                buffer[index + 2] = seed;
                index += 3;
            }
        }
        let image = ImageBuffer::new(width, height, PixelFormat::Rgb8, buffer)?;
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
    fn scan(&self, request: &ScanRequest) -> Result<ImageBuffer> {
        reject_single_page_duplex(request)?;
        if *self
            .closed
            .lock()
            .unwrap_or_else(|error| error.into_inner())
        {
            return Err(ScanError::Other("session closed".into()));
        }
        if *self
            .cancelled
            .lock()
            .unwrap_or_else(|error| error.into_inner())
        {
            return Err(ScanError::Cancelled("scan cancelled".into()));
        }
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
