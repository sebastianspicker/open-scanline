//! Native media binding for inbound GUI publication actions.

use crate::composition::Runtime;
use crate::domain::image::ImageBuffer;
use crate::error::Result;
use crate::workflows::operation::CancellationToken;
use crate::workflows::publication::{
    apply_export_profile_with_media, prepare_export_options_with_media,
    save_final_image_with_searchable_text_with_media,
    save_final_multipage_from_paths_with_cancellation_with_media, ExportOptions,
    PreparedExportOptions,
};
use std::path::{Path, PathBuf};

pub(crate) fn prepare_export_options(
    destination: &Path,
    options: &ExportOptions,
) -> Result<PreparedExportOptions> {
    let runtime = Runtime::default();
    prepare_export_options_with_media(destination, options, runtime.media())
}

pub(crate) fn apply_export_profile(
    image: &ImageBuffer,
    prepared: &PreparedExportOptions,
) -> Result<ImageBuffer> {
    let runtime = Runtime::default();
    apply_export_profile_with_media(image, prepared, runtime.media())
}

pub(crate) fn save_final_image(
    destination: &Path,
    image: &ImageBuffer,
    dpi: Option<u32>,
    quality: Option<u8>,
    prepared: &PreparedExportOptions,
) -> Result<PathBuf> {
    save_final_image_with_cancellation(destination, image, dpi, quality, prepared, None)
}

pub(crate) fn save_final_image_with_cancellation(
    destination: &Path,
    image: &ImageBuffer,
    dpi: Option<u32>,
    quality: Option<u8>,
    prepared: &PreparedExportOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    let runtime = Runtime::default();
    save_final_image_with_searchable_text_with_media(
        destination,
        image,
        dpi,
        quality,
        prepared,
        None,
        cancellation,
        runtime.media(),
    )
}

#[cfg_attr(not(any(feature = "gui", test)), allow(dead_code))]
pub(crate) fn save_final_multipage_from_paths_with_cancellation(
    destination: &Path,
    paths: &[PathBuf],
    dpi: u32,
    options: &ExportOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    let runtime = Runtime::default();
    save_final_multipage_from_paths_with_cancellation_with_media(
        destination,
        paths,
        dpi,
        options,
        cancellation,
        runtime.media(),
    )
}
