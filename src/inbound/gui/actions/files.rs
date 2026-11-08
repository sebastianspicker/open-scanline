#[cfg(feature = "gui")]
use super::super::normalize_image_ext;
use super::super::open_image_file;
use super::super::state::GuiState;
use crate::workflows::process::ProcessOptions;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use super::super::state::MultipageSaveCandidate;
use crate::domain::export::ExportOptions;

/// Immutable save request assembled before any container or encoder work.
#[derive(Clone)]
pub(in crate::inbound::gui) struct SaveAction {
    pub(in crate::inbound::gui) src: PathBuf,
    pub(in crate::inbound::gui) dest: PathBuf,
    pub(in crate::inbound::gui) dpi: u32,
    pub(in crate::inbound::gui) export: ExportOptions,
    pub(in crate::inbound::gui) candidate: Option<MultipageSaveCandidate>,
    pub(in crate::inbound::gui) advance_frame: bool,
    pub(in crate::inbound::gui) pending_pdf_password: Option<String>,
}

/// Immutable reprocess request assembled before the worker starts.
///
/// Only `execute_reprocess_action` (gui-gated) reads every field; without
/// the `gui` feature this snapshot is assembled but never executed.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
#[derive(Clone)]
pub(in crate::inbound::gui) struct ReprocessAction {
    pub(in crate::inbound::gui) options: ProcessOptions,
    pub(in crate::inbound::gui) export: ExportOptions,
    pub(in crate::inbound::gui) dpi: u32,
    pub(in crate::inbound::gui) pending_pdf_password: Option<String>,
}

#[cfg_attr(all(test, not(feature = "gui")), allow(dead_code))]
impl GuiState {
    pub(in super::super) fn do_open_file(&mut self, path: Option<&Path>) {
        let selected = path.map(Path::to_path_buf).or_else(|| {
            (!self.open_path_input.trim().is_empty())
                .then(|| PathBuf::from(self.open_path_input.trim()))
        });
        #[cfg(feature = "gui")]
        let selected = selected.or_else(|| {
            // Deliberately narrower than `infrastructure::media::supported_extensions()`:
            // this dialog picks a file for `load_image`, which decodes raster
            // formats only; "pdf" and "jxl" are supported write targets but not
            // read here.
            rfd::FileDialog::new()
                .add_filter(
                    "Images",
                    &["png", "jpg", "jpeg", "tif", "tiff", "webp", "bmp", "gif"],
                )
                .pick_file()
        });
        let Some(path) = selected else {
            self.status = self.translator.t("menu.open");
            return;
        };
        match open_image_file(&path) {
            Ok(path) => {
                self.open_path_input = path.display().to_string();
                // A newly opened file is not derived from the last scan's raw.
                self.last_raw = None;
                self.set_image_success(path);
            }
            Err(error) => self.set_error(error),
        }
    }

    /// Keep the native save dialog and validation on the UI thread, but leave
    /// image/container work to the app-owned worker.
    pub(in super::super) fn prepare_save(&mut self) -> Option<SaveAction> {
        let Some(src) = self.last_image.clone() else {
            self.set_open_error();
            return None;
        };
        let dest = match self.save_destination() {
            Ok(Some(dest)) => dest,
            Ok(None) => {
                self.status = self.translator.t("status.cancelled");
                return None;
            }
            Err(error) => {
                self.set_error(error);
                return None;
            }
        };
        self.prepare_save_to(src, dest, false)
    }

    /// Snapshot a Save+ request without running its encoder/container work.
    pub(in super::super) fn prepare_save_plus(&mut self) -> Option<SaveAction> {
        let Some(src) = self.last_image.clone() else {
            self.set_open_error();
            return None;
        };
        let dest = if self.multipage {
            self.multipage_save_destination()
        } else {
            self.out_path("save")
        };
        let dest = match dest {
            Ok(dest) => dest,
            Err(error) => {
                self.set_error(error);
                return None;
            }
        };
        self.prepare_save_to(src, dest, true)
    }

    /// Validate and snapshot reprocessing before moving expensive pipeline work
    /// to the app-owned worker.
    pub(in super::super) fn prepare_reprocess(&mut self) -> Option<ReprocessAction> {
        if self.cancel_requested.load(Ordering::SeqCst) {
            self.status = self.translator.t("status.cancelled");
            return None;
        }
        if let Err(error) = self.validate_color_controls() {
            self.set_error(format!("invalid color controls: {error}"));
            return None;
        }
        // The saved raw file is unprocessed; the last image already has the
        // pipeline applied and would stack the effects a second time.
        let Some(src) = self
            .last_raw
            .clone()
            .filter(|raw| raw.is_file())
            .or_else(|| self.last_image.clone())
        else {
            self.set_open_error();
            return None;
        };
        let out = match self.out_path_plain("reprocess") {
            Ok(out) => out,
            Err(error) => {
                self.set_error(error);
                return None;
            }
        };
        let pipeline = self.pipeline_prefs();
        let export = self.take_export_options();
        let pending_pdf_password = export.pdf_password.clone();
        Some(ReprocessAction {
            options: ProcessOptions {
                src,
                dst: out,
                pipeline,
                quality: None,
                overwrite: true,
            },
            export,
            dpi: self.dpi,
            pending_pdf_password,
        })
    }

    fn save_destination(&self) -> crate::error::Result<Option<PathBuf>> {
        let dest = if self.multipage {
            self.multipage_save_destination()?
        } else {
            self.out_path_plain("save")?
        };
        #[cfg(feature = "gui")]
        {
            let default_name = dest
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("scan.png");
            let mut dialog = rfd::FileDialog::new().set_file_name(default_name);
            if let Some(parent) = dest.parent() {
                dialog = dialog.set_directory(parent);
            }
            let Some(mut dest) = dialog.save_file() else {
                return Ok(None);
            };
            if self.multipage {
                // Native dialogs do not consistently enforce filters. Keep
                // ordinary Save aligned with the selected multipage writer.
                dest.set_extension(self.multipage_extension()?);
            } else if dest.extension().is_none() {
                dest.set_extension(normalize_image_ext(&self.output_fmt, "png"));
            }
            Ok(Some(dest))
        }
        #[cfg(not(feature = "gui"))]
        Ok(Some(dest))
    }

    fn multipage_extension(&self) -> crate::error::Result<&'static str> {
        match self.multipage_format.trim().to_ascii_lowercase().as_str() {
            "pdf" => Ok("pdf"),
            "tif" | "tiff" => Ok("tif"),
            other => Err(crate::error::ScanError::Invalid(format!(
                "multipage format must be PDF or TIFF, not '{other}'"
            ))),
        }
    }

    pub(in super::super) fn multipage_save_destination(&self) -> crate::error::Result<PathBuf> {
        let output_name = crate::domain::settings::validate_output_name(&self.output_name)?;
        Ok(PathBuf::from(&self.output_dir).join(format!(
            "{output_name}_multipage.{}",
            self.multipage_extension()?
        )))
    }

    fn prepare_save_to(
        &mut self,
        src: PathBuf,
        dest: PathBuf,
        advance_frame: bool,
    ) -> Option<SaveAction> {
        let (export, candidate, pending_pdf_password) = if self.multipage {
            let (candidate, pending_pdf_password) = self.take_multipage_save_candidate(&dest, &src);
            (
                candidate.export.clone(),
                Some(candidate),
                pending_pdf_password,
            )
        } else {
            self.clear_multipage_session();
            let export = self.take_export_options();
            let pending_pdf_password = export.pdf_password.clone();
            (export, None, pending_pdf_password)
        };
        Some(SaveAction {
            src,
            dest,
            dpi: self.dpi,
            export,
            candidate,
            advance_frame,
            pending_pdf_password,
        })
    }
}
