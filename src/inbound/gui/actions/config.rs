use super::super::parse_crop_string;
use super::super::state::GuiState;
use crate::infrastructure::config::json::{parse_curve_points, save_config};

type SaveConfigValues = (Option<[i32; 4]>, Option<Vec<[i32; 2]>>);

macro_rules! apply_gui_config_fields {
    ($config:ident, $state:ident, $crop:ident, $curves:ident) => {{
        $config.last_device_id = $state.device.clone();
        $config.default_dpi = $state.dpi;
        $config.default_width = $state.width;
        $config.default_height = $state.height;
        $config.scan_mode = $state.scan_mode();
        $config.duplex = $state.duplex;
        $config.output_dir = $state.output_dir.clone();
        $config.language = $state.language.clone();
        $config.rotate = $state.rotate;
        $config.flip_h = $state.flip_h;
        $config.flip_v = $state.flip_v;
        $config.crop = $crop;
        $config.brightness = $state.brightness;
        $config.contrast = $state.contrast;
        $config.saturation = $state.saturation;
        $config.hue = $state.hue;
        $config.curves = $curves;
        $config.desaturate = $state.desaturate;
        $config.levels_black = $state.levels_black;
        $config.levels_white = $state.levels_white;
        $config.levels_gamma = $state.levels_gamma;
        $config.invert_colors = $state.invert_colors;
        $config.auto_deskew = $state.auto_deskew;
        $config.deskew_angle = $state.deskew_angle;
        $config.auto_orient = $state.auto_orient;
        $config.auto_crop = $state.auto_crop;
        $config.white_balance = $state.white_balance;
        $config.auto_levels = $state.auto_levels;
        $config.sharpen_amount = $state.sharpen_amount;
        $config.infrared_clean =
            ($state.infrared_clean != "off").then(|| $state.infrared_clean.clone());
        $config.descreen = $state.descreen;
        $config.descreen_dpi = $state.descreen_dpi;
        $config.restore_colors = $state.restore_colors;
        $config.restore_fading = $state.restore_fading;
        $config.grain_reduction =
            ($state.grain_reduction != "off").then(|| $state.grain_reduction.clone());
        $config.flatten = $state.flatten;
        $config.hole_punch = $state.hole_punch;
        $config.colorize_mode =
            ($state.colorize_mode != "off").then(|| $state.colorize_mode.clone());
        $config.film_type = (!$state.film_type.trim().is_empty()).then(|| $state.film_type.clone());
        $config.batch_pages = $state.batch_pages.max(1);
        $config.output_name = $state.output_name.clone();
        $config.output_format = $state.output_fmt.clone();
        $config.multipage = $state.multipage;
        $config.multipage_format = $state.multipage_format.clone();
        $config.save_raw = $state.save_raw;
        $config.contact_sheet = $state.contact_sheet;
        $config.ocr_engine = $state.ocr_engine.clone();
        $config.ocr_language = $state.ocr_language.clone();
    }};
}

#[cfg_attr(all(test, not(feature = "gui")), allow(dead_code))]
impl GuiState {
    pub(in super::super) fn do_save_config(&mut self) {
        if self.config_recovery_required {
            let error = self.config_load_error.as_deref().unwrap_or("unknown error");
            self.set_error(format!(
                "configuration was not saved because loading {} failed: {error}. Reset configuration before replacing it",
                self.config_path.display()
            ));
            return;
        }
        let Some((crop, curves)) = self.config_values_for_save() else {
            return;
        };
        let config = self.config_for_save(crop, curves);
        match save_config(&config, &self.config_path) {
            Ok(_) => {
                self.error_message = None;
                self.config = config;
                self.status = format!(
                    "{} → {}",
                    self.translator.t("save_config"),
                    self.config_path.display()
                )
            }
            Err(error) => self.set_error(error),
        }
    }

    fn config_values_for_save(&mut self) -> Option<SaveConfigValues> {
        let crop = match parse_crop_string(&self.crop_text) {
            Ok(Some(crop)) if crop[2] > 0 && crop[3] > 0 => Some(crop),
            Ok(None) => None,
            Ok(Some(_)) => return self.save_config_error("crop width and height must be positive"),
            Err(error) => return self.save_config_error(format!("invalid crop: {error}")),
        };
        let curves = match parse_curve_points(&self.curves_text) {
            Ok(curves) => curves,
            Err(error) => return self.save_config_error(format!("invalid curves: {error}")),
        };
        self.validate_color_controls()
            .map_err(|error| format!("invalid color controls: {error}"))
            .map_or_else(
                |error| self.save_config_error(error),
                |_| Some((crop, curves)),
            )
    }

    fn config_for_save(
        &self,
        crop: Option<[i32; 4]>,
        curves: Option<Vec<[i32; 2]>>,
    ) -> crate::workflows::settings::AppConfig {
        let mut config = self.config.clone();
        apply_gui_config_fields!(config, self, crop, curves);
        config
    }

    fn save_config_error<T>(&mut self, error: impl Into<String>) -> Option<T> {
        self.set_error(error.into());
        None
    }

    /// Explicitly allow the GUI defaults to replace a config that could not
    /// be loaded. The original bytes remain untouched until Save Config.
    pub(in super::super) fn do_reset_config_recovery(&mut self) {
        self.error_message = None;
        if !self.config_recovery_required {
            self.status = "configuration recovery is not needed".into();
            return;
        }
        self.config_recovery_required = false;
        self.config_load_error = None;
        self.status = format!(
            "configuration recovery enabled; Save Config will replace {}",
            self.config_path.display()
        );
    }
}
