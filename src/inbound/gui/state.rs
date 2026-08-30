use super::{normalize_image_ext, parse_crop_string};
use crate::domain::acquisition::{ScanMode, MIN_SCAN_DPI};
use crate::domain::image::{Rect, Rotate};
use crate::domain::processing::PipelinePrefs;
use crate::error::Result;
use crate::inbound::i18n::Translator;
use crate::infrastructure::acquisition::list_all_devices;
use crate::infrastructure::config::json::{default_config_path, load_config};
use crate::infrastructure::config::json::{
    format_curve_points, parse_curve_points, validate_hue, validate_output_name,
    validate_saturation,
};
use crate::workflows::publication::{ExportOptions, OcrEngine};
use crate::workflows::settings::{resolve_defaults, AppConfig};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Runtime-only state for one GUI multipage destination.
///
/// The export options deliberately stay here rather than in `AppConfig`: the
/// password is consumed from the UI once and must remain available only while
/// the active document is being rebuilt.
#[derive(Debug, Clone)]
struct MultipageExportSession {
    destination: PathBuf,
    sources: Vec<PathBuf>,
    export: ExportOptions,
}

/// Immutable export attempt assembled before the container writer runs.
///
/// It must not alter the active session or consume the UI password because a
/// profile/encoder failure leaves the attempt retryable.
#[derive(Debug, Clone)]
pub(super) struct MultipageSaveCandidate {
    pub(in crate::inbound::gui) destination: PathBuf,
    pub(in crate::inbound::gui) sources: Vec<PathBuf>,
    pub(in crate::inbound::gui) export: ExportOptions,
}

#[cfg_attr(all(test, not(feature = "gui")), allow(dead_code))]
pub(super) struct GuiState {
    pub(super) config: AppConfig,
    pub(super) config_path: PathBuf,
    pub(super) config_load_error: Option<String>,
    pub(super) config_recovery_required: bool,
    pub(super) active_tab: usize,
    pub(super) device: String,
    pub(super) devices: Vec<String>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) dpi: u32,
    pub(super) rotate: i32,
    pub(super) flip_h: bool,
    pub(super) flip_v: bool,
    pub(super) crop_text: String,
    pub(super) brightness: i32,
    pub(super) contrast: i32,
    pub(super) saturation: f64,
    pub(super) hue: f64,
    pub(super) curves_text: String,
    pub(super) desaturate: bool,
    pub(super) levels_black: i32,
    pub(super) levels_white: i32,
    pub(super) levels_gamma: f64,
    pub(super) auto_deskew: bool,
    pub(super) deskew_angle: f64,
    pub(super) auto_orient: bool,
    pub(super) auto_crop: bool,
    pub(super) white_balance: bool,
    pub(super) infrared_clean: String,
    pub(super) sharpen_amount: f64,
    pub(super) descreen: bool,
    pub(super) descreen_dpi: i32,
    pub(super) invert_colors: bool,
    pub(super) auto_levels: bool,
    pub(super) restore_colors: bool,
    pub(super) restore_fading: bool,
    pub(super) grain_reduction: String,
    pub(super) flatten: bool,
    pub(super) hole_punch: bool,
    pub(super) colorize_mode: String,
    pub(super) film_type: String,
    pub(super) media: String,
    pub(super) duplex: bool,
    pub(super) batch_pages: u32,
    pub(super) output_dir: String,
    pub(super) output_name: String,
    pub(super) output_fmt: String,
    pub(super) multipage: bool,
    pub(super) multipage_format: String,
    pub(super) save_raw: bool,
    pub(super) contact_sheet: bool,
    pub(super) status: String,
    pub(super) last_image: Option<PathBuf>,
    multipage_session: Option<MultipageExportSession>,
    pub(super) hist_summary: String,
    pub(super) language: String,
    pub(super) ocr_engine: String,
    pub(super) ocr_language: String,
    pub(super) searchable_pdf: bool,
    /// Runtime-only PDF password. It is intentionally never copied to AppConfig.
    pub(super) pdf_password: String,
    pub(super) scanner_profile_path: String,
    pub(super) translator: Translator,
    pub(super) zoom: f32,
    pub(super) frame_index: i32,
    pub(super) open_path_input: String,
    pub(super) cancel_requested: Arc<AtomicBool>,
    pub(super) scanning: bool,
}

#[cfg_attr(all(test, not(feature = "gui")), allow(dead_code))]
impl GuiState {
    pub(super) fn new(config_path: Option<&Path>) -> Self {
        let (path, config, lang, translator, config_load_error) =
            load_config_and_translation(config_path);
        // Persisted acquisition and processing fields flow through the shared
        // resolver. The remaining constructor values are explicit GUI-only
        // state (output/session, locale, and view state).
        let resolved = resolve_defaults(&config);
        let acquisition = &resolved.acquisition;
        let pipeline = &resolved.processing;
        let (devices, device) = devices_and_selection(&config);
        let out_dir = configured_output_dir(&config);
        Self {
            config: config.clone(),
            config_path: path,
            active_tab: 0,
            device,
            devices,
            width: acquisition.width.max(16),
            height: acquisition.height.max(16),
            dpi: acquisition.dpi_x.max(MIN_SCAN_DPI),
            rotate: pipeline.rotate.degrees(),
            flip_h: pipeline.flip_h,
            flip_v: pipeline.flip_v,
            crop_text: pipeline
                .crop
                .map(|crop| format!("{},{},{},{}", crop.x, crop.y, crop.width, crop.height))
                .unwrap_or_default(),
            brightness: pipeline.brightness,
            contrast: pipeline.contrast,
            saturation: pipeline.saturation,
            hue: pipeline.hue,
            curves_text: format_curve_points(pipeline.curves.as_deref()),
            desaturate: pipeline.desaturate,
            levels_black: pipeline.levels_black,
            levels_white: pipeline.levels_white,
            levels_gamma: pipeline.levels_gamma,
            auto_deskew: pipeline.auto_deskew,
            deskew_angle: pipeline.deskew_angle,
            auto_orient: pipeline.auto_orient,
            auto_crop: pipeline.auto_crop,
            white_balance: pipeline.white_balance,
            infrared_clean: pipeline
                .infrared_clean
                .clone()
                .unwrap_or_else(|| "off".into()),
            sharpen_amount: pipeline.sharpen_amount,
            descreen: pipeline.descreen,
            descreen_dpi: pipeline.descreen_dpi,
            invert_colors: pipeline.invert,
            auto_levels: pipeline.auto_levels,
            restore_colors: pipeline.restore_colors,
            restore_fading: pipeline.restore_fading,
            grain_reduction: pipeline
                .grain_reduction
                .clone()
                .unwrap_or_else(|| "off".into()),
            flatten: pipeline.flatten,
            hole_punch: pipeline.hole_punch,
            colorize_mode: pipeline
                .colorize_mode
                .clone()
                .unwrap_or_else(|| "off".into()),
            film_type: pipeline.film_type.clone().unwrap_or_default(),
            media: media_for_scan_mode(acquisition.mode).into(),
            duplex: acquisition.duplex,
            batch_pages: config.batch_pages.max(1),
            output_dir: out_dir,
            output_name: nonempty_or(&config.output_name, "scan"),
            output_fmt: normalize_image_ext(&config.output_format, "png"),
            multipage: config.multipage,
            multipage_format: multipage_format(&config.multipage_format),
            save_raw: config.save_raw,
            contact_sheet: config.contact_sheet,
            status: config_load_error.as_ref().map_or_else(
                || translator.t("ready"),
                |error| format!("{}: {error}", translator.t("error.config")),
            ),
            config_recovery_required: config_load_error.is_some(),
            config_load_error,
            last_image: None,
            multipage_session: None,
            hist_summary: String::new(),
            language: lang,
            ocr_engine: ocr_engine(&config.ocr_engine),
            ocr_language: nonempty_or(&config.ocr_language, "eng"),
            searchable_pdf: false,
            pdf_password: String::new(),
            scanner_profile_path: String::new(),
            translator,
            zoom: 1.0,
            frame_index: 0,
            open_path_input: String::new(),
            cancel_requested: Arc::new(AtomicBool::new(false)),
            scanning: false,
        }
    }

    pub(super) fn out_path(&self, suffix: &str) -> Result<PathBuf> {
        let output_name = validate_output_name(&self.output_name)?;
        let ext = normalize_image_ext(&self.output_fmt, "png");
        Ok(PathBuf::from(&self.output_dir).join(format!(
            "{output_name}_{suffix}_{:03}.{ext}",
            self.frame_index
        )))
    }

    pub(super) fn out_path_plain(&self, suffix: &str) -> Result<PathBuf> {
        let output_name = validate_output_name(&self.output_name)?;
        let ext = normalize_image_ext(&self.output_fmt, "png");
        Ok(PathBuf::from(&self.output_dir).join(format!("{output_name}_{suffix}.{ext}")))
    }

    pub(super) fn pipeline_prefs(&self) -> PipelinePrefs {
        let crop = parse_crop_string(&self.crop_text)
            .ok()
            .flatten()
            .and_then(|value| {
                (value[2] > 0 && value[3] > 0)
                    .then(|| Rect::new(value[0], value[1], value[2] as u32, value[3] as u32))
            });
        PipelinePrefs {
            rotate: Rotate::from_degrees(self.rotate),
            flip_h: self.flip_h,
            flip_v: self.flip_v,
            crop,
            brightness: self.brightness,
            contrast: self.contrast,
            saturation: self.saturation,
            hue: self.hue,
            curves: parse_curve_points(&self.curves_text).ok().flatten(),
            desaturate: self.desaturate,
            levels_black: self.levels_black,
            levels_white: self.levels_white,
            levels_gamma: self.levels_gamma,
            auto_deskew: self.auto_deskew,
            deskew_angle: self.deskew_angle,
            auto_orient: self.auto_orient,
            auto_crop: self.auto_crop,
            white_balance: self.white_balance,
            sharpen_amount: self.sharpen_amount,
            invert: self.invert_colors,
            auto_levels: self.auto_levels,
            infrared_clean: (self.infrared_clean != "off").then(|| self.infrared_clean.clone()),
            descreen: self.descreen,
            descreen_dpi: self.descreen_dpi,
            restore_colors: self.restore_colors,
            restore_fading: self.restore_fading,
            grain_reduction: (self.grain_reduction != "off").then(|| self.grain_reduction.clone()),
            flatten: self.flatten,
            hole_punch: self.hole_punch,
            colorize_mode: (self.colorize_mode != "off").then(|| self.colorize_mode.clone()),
            film_type: (!self.film_type.trim().is_empty()).then(|| self.film_type.clone()),
        }
    }

    pub(super) fn validate_color_controls(&self) -> Result<()> {
        validate_saturation(self.saturation)?;
        validate_hue(self.hue)?;
        parse_curve_points(&self.curves_text)?;
        Ok(())
    }

    pub(super) fn scan_mode(&self) -> ScanMode {
        match self.media.as_str() {
            "adf" | "document" => ScanMode::Document,
            "transparency" | "film" => ScanMode::Film,
            _ => ScanMode::Reflective,
        }
    }

    /// Snapshot runtime-only export choices for a worker, clearing the secret
    /// as soon as it has been copied into that immutable snapshot.
    pub(super) fn take_export_options(&mut self) -> ExportOptions {
        let mut export = self.export_options();
        export.pdf_password =
            (!self.pdf_password.is_empty()).then(|| std::mem::take(&mut self.pdf_password));
        export
    }

    /// Restore a password consumed for a failed export only while the user has
    /// not supplied a replacement in the meantime.
    pub(super) fn restore_export_password(&mut self, password: Option<String>) {
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
            ocr_engine: if self.ocr_engine == "tesseract" {
                OcrEngine::Tesseract
            } else {
                OcrEngine::Offline
            },
            scanner_profile: (!self.scanner_profile_path.trim().is_empty())
                .then(|| PathBuf::from(self.scanner_profile_path.trim())),
        }
    }

    /// Snapshot a pending multipage save. A new destination consumes the
    /// runtime password into the worker snapshot; an existing document keeps
    /// its already-owned export settings for retry/cancel safety.
    pub(super) fn take_multipage_save_candidate(
        &mut self,
        destination: &Path,
        source: &Path,
    ) -> (MultipageSaveCandidate, Option<String>) {
        let session = self
            .multipage_session
            .as_ref()
            .filter(|session| session.destination == destination);
        let mut sources = session.map_or_else(Vec::new, |session| session.sources.clone());
        sources.push(source.to_path_buf());
        let (export, pending_pdf_password) = match session {
            Some(session) => (session.export.clone(), None),
            None => {
                let export = self.take_export_options();
                let pending_pdf_password = export.pdf_password.clone();
                (export, pending_pdf_password)
            }
        };
        (
            MultipageSaveCandidate {
                destination: destination.to_path_buf(),
                sources,
                export,
            },
            pending_pdf_password,
        )
    }

    /// Install an export candidate only after its complete rebuilt container
    /// was published, then consume the runtime-only password.
    pub(super) fn commit_multipage_save(&mut self, candidate: MultipageSaveCandidate) {
        self.multipage_session = Some(MultipageExportSession {
            destination: candidate.destination,
            sources: candidate.sources,
            export: candidate.export,
        });
        self.pdf_password.clear();
    }

    pub(super) fn clear_multipage_session(&mut self) {
        self.multipage_session = None;
    }

    pub(super) fn references_working_source(&self, path: &Path) -> bool {
        self.last_image.as_deref() == Some(path)
            || self
                .multipage_session
                .as_ref()
                .is_some_and(|session| session.sources.iter().any(|source| source == path))
    }
}

fn media_for_scan_mode(mode: ScanMode) -> &'static str {
    match mode {
        ScanMode::Reflective => "flatbed",
        ScanMode::Film => "film",
        ScanMode::Document => "document",
    }
}

fn load_config_and_translation(
    config_path: Option<&Path>,
) -> (PathBuf, AppConfig, String, Translator, Option<String>) {
    let path = config_path
        .map(Path::to_path_buf)
        .unwrap_or_else(default_config_path);
    let (config, config_load_error) = match load_config(Some(&path)) {
        Ok(config) => (config, None),
        Err(error) => (
            AppConfig::default(),
            Some(format!(
                "could not load {} ({error}); reset configuration before saving a replacement",
                path.display()
            )),
        ),
    };
    let language = if config.language.is_empty() {
        "en".into()
    } else {
        config.language.clone()
    };
    let translator = Translator::new(&language);
    let language = translator.language().to_owned();
    (path, config, language, translator, config_load_error)
}

fn devices_and_selection(config: &AppConfig) -> (Vec<String>, String) {
    let devices: Vec<String> = list_all_devices()
        .into_iter()
        .map(|device| device.id)
        .collect();
    let device = if config.last_device_id.is_empty() {
        devices.first().cloned().unwrap_or_else(|| "mock".into())
    } else {
        config.last_device_id.clone()
    };
    (devices, device)
}

fn configured_output_dir(config: &AppConfig) -> String {
    if config.output_dir.is_empty() {
        ".".into()
    } else {
        config.output_dir.clone()
    }
}

fn nonempty_or(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.into()
    } else {
        value.into()
    }
}

fn ocr_engine(value: &str) -> String {
    if value.eq_ignore_ascii_case("tesseract") {
        "tesseract".into()
    } else {
        "offline".into()
    }
}

fn multipage_format(value: &str) -> String {
    if matches!(value.to_ascii_lowercase().as_str(), "tif" | "tiff") {
        "tif".into()
    } else {
        "pdf".into()
    }
}
