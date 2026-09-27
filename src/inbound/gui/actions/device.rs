use super::super::state::GuiState;
use crate::domain::acquisition::ScanRequest;
use crate::domain::image::PixelFormat;
use crate::workflows::maintenance::{
    calibrate_device, create_scanner_profile, exposure_from_preview, focus_device,
};
use serde_json::Value;
use std::path::Path;

#[cfg_attr(all(test, not(feature = "gui")), allow(dead_code))]
impl GuiState {
    pub(in super::super) fn do_calibrate(&mut self) {
        if !self.calibration_available() {
            self.status = format!(
                "calibrate {}: unsupported ({})",
                self.device,
                self.calibration_explanation()
            );
            return;
        }
        self.set_device_result("calibrate", calibrate_device(&self.device));
    }

    pub(in super::super) fn do_focus(&mut self) {
        if !self.focus_available() {
            self.status = format!(
                "focus {}: unsupported ({})",
                self.device,
                self.focus_explanation()
            );
            return;
        }
        self.set_device_result("focus", focus_device(&self.device, 0.5, 0.5));
    }

    pub(in super::super) fn do_exposure(&mut self) {
        let request = ScanRequest {
            device_id: self.device.clone(),
            width: self.width.min(160),
            height: self.height.min(120),
            dpi_x: self.dpi,
            dpi_y: self.dpi,
            seed: 1,
            pixel_format: PixelFormat::Rgb8,
            ..Default::default()
        };
        let value = exposure_from_preview(&self.device, &request);
        self.status = format!(
            "exposure {}: {}",
            self.device,
            serde_json::to_string(&value).unwrap_or_else(|_| value.to_string())
        );
    }

    pub(in super::super) fn do_profile_scanner(&mut self) {
        match create_scanner_profile(self.last_image.as_deref(), Path::new(&self.output_dir)) {
            Ok((dest, profile)) => {
                self.scanner_profile_path = dest.display().to_string();
                self.status = format!(
                    "profile scanner IT8 → {} ok={}",
                    dest.display(),
                    profile
                        .get("ok")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
                );
            }
            Err(error) => self.set_error(error),
        }
    }

    pub(in super::super) fn refresh_devices(&mut self) {
        self.discovery_refresh = true;
        self.refresh_maintenance_capabilities();
    }

    fn set_device_result(&mut self, action: &str, value: Value) {
        let outcome = value
            .get("status")
            .and_then(|status| status.as_str())
            .or_else(|| value.get("ok").map(device_action_outcome))
            .unwrap_or("done");
        self.status = format!("{action} {}: {outcome}", self.device);
    }
}

fn device_action_outcome(ok: &Value) -> &'static str {
    if ok.as_bool() == Some(true) {
        "ok"
    } else {
        "error"
    }
}
