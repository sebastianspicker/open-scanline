use super::*;

impl GuiState {
    pub(in crate::inbound::gui) fn out_path(&self, suffix: &str) -> Result<PathBuf> {
        let output_name = validate_output_name(&self.output_name)?;
        let ext = normalize_image_ext(&self.output_fmt, "png");
        Ok(PathBuf::from(&self.output_dir).join(format!(
            "{output_name}_{suffix}_{:03}.{ext}",
            self.frame_index
        )))
    }

    pub(in crate::inbound::gui) fn out_path_plain(&self, suffix: &str) -> Result<PathBuf> {
        let output_name = validate_output_name(&self.output_name)?;
        let ext = normalize_image_ext(&self.output_fmt, "png");
        Ok(PathBuf::from(&self.output_dir).join(format!("{output_name}_{suffix}.{ext}")))
    }
}
