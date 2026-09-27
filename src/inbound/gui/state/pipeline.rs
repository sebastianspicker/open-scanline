use super::*;

impl GuiState {
    pub(in crate::inbound::gui) fn pipeline_prefs(&self) -> PipelinePrefs {
        let crop = parse_crop_string(&self.crop_text)
            .ok()
            .flatten()
            .and_then(|value| {
                (value[2] > 0 && value[3] > 0)
                    .then(|| Rect::new(value[0], value[1], value[2] as u32, value[3] as u32))
            });
        PipelinePrefs {
            rotate: Rotate::from_degrees(self.rotate),
            flip_h: self.flip_h,
            flip_v: self.flip_v,
            crop,
            brightness: self.brightness,
            contrast: self.contrast,
            saturation: self.saturation,
            hue: self.hue,
            curves: parse_curve_points(&self.curves_text).ok().flatten(),
            desaturate: self.desaturate,
            levels_black: self.levels_black,
            levels_white: self.levels_white,
            levels_gamma: self.levels_gamma,
            auto_deskew: self.auto_deskew,
            deskew_angle: self.deskew_angle,
            auto_orient: self.auto_orient,
            auto_crop: self.auto_crop,
            white_balance: self.white_balance,
            sharpen_amount: self.sharpen_amount,
            invert: self.invert_colors,
            auto_levels: self.auto_levels,
            infrared_clean: disabled_tier_or(&self.infrared_clean),
            descreen: self.descreen,
            descreen_dpi: self.descreen_dpi,
            restore_colors: self.restore_colors,
            restore_fading: self.restore_fading,
            grain_reduction: disabled_tier_or(&self.grain_reduction),
            flatten: self.flatten,
            hole_punch: self.hole_punch,
            colorize_mode: disabled_value_or(&self.colorize_mode),
            film_type: (!self.film_type.trim().is_empty()).then(|| self.film_type.clone()),
        }
    }

    pub(in crate::inbound::gui) fn validate_color_controls(&self) -> Result<()> {
        validate_saturation(self.saturation)?;
        validate_hue(self.hue)?;
        parse_curve_points(&self.curves_text)?;
        Ok(())
    }

    pub(in crate::inbound::gui) fn scan_mode(&self) -> ScanMode {
        match self.media.as_str() {
            "adf" | "document" => ScanMode::Document,
            "transparency" | "film" => ScanMode::Film,
            _ => ScanMode::Reflective,
        }
    }
}
