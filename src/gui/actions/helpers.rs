use super::super::histogram_for_path;
use super::super::state::GuiState;
use std::path::PathBuf;

#[cfg_attr(all(test, not(feature = "gui")), allow(dead_code))]
impl GuiState {
    pub(in crate::gui) fn set_image_success(&mut self, path: PathBuf) {
        self.last_image = Some(path.clone());
        self.status = format!("{} {}", self.translator.t("status.done"), path.display());
        self.update_histogram();
    }

    pub(in crate::gui) fn update_histogram(&mut self) {
        if let Some(path) = &self.last_image {
            if let Ok(histogram) = histogram_for_path(path) {
                self.hist_summary = format!(
                    "histogram count={}",
                    histogram["count"].as_u64().unwrap_or(0)
                );
            }
        }
    }

    pub(super) fn set_open_error(&mut self) {
        self.status = self.translator.t("error.open");
    }

    pub(in crate::gui) fn set_error(&mut self, error: impl std::fmt::Display) {
        self.status = self
            .translator
            .t_args("status.error", &[("msg", &error.to_string())]);
    }
}
