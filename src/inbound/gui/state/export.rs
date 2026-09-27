use super::*;

impl GuiState {
    pub(in crate::inbound::gui) fn take_export_options(&mut self) -> ExportOptions {
        let mut export = self.export_options();
        export.pdf_password =
            (!self.pdf_password.is_empty()).then(|| std::mem::take(&mut self.pdf_password));
        export
    }

    /// Restore a password consumed for a failed export only while the user has
    /// not supplied a replacement in the meantime.
    pub(in crate::inbound::gui) fn restore_export_password(&mut self, password: Option<String>) {
        if self.pdf_password.is_empty() {
            if let Some(password) = password.filter(|password| !password.is_empty()) {
                self.pdf_password = password;
            }
        }
    }

    pub(in crate::inbound::gui) fn export_options(&self) -> ExportOptions {
        ExportOptions {
            pdf_password: (!self.pdf_password.is_empty()).then(|| self.pdf_password.clone()),
            searchable_pdf: self.searchable_pdf,
            ocr_language: nonempty_or(&self.ocr_language, "eng"),
            ocr_engine: self.selected_ocr_engine(),
            scanner_profile: (!self.scanner_profile_path.trim().is_empty())
                .then(|| PathBuf::from(self.scanner_profile_path.trim())),
        }
    }

    pub(in crate::inbound::gui) fn selected_ocr_engine(&self) -> OcrEngine {
        match self.ocr_engine.as_str() {
            "ocrs" => OcrEngine::Ocrs,
            "tesseract" => OcrEngine::Tesseract,
            _ => OcrEngine::Offline,
        }
    }
}
