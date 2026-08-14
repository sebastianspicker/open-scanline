#[cfg(feature = "gui")]
use super::super::normalize_image_ext;
use super::super::open_image_file;
#[cfg(any(test, not(feature = "gui")))]
use super::super::save_image_to_with_export_options;
use super::super::state::GuiState;
#[cfg(any(test, not(feature = "gui")))]
use crate::export::save_final_multipage_from_paths;
#[cfg(any(test, not(feature = "gui")))]
use crate::process::process_image_file_with_export_options;
use crate::process::ProcessOptions;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use super::super::state::MultipageSaveCandidate;
use crate::export::ExportOptions;

/// Immutable save request assembled before any container or encoder work.
#[derive(Debug, Clone)]
pub(in crate::gui) struct SaveAction {
    pub(in crate::gui) src: PathBuf,
    pub(in crate::gui) dest: PathBuf,
    pub(in crate::gui) dpi: u32,
    pub(in crate::gui) export: ExportOptions,
    pub(in crate::gui) candidate: Option<MultipageSaveCandidate>,
    pub(in crate::gui) advance_frame: bool,
    pub(in crate::gui) pending_pdf_password: Option<String>,
}

/// Immutable reprocess request assembled before the worker starts.
#[derive(Debug, Clone)]
pub(in crate::gui) struct ReprocessAction {
    pub(in crate::gui) options: ProcessOptions,
    pub(in crate::gui) export: ExportOptions,
    #[cfg_attr(not(feature = "gui"), allow(dead_code))]
    pub(in crate::gui) dpi: u32,
    pub(in crate::gui) pending_pdf_password: Option<String>,
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
        let Some(src) = self.last_image.clone() else {
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
        let mut pipeline = self.pipeline_prefs();
        pipeline.auto_levels = self.auto_levels;
        let export = self.take_export_options();
        let pending_pdf_password = export.pdf_password.clone();
        Some(ReprocessAction {
            options: ProcessOptions {
                src,
                dst: out,
                pipeline,
                quality: None,
            },
            export,
            dpi: self.dpi,
            pending_pdf_password,
        })
    }

    /// Synchronous test/non-GUI compatibility path. The desktop UI always
    /// uses `prepare_reprocess` and runs this work through `OpenScanlineApp`.
    #[allow(dead_code)]
    #[cfg(any(test, not(feature = "gui")))]
    pub(in super::super) fn do_reprocess(&mut self) {
        let Some(action) = self.prepare_reprocess() else {
            return;
        };
        match process_image_file_with_export_options(&action.options, &action.export) {
            Ok(path) => self.set_image_success(path),
            Err(error) => {
                self.restore_export_password(action.pending_pdf_password);
                self.set_error(error);
            }
        }
    }

    fn save_destination(&self) -> crate::core::Result<Option<PathBuf>> {
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

    fn multipage_extension(&self) -> crate::core::Result<&'static str> {
        match self.multipage_format.trim().to_ascii_lowercase().as_str() {
            "pdf" => Ok("pdf"),
            "tif" | "tiff" => Ok("tif"),
            other => Err(crate::core::ScanError::Invalid(format!(
                "multipage format must be PDF or TIFF, not '{other}'"
            ))),
        }
    }

    pub(in super::super) fn multipage_save_destination(&self) -> crate::core::Result<PathBuf> {
        let output_name = crate::config::validate_output_name(&self.output_name)?;
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

    /// Synchronous test/non-GUI compatibility path. The desktop UI always
    /// uses `prepare_save` / `prepare_save_plus` and runs this through the
    /// cancellable app worker.
    #[allow(dead_code)]
    #[cfg(any(test, not(feature = "gui")))]
    pub(in super::super) fn do_save(&mut self) {
        let Some(action) = self.prepare_save() else {
            return;
        };
        self.run_save_action(action);
    }

    #[cfg(any(test, not(feature = "gui")))]
    pub(in super::super) fn do_save_plus(&mut self) {
        let Some(action) = self.prepare_save_plus() else {
            return;
        };
        self.run_save_action(action);
    }

    #[cfg(any(test, not(feature = "gui")))]
    fn run_save_action(&mut self, action: SaveAction) {
        let result = match action.candidate.as_ref() {
            Some(candidate) => save_final_multipage_from_paths(
                &action.dest,
                &candidate.sources,
                action.dpi,
                &action.export,
            ),
            None => save_image_to_with_export_options(
                &action.src,
                &action.dest,
                action.dpi,
                &action.export,
            ),
        };
        match result {
            Ok(path) => {
                if let Some(candidate) = action.candidate {
                    self.commit_multipage_save(candidate);
                } else {
                    self.last_image = Some(path.clone());
                }
                if action.advance_frame {
                    self.frame_index += 1;
                    self.status = format!(
                        "{} {} ({})",
                        self.translator.t("status.done"),
                        path.display(),
                        self.frame_index
                    );
                } else {
                    self.status =
                        format!("{} {}", self.translator.t("status.done"), path.display());
                }
            }
            Err(error) => {
                self.restore_export_password(action.pending_pdf_password);
                self.set_error(error);
            }
        }
    }
}
