use super::super::state::GuiState;
use std::path::PathBuf;

#[cfg_attr(all(test, not(feature = "gui")), allow(dead_code))]
impl GuiState {
    pub(in crate::inbound::gui) fn set_image_success(&mut self, path: PathBuf) {
        self.error_message = None;
        self.last_image = Some(path.clone());
        self.status = format!("{} {}", self.translator.t("status.done"), path.display());
        self.update_histogram();
    }

    pub(in crate::inbound::gui) fn update_histogram(&mut self) {
        self.preview = None;
        self.hist_summary.clear();
        self.preview_revision = self.preview_revision.wrapping_add(1);
    }

    pub(super) fn set_open_error(&mut self) {
        self.error_message = Some(self.translator.t("error.open"));
        self.status = self.translator.t("error.open");
    }

    pub(in crate::inbound::gui) fn set_error(&mut self, error: impl std::fmt::Display) {
        self.error_message = Some(error.to_string());
        self.status = self
            .translator
            .t_args("status.error", &[("msg", &error.to_string())]);
    }
}
