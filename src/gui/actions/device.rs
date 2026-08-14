use super::super::state::GuiState;
use crate::core::{ImageBuffer, PixelFormat, Result, ScanRequest};
use crate::device::{calibrate_device, find_scanners, focus_device};
use crate::icc::{make_it8_target_image, profile_scanner_it8, save_profile_json};
use crate::imaging::load_image;
use serde_json::Value;
use std::path::PathBuf;

#[cfg_attr(all(test, not(feature = "gui")), allow(dead_code))]
impl GuiState {
    pub(in super::super) fn do_calibrate(&mut self) {
        self.set_device_result("calibrate", calibrate_device(&self.device));
    }

    pub(in super::super) fn do_focus(&mut self) {
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
        let value = crate::device::exposure_from_preview(&self.device, &request);
        self.status = format!(
            "exposure {}: {}",
            self.device,
            serde_json::to_string(&value).unwrap_or_else(|_| value.to_string())
        );
    }

    pub(in super::super) fn do_profile_scanner(&mut self) {
        match self.create_scanner_profile() {
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
        self.devices = find_scanners(true)
            .into_iter()
            .map(|device| device.id)
            .collect();
        if !self.devices.iter().any(|device| device == &self.device) {
            self.device = self
                .devices
                .first()
                .cloned()
                .unwrap_or_else(|| "mock".into());
        }
        self.status = format!("{}: {}", self.translator.t("devices"), self.devices.len());
    }

    fn create_scanner_profile(&self) -> Result<(PathBuf, Value)> {
        let image = self.scanner_profile_image()?;
        let profile = profile_scanner_it8(&image)?;
        let dest = PathBuf::from(&self.output_dir).join("scanner_it8_profile.json");
        save_profile_json(&dest, &profile)?;
        Ok((dest, profile))
    }

    fn scanner_profile_image(&self) -> Result<ImageBuffer> {
        match &self.last_image {
            Some(path) => load_image(path),
            None => make_it8_target_image(120, 80),
        }
    }

    fn set_device_result(&mut self, action: &str, value: Value) {
        let outcome = value
            .get("status")
            .and_then(|status| status.as_str())
            .or_else(|| {
                value.get("ok").map(|ok| {
                    if ok.as_bool() == Some(true) {
                        "ok"
                    } else {
                        "error"
                    }
                })
            })
            .unwrap_or("done");
        self.status = format!("{action} {}: {outcome}", self.device);
    }
}
