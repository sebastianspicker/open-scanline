use super::{normalize_image_ext, parse_crop_string};
use crate::domain::acquisition::{ScanMode, MIN_SCAN_DPI};
use crate::domain::export::{ExportOptions, OcrEngine};
use crate::domain::image::{Rect, Rotate};
use crate::domain::processing::PipelinePrefs;
use crate::domain::settings::{
    format_curve_points, parse_curve_points, validate_hue, validate_output_name,
    validate_saturation,
};
use crate::error::Result;
use crate::inbound::i18n::Translator;
use crate::infrastructure::acquisition::DeviceMaintenanceCapabilities;
use crate::infrastructure::config::json::{default_config_path, load_config};
use crate::infrastructure::config::AppConfig;
use crate::workflows::settings::resolve_defaults;
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
    pub(super) maintenance_capabilities: DeviceMaintenanceCapabilities,
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
    /// Structured feedback for the workspace; status remains localized display text.
    pub(super) error_message: Option<String>,
    pub(super) last_image: Option<PathBuf>,
    multipage_session: Option<MultipageExportSession>,
    pub(super) hist_summary: String,
    pub(super) preview: Option<PreviewStatistics>,
    pub(super) preview_revision: u64,
    pub(super) discovery_requested: bool,
    pub(super) discovery_loading: bool,
    pub(super) discovery_refresh: bool,
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

macro_rules! build_gui_state {
    ($path:expr, $config:expr, $lang:expr, $translator:expr, $config_load_error:expr, $acquisition:expr, $pipeline:expr, $devices:expr, $device:expr, $maintenance_capabilities:expr, $out_dir:expr) => {
        Self {
            config: $config.clone(),
            config_path: $path,
            active_tab: 0,
            error_message: None,
            device: $device,
            devices: $devices,
            maintenance_capabilities: $maintenance_capabilities,
            width: $acquisition.width.max(16),
            height: $acquisition.height.max(16),
            dpi: $acquisition.dpi_x.max(MIN_SCAN_DPI),
            rotate: $pipeline.rotate.degrees(),
            flip_h: $pipeline.flip_h,
            flip_v: $pipeline.flip_v,
            crop_text: $pipeline
                .crop
                .map(|crop| format!("{},{},{},{}", crop.x, crop.y, crop.width, crop.height))
                .unwrap_or_default(),
            brightness: $pipeline.brightness,
            contrast: $pipeline.contrast,
            saturation: $pipeline.saturation,
            hue: $pipeline.hue,
            curves_text: format_curve_points($pipeline.curves.as_deref()),
            desaturate: $pipeline.desaturate,
            levels_black: $pipeline.levels_black,
            levels_white: $pipeline.levels_white,
            levels_gamma: $pipeline.levels_gamma,
            auto_deskew: $pipeline.auto_deskew,
            deskew_angle: $pipeline.deskew_angle,
            auto_orient: $pipeline.auto_orient,
            auto_crop: $pipeline.auto_crop,
            white_balance: $pipeline.white_balance,
            infrared_clean: $pipeline
                .infrared_clean
                .clone()
                .unwrap_or_else(|| "off".into()),
            sharpen_amount: $pipeline.sharpen_amount,
            descreen: $pipeline.descreen,
            descreen_dpi: $pipeline.descreen_dpi,
            invert_colors: $pipeline.invert,
            auto_levels: $pipeline.auto_levels,
            restore_colors: $pipeline.restore_colors,
            restore_fading: $pipeline.restore_fading,
            grain_reduction: $pipeline
                .grain_reduction
                .clone()
                .unwrap_or_else(|| "off".into()),
            flatten: $pipeline.flatten,
            hole_punch: $pipeline.hole_punch,
            colorize_mode: $pipeline
                .colorize_mode
                .clone()
                .unwrap_or_else(|| "off".into()),
            film_type: $pipeline.film_type.clone().unwrap_or_default(),
            media: media_for_scan_mode($acquisition.mode).into(),
            duplex: $acquisition.duplex,
            batch_pages: $config.batch_pages.max(1),
            output_dir: $out_dir,
            output_name: nonempty_or(&$config.output_name, "scan"),
            output_fmt: normalize_image_ext(&$config.output_format, "png"),
            multipage: $config.multipage,
            multipage_format: multipage_format(&$config.multipage_format),
            save_raw: $config.save_raw,
            contact_sheet: $config.contact_sheet,
            status: $config_load_error.as_ref().map_or_else(
                || $translator.t("ready"),
                |error| format!("{}: {error}", $translator.t("error.config")),
            ),
            config_recovery_required: $config_load_error.is_some(),
            config_load_error: $config_load_error,
            last_image: None,
            multipage_session: None,
            hist_summary: String::new(),
            preview: None,
            preview_revision: 0,
            discovery_requested: true,
            discovery_loading: true,
            discovery_refresh: false,
            language: $lang,
            ocr_engine: ocr_engine(&$config.ocr_engine),
            ocr_language: nonempty_or(&$config.ocr_language, "eng"),
            searchable_pdf: false,
            pdf_password: String::new(),
            scanner_profile_path: String::new(),
            translator: $translator,
            zoom: 1.0,
            frame_index: 0,
            open_path_input: String::new(),
            cancel_requested: Arc::new(AtomicBool::new(false)),
            scanning: false,
        }
    };
}

mod export;
mod init;
mod maintenance;
mod multipage;
mod paths;
mod pipeline;

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
    let device = nonempty_or(&config.last_device_id, "mock");
    let mut devices = vec!["mock".into()];
    if device != "mock" {
        devices.push(device.clone());
    }
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
    match value.trim().to_ascii_lowercase().as_str() {
        "ocrs" => "ocrs".into(),
        "tesseract" => "tesseract".into(),
        _ => "offline".into(),
    }
}

fn multipage_format(value: &str) -> String {
    if matches!(value.to_ascii_lowercase().as_str(), "tif" | "tiff") {
        "tif".into()
    } else {
        "pdf".into()
    }
}

#[cfg_attr(not(feature = "gui"), allow(dead_code))]
#[derive(Debug)]
pub(super) struct PreviewStatistics {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) format: crate::domain::image::PixelFormat,
    pub(super) luma: Vec<u64>,
}
