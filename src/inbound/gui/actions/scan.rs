use super::super::state::GuiState;
use crate::workflows::capture::single::ScanToFileArgs;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

impl GuiState {
    pub(in super::super) fn do_cancel(&mut self) {
        self.cancel_requested.store(true, Ordering::SeqCst);
        self.status = self.translator.t(if self.scanning {
            "status.cancelled"
        } else {
            "status.ready"
        });
    }

    pub(in crate::inbound::gui) fn scan_args(
        &self,
        preview: bool,
    ) -> crate::error::Result<ScanToFileArgs> {
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

    fn scan_output_path(&self, preview: bool) -> crate::error::Result<PathBuf> {
        if preview {
            let output_name = crate::domain::settings::validate_output_name(&self.output_name)?;
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

    fn raw_output_path(&self, preview: bool) -> crate::error::Result<Option<PathBuf>> {
        (!preview && self.save_raw)
            .then(|| {
                crate::domain::settings::validate_output_name(&self.output_name).map(
                    |output_name| {
                        PathBuf::from(&self.output_dir).join(format!("{output_name}_raw.tif"))
                    },
                )
            })
            .transpose()
    }
}
