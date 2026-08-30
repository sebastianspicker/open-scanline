//! Desktop GUI shell (OSL-GUI) — interactive egui/eframe product surface.
//!
//! Shares scan/process/batch orchestration with CLI via library entry points.

use crate::domain::processing::histogram;
use crate::inbound::api::publication::{
    apply_export_profile, prepare_export_options, save_final_image,
};
use crate::infrastructure::media::load_image;
use crate::workflows::publication::ExportOptions;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[cfg(any(feature = "gui", test))]
mod actions;
mod app;
#[cfg(any(feature = "gui", test))]
mod state;
#[cfg(test)]
mod tests;
#[cfg(feature = "gui")]
mod view;

/// Parse `x,y,w,h` crop text; empty → None.
pub fn parse_crop_string(value: &str) -> Result<Option<[i32; 4]>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    crate::domain::image::parse_crop_components(value)
        .map(Some)
        .map_err(|error| error.to_string())
}

pub fn normalize_image_ext(format: &str, default: &str) -> String {
    let extension = format.trim().trim_start_matches('.').to_ascii_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "tif" | "tiff" | "webp" | "bmp" | "gif" | "pdf" | "jxl" => {
            match extension.as_str() {
                "jpeg" => "jpg".into(),
                "tiff" => "tif".into(),
                _ => extension,
            }
        }
        _ => default.into(),
    }
}

/// Histogram data for graph views (GUI graphs).
pub fn histogram_for_path(path: impl AsRef<Path>) -> crate::error::Result<Value> {
    histogram(&load_image(path)?)
}

/// Validate a readable image and return the path used by the Open action.
pub fn open_image_file(path: impl AsRef<Path>) -> crate::error::Result<PathBuf> {
    let path = path.as_ref();
    if !path.is_file() {
        return Err(crate::error::ScanError::Other(format!(
            "file not found: {}",
            path.display()
        )));
    }
    let _image = load_image(path)?;
    Ok(path.to_path_buf())
}

/// Copy the last image through the shared imaging path (Save / Save+).
pub fn save_image_to(
    src: impl AsRef<Path>,
    dest: impl AsRef<Path>,
    dpi: u32,
) -> crate::error::Result<PathBuf> {
    save_image_to_with_export_options(src, dest, dpi, &ExportOptions::default())
}

/// Copy an image through the runtime-only PDF/OCR/profile export path.
pub(crate) fn save_image_to_with_export_options(
    src: impl AsRef<Path>,
    dest: impl AsRef<Path>,
    dpi: u32,
    export: &ExportOptions,
) -> crate::error::Result<PathBuf> {
    let dest = dest.as_ref();
    let prepared = prepare_export_options(dest, export)?;
    let image = apply_export_profile(&load_image(src)?, &prepared)?;
    save_final_image(dest, &image, Some(dpi), None, &prepared)
}

/// File fingerprint for preview cache: `(mtime_unix_secs_or_0, byte_len)`.
pub fn preview_file_fingerprint(path: impl AsRef<Path>) -> Option<(u64, u64)> {
    let metadata = std::fs::metadata(path.as_ref()).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    Some((modified, metadata.len()))
}

/// True when the preview texture must be rebuilt for a path or file-content change.
pub fn preview_texture_needs_reload(
    last_image: Option<&Path>,
    cached_path: Option<&Path>,
    cached_fingerprint: Option<(u64, u64)>,
) -> bool {
    let Some(path) = last_image else {
        return false;
    };
    let Some(fingerprint) = preview_file_fingerprint(path) else {
        return true;
    };
    cached_path != Some(path) || cached_fingerprint != Some(fingerprint)
}

/// Launch desktop GUI. Uses eframe when the `gui` feature is enabled.
pub fn run_gui(config_path: Option<&Path>) -> i32 {
    app::run(config_path)
}
