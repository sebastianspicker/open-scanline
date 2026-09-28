use super::*;

impl GuiState {
    pub(in crate::inbound::gui) fn refresh_maintenance_capabilities(&mut self) {
        self.maintenance_capabilities =
            DeviceMaintenanceCapabilities::unsupported("Loading maintenance capabilities");
        self.discovery_requested = true;
        self.discovery_loading = true;
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
