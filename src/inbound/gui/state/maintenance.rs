use super::*;

impl GuiState {
    pub(in crate::inbound::gui) fn refresh_maintenance_capabilities(&mut self) {
        self.maintenance_capabilities =
            DeviceMaintenanceCapabilities::unsupported("Loading maintenance capabilities");
        self.discovery_requested = true;
        self.discovery_loading = true;
    }

    /// The selected scanner is absent from a finished discovery (e.g. a saved device that is unplugged).
    pub(in crate::inbound::gui) fn device_missing(&self) -> bool {
        !self.discovery_loading
            && !self.device.trim().is_empty()
            && !self.devices.contains(&self.device)
    }

    /// Combo label for a device, flagging a selected scanner that discovery did not list.
    pub(in crate::inbound::gui) fn device_label(&self, device: &str) -> String {
        if device == self.device && self.device_missing() {
            self.translator
                .t_args("ws.device_not_found", &[("device", device)])
        } else {
            device.to_owned()
        }
    }

    pub(in crate::inbound::gui) fn calibration_available(&self) -> bool {
        self.maintenance_capabilities.calibration.is_available()
    }

    pub(in crate::inbound::gui) fn focus_available(&self) -> bool {
        self.maintenance_capabilities.focus.is_available()
    }

    pub(in crate::inbound::gui) fn calibration_explanation(&self) -> &str {
        self.maintenance_capabilities.calibration.explanation()
    }

    pub(in crate::inbound::gui) fn focus_explanation(&self) -> &str {
        self.maintenance_capabilities.focus.explanation()
    }
}
