use super::*;

#[cfg_attr(all(test, not(feature = "gui")), allow(dead_code))]
impl GuiState {
    pub(in crate::inbound::gui) fn new(config_path: Option<&Path>) -> Self {
        let (path, config, lang, translator, config_load_error) =
            load_config_and_translation(config_path);
        let resolved = resolve_defaults(&config);
        let (devices, device) = devices_and_selection(&config);
        build_gui_state!(
            path,
            config,
            lang,
            translator,
            config_load_error,
            &resolved.acquisition,
            &resolved.processing,
            devices,
            device.clone(),
            DeviceMaintenanceCapabilities::unsupported("Loading maintenance capabilities"),
            configured_output_dir(&config)
        )
    }
}
