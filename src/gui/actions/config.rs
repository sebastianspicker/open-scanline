use super::super::parse_crop_string;
use super::super::state::GuiState;
use crate::config::{parse_curve_points, save_config};

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
        let crop = match parse_crop_string(&self.crop_text) {
            Ok(Some(crop)) if crop[2] > 0 && crop[3] > 0 => Some(crop),
            Ok(None) => None,
            Ok(Some(_)) => {
                self.set_error("crop width and height must be positive");
                return;
            }
            Err(error) => {
                self.set_error(format!("invalid crop: {error}"));
                return;
            }
        };
        let curves = match parse_curve_points(&self.curves_text) {
            Ok(curves) => curves,
            Err(error) => {
                self.set_error(format!("invalid curves: {error}"));
                return;
            }
        };
        if let Err(error) = self.validate_color_controls() {
            self.set_error(format!("invalid color controls: {error}"));
            return;
        }
        let mut config = self.config.clone();
        config.last_device_id = self.device.clone();
        config.default_dpi = self.dpi;
        config.default_width = self.width;
        config.default_height = self.height;
        config.scan_mode = self.scan_mode();
        config.duplex = self.duplex;
        config.output_dir = self.output_dir.clone();
        config.language = self.language.clone();
        config.rotate = self.rotate;
        config.flip_h = self.flip_h;
        config.flip_v = self.flip_v;
        config.crop = crop;
        config.brightness = self.brightness;
        config.contrast = self.contrast;
        config.saturation = self.saturation;
        config.hue = self.hue;
        config.curves = curves;
        config.desaturate = self.desaturate;
        config.levels_black = self.levels_black;
        config.levels_white = self.levels_white;
        config.levels_gamma = self.levels_gamma;
        config.invert_colors = self.invert_colors;
        config.auto_deskew = self.auto_deskew;
        config.deskew_angle = self.deskew_angle;
        config.auto_orient = self.auto_orient;
        config.auto_crop = self.auto_crop;
        config.white_balance = self.white_balance;
        config.auto_levels = self.auto_levels;
        config.sharpen_amount = self.sharpen_amount;
        config.infrared_clean = (self.infrared_clean != "off").then(|| self.infrared_clean.clone());
        config.descreen = self.descreen;
        config.descreen_dpi = self.descreen_dpi;
        config.restore_colors = self.restore_colors;
        config.restore_fading = self.restore_fading;
        config.grain_reduction =
            (self.grain_reduction != "off").then(|| self.grain_reduction.clone());
        config.flatten = self.flatten;
        config.hole_punch = self.hole_punch;
        config.colorize_mode = (self.colorize_mode != "off").then(|| self.colorize_mode.clone());
        config.film_type = (!self.film_type.trim().is_empty()).then(|| self.film_type.clone());
        config.batch_pages = self.batch_pages.max(1);
        config.output_name = self.output_name.clone();
        config.output_format = self.output_fmt.clone();
        config.multipage = self.multipage;
        config.multipage_format = self.multipage_format.clone();
        config.save_raw = self.save_raw;
        config.contact_sheet = self.contact_sheet;
        config.ocr_engine = self.ocr_engine.clone();
        config.ocr_language = self.ocr_language.clone();
        match save_config(&config, &self.config_path) {
            Ok(_) => {
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

    /// Explicitly allow the GUI defaults to replace a config that could not
    /// be loaded. The original bytes remain untouched until Save Config.
    pub(in super::super) fn do_reset_config_recovery(&mut self) {
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
