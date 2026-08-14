use super::super::normalize_image_ext;
use super::super::state::GuiState;
use crate::batch::{run_batch_scan, BatchScanArgs};
use crate::config::validate_output_name;
use crate::core::Result;
#[cfg(any(test, not(feature = "gui")))]
use crate::ocr::ocr_file;
use std::path::PathBuf;

/// Immutable OCR request assembled on the UI thread before a worker starts.
#[derive(Debug, Clone)]
pub(in crate::gui) struct OcrAction {
    pub(in crate::gui) src: PathBuf,
    pub(in crate::gui) language: String,
    pub(in crate::gui) offline: bool,
}

#[cfg_attr(all(test, not(feature = "gui")), allow(dead_code))]
impl GuiState {
    #[allow(dead_code)]
    pub(in super::super) fn do_batch(&mut self) {
        let pages = self.batch_pages.max(1);
        let out_dir = PathBuf::from(&self.output_dir).join("batch");
        let args = match self.batch_args(out_dir.clone(), pages) {
            Ok(args) => args,
            Err(error) => {
                self.set_error(error);
                return;
            }
        };
        match run_batch_scan(args) {
            Ok(paths) => {
                if let Some(last) = paths.last() {
                    self.last_image = Some(last.clone());
                    self.update_histogram();
                }
                self.status = format!("{} {}", self.translator.t("batch.done"), paths.len());
            }
            Err(error) => self.set_error(error),
        }
    }

    /// Validate and snapshot an OCR request without performing any OCR.
    pub(in super::super) fn prepare_ocr(&mut self) -> Option<OcrAction> {
        let Some(src) = self.last_image.clone() else {
            self.set_open_error();
            return None;
        };
        let (language, offline) = self.ocr_options();
        Some(OcrAction {
            src,
            language: language.into(),
            offline,
        })
    }

    /// Synchronous test/non-GUI compatibility path. The desktop UI always
    /// uses `prepare_ocr` and runs this work through `OpenScanlineApp`.
    #[cfg(any(test, not(feature = "gui")))]
    pub(in super::super) fn do_ocr(&mut self) {
        let Some(action) = self.prepare_ocr() else {
            return;
        };
        match ocr_file(&action.src, &action.language, action.offline) {
            Ok(result) => {
                self.status = format!(
                    "{} ({}): {}",
                    self.translator.t("ocr"),
                    result.engine,
                    result.text
                )
            }
            Err(error) => self.set_error(error),
        }
    }

    pub(in super::super) fn batch_args(
        &self,
        out_dir: PathBuf,
        pages: u32,
    ) -> Result<BatchScanArgs> {
        let output_name = validate_output_name(&self.output_name)?;
        let format = normalize_image_ext(&self.output_fmt, "png");
        if format == "jxl" {
            return Err(crate::core::ScanError::Invalid(
                "GUI batch pages must use a built-in raster format; JPEG XL is available for single-image scan, reprocess, and save".into(),
            ));
        }
        let pdf_output = format == "pdf";
        let page_format = if pdf_output { "png".into() } else { format };
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
            format: page_format,
            multipage_tiff: None,
            multipage_pdf: None,
            multipage_out: (self.multipage || pdf_output).then(|| {
                let multipage_format = if pdf_output {
                    "pdf"
                } else {
                    &self.multipage_format
                };
                out_dir.join(format!("{}_multipage.{}", output_name, multipage_format))
            }),
            contact_sheet: self
                .contact_sheet
                .then(|| out_dir.join(format!("{output_name}_contact.bmp"))),
            pipeline: self.pipeline_prefs(),
            on_progress: None,
        })
    }

    pub(in super::super) fn ocr_options(&self) -> (&str, bool) {
        let language = self.ocr_language.trim();
        (
            if language.is_empty() { "eng" } else { language },
            self.ocr_engine == "offline",
        )
    }
}
