use super::super::normalize_image_ext;
use super::super::state::GuiState;
use crate::domain::export::OcrEngine;
use crate::domain::settings::validate_output_name;
use crate::error::Result;
use crate::workflows::capture::batch::{plan_batch_outputs, BatchOutputRequest, BatchScanArgs};
use std::path::PathBuf;

/// Immutable OCR request assembled on the UI thread before a worker starts.
#[cfg(feature = "gui")]
#[derive(Debug, Clone)]
pub(in crate::inbound::gui) struct OcrAction {
    pub(in crate::inbound::gui) src: PathBuf,
    pub(in crate::inbound::gui) language: String,
    pub(in crate::inbound::gui) engine: OcrEngine,
}

#[cfg_attr(all(test, not(feature = "gui")), allow(dead_code))]
impl GuiState {
    /// Validate and snapshot an OCR request without performing any OCR.
    #[cfg(feature = "gui")]
    pub(in super::super) fn prepare_ocr(&mut self) -> Option<OcrAction> {
        let Some(src) = self.last_image.clone() else {
            self.set_open_error();
            return None;
        };
        let (language, engine) = self.ocr_options();
        Some(OcrAction {
            src,
            language: language.into(),
            engine,
        })
    }

    pub(in super::super) fn batch_args(
        &self,
        out_dir: PathBuf,
        pages: u32,
    ) -> Result<BatchScanArgs> {
        let output_name = validate_output_name(&self.output_name)?;
        let format = normalize_image_ext(&self.output_fmt, "png");
        if format == "jxl" {
            return Err(crate::error::ScanError::Invalid(
                "GUI batch pages must use a built-in raster format; JPEG XL is available for single-image scan, reprocess, and save".into(),
            ));
        }
        let plan = plan_batch_outputs(BatchOutputRequest {
            out_dir: &out_dir,
            output_name,
            configured_format: &format,
            want_multipage: self.multipage,
            multipage_format: &self.multipage_format,
            want_contact_sheet: self.contact_sheet,
        });
        Ok(BatchScanArgs {
            device: self.device.clone(),
            out_dir: out_dir.clone(),
            pages,
            width: self.width,
            height: self.height,
            seed: 1,
            dpi: self.dpi,
            mode: self.scan_mode(),
            duplex: self.duplex,
            format: plan.page_format,
            multipage_tiff: None,
            multipage_pdf: None,
            multipage_out: plan.multipage_out,
            contact_sheet: plan.contact_sheet,
            pipeline: self.pipeline_prefs(),
            on_progress: None,
        })
    }

    pub(in super::super) fn ocr_options(&self) -> (&str, OcrEngine) {
        let language = self.ocr_language.trim();
        (
            if language.is_empty() { "eng" } else { language },
            self.selected_ocr_engine(),
        )
    }
}

#[cfg(all(test, feature = "gui"))]
mod tests {
    use super::*;

    #[test]
    fn prepare_ocr_preserves_the_ocrs_engine_choice() {
        let mut state = GuiState::new(None);
        state.last_image = Some(PathBuf::from("source.png"));
        state.ocr_engine = "ocrs".into();

        let action = state.prepare_ocr().expect("OCR action");

        assert_eq!(action.engine, OcrEngine::Ocrs);
    }
}
