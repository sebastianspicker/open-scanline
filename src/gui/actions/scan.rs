use super::super::state::GuiState;
use crate::scan::{run_scan_to_file, ScanToFileArgs};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[cfg_attr(all(test, not(feature = "gui")), allow(dead_code))]
impl GuiState {
    #[allow(dead_code)]
    pub(in super::super) fn do_scan(&mut self, preview: bool) {
        if let Err(error) = self.validate_color_controls() {
            self.set_error(format!("invalid color controls: {error}"));
            return;
        }
        self.cancel_requested.store(false, Ordering::SeqCst);
        self.scanning = true;
        let result = self.scan_args(preview).and_then(run_scan_to_file);
        self.scanning = false;
        match result {
            Ok(path) => self.set_image_success(path),
            Err(crate::core::ScanError::Cancelled(_)) => {
                self.status = self.translator.t("status.cancelled");
            }
            Err(error) => self.set_error(error),
        }
    }

    pub(in super::super) fn do_cancel(&mut self) {
        self.cancel_requested.store(true, Ordering::SeqCst);
        self.status = self.translator.t(if self.scanning {
            "status.cancelled"
        } else {
            "status.ready"
        });
    }

    pub(in crate::gui) fn scan_args(&self, preview: bool) -> crate::core::Result<ScanToFileArgs> {
        let out = self.scan_output_path(preview)?;
        let mut pipeline = self.pipeline_prefs();
        let invert_colors = self.invert_colors || pipeline.invert;
        pipeline.invert = false;
        let cancel_requested = Arc::clone(&self.cancel_requested);
        let (width, height) = self.scan_dimensions(preview);
        Ok(ScanToFileArgs {
            device: Some(self.device.clone()),
            out,
            width,
            height,
            seed: 1,
            dpi: self.dpi,
            mode: self.scan_mode(),
            duplex: self.duplex,
            pipeline,
            invert_colors,
            use_preview: preview,
            // GUI controls are the authoritative pipeline snapshot for this
            // scan. Passing the persisted config here would re-enable saved
            // effects that the user has since unchecked in the UI.
            config: None,
            on_progress: None,
            cancel_check: Some(Box::new(move || cancel_requested.load(Ordering::SeqCst))),
            raw_out: self.raw_output_path(preview)?,
        })
    }

    fn scan_output_path(&self, preview: bool) -> crate::core::Result<PathBuf> {
        if preview {
            let output_name = crate::config::validate_output_name(&self.output_name)?;
            Ok(PathBuf::from(&self.output_dir).join(format!("{output_name}_preview.png")))
        } else {
            self.out_path("scan")
        }
    }

    fn scan_dimensions(&self, preview: bool) -> (u32, u32) {
        if preview {
            (self.width.min(320), self.height.min(240))
        } else {
            (self.width, self.height)
        }
    }

    fn raw_output_path(&self, preview: bool) -> crate::core::Result<Option<PathBuf>> {
        (!preview && self.save_raw)
            .then(|| {
                crate::config::validate_output_name(&self.output_name).map(|output_name| {
                    PathBuf::from(&self.output_dir).join(format!("{output_name}_raw.tif"))
                })
            })
            .transpose()
    }
}
