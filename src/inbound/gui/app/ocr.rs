#[cfg(feature = "gui")]
use super::*;

#[cfg(feature = "gui")]
impl OpenScanlineApp {
    pub(in crate::inbound::gui) fn start_ocr(&mut self) {
        if self.job_active() || self.closing {
            return;
        }
        let Some(action) = self.state.prepare_ocr() else {
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let token = CancellationToken::from_arc(Arc::clone(&cancel));
        self.start_file_job(cancel, None, "status.processing", move || {
            let result = load_image(&action.src).and_then(|image| {
                ocr_image_with_engine_with_cancellation(
                    &image,
                    &action.language,
                    action.engine,
                    Some(&token),
                )
            });
            match result {
                Ok(result) => GuiJobEvent::OcrFinished {
                    engine: result.engine,
                    text: result.text,
                },
                Err(ScanError::Cancelled(_)) => GuiJobEvent::Cancelled,
                Err(error) => GuiJobEvent::Failed(error.to_string()),
            }
        });
    }
}
