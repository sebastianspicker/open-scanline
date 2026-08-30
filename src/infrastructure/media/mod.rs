//! Native image, profile, OCR, and document-media integrations.

mod codecs;
pub mod icc;
mod multipage;
pub mod ocr;
mod pdf;
mod preview;
mod tiff;

pub use codecs::{
    convert_image, detect_format, detect_format_bytes, is_png_magic, load_image,
    load_image_with_limits, read_magic, save_image, save_image_with_cancellation,
    supported_extensions, to_rgb_bytes, validate_png_magic, PNG_MAGIC,
};
pub use icc::*;
pub(crate) use multipage::save_raw_image_with_cancellation;
pub use multipage::{
    save_index_contact_sheet, save_index_contact_sheet_with_cancellation, save_raw_image,
};
pub use ocr::*;
pub(crate) use pdf::{checked_pdf_searchable_text_total, validate_pdf_password};
pub use pdf::{
    save_multipage_pdf, save_multipage_pdf_with_cancellation, save_pdf_from_paths_with_options,
    save_pdf_from_paths_with_options_and_cancellation,
    save_pdf_from_paths_with_options_and_transform,
    save_pdf_from_paths_with_options_and_transform_and_cancellation, save_pdf_with_options,
    save_pdf_with_options_and_cancellation, PdfOptions,
};
pub use preview::image_buffer_to_rgba;
pub use tiff::{
    append_page_to_multipage, save_multipage_tiff, save_multipage_tiff_from_paths_with_transform,
    save_multipage_tiff_from_paths_with_transform_and_cancellation,
    save_multipage_tiff_with_cancellation,
};

/// Convert an image while forwarding cancellation to command-backed encoders.
pub fn convert_image_with_cancellation(
    input: impl AsRef<std::path::Path>,
    output: impl AsRef<std::path::Path>,
    dpi: Option<u32>,
    cancellation: crate::workflows::operation::CancellationToken,
) -> crate::error::Result<std::path::PathBuf> {
    let image = load_image(input)?;
    save_image_with_cancellation(output, &image, dpi, None, Some(&cancellation))
}

/// Production media adapter used by workflows.
#[derive(Debug, Default, Clone, Copy)]
pub struct NativeMedia;

impl crate::workflows::ports::media::MediaPort for NativeMedia {
    fn validate_output_leaf(
        &self,
        path: &std::path::Path,
        label: &str,
    ) -> crate::error::Result<()> {
        crate::infrastructure::runtime::atomic_publish::validate_output_leaf(path, label)
    }

    fn output_paths_alias(
        &self,
        left: &std::path::Path,
        right: &std::path::Path,
    ) -> crate::error::Result<bool> {
        crate::infrastructure::runtime::atomic_publish::output_paths_alias(left, right)
    }

    fn prepare_output_directory(&self, directory: &std::path::Path) -> crate::error::Result<()> {
        std::fs::create_dir_all(directory)?;
        Ok(())
    }

    fn supports_extension(&self, extension: &str) -> bool {
        supported_extensions().contains(&extension)
    }

    fn load(
        &self,
        source: &std::path::Path,
    ) -> crate::error::Result<crate::domain::image::ImageBuffer> {
        load_image(source)
    }

    fn load_scanner_profile(
        &self,
        source: &std::path::Path,
    ) -> crate::error::Result<serde_json::Value> {
        load_scanner_profile(source)
    }

    fn validate_pdf_password(&self, password: &str) -> crate::error::Result<()> {
        validate_pdf_password(password)
    }

    fn apply_scanner_profile(
        &self,
        image: &crate::domain::image::ImageBuffer,
        profile: Option<&serde_json::Value>,
    ) -> crate::error::Result<crate::domain::image::ImageBuffer> {
        profile.map_or_else(
            || Ok(image.clone()),
            |profile| apply_scanner_profile(image, profile),
        )
    }

    fn recognize(
        &self,
        image: &crate::domain::image::ImageBuffer,
        language: &str,
        offline: bool,
        cancellation: Option<&crate::workflows::operation::CancellationToken>,
    ) -> crate::error::Result<String> {
        Ok(ocr_image_with_cancellation(image, language, offline, cancellation)?.text)
    }

    fn publish_page(
        &self,
        destination: &std::path::Path,
        image: &crate::domain::image::ImageBuffer,
        dpi: Option<u32>,
        quality: Option<u8>,
        cancellation: Option<&crate::workflows::operation::CancellationToken>,
    ) -> crate::error::Result<std::path::PathBuf> {
        save_image_with_cancellation(destination, image, dpi, quality, cancellation)
    }

    fn publish_raw(
        &self,
        destination: &std::path::Path,
        image: &crate::domain::image::ImageBuffer,
        dpi: Option<u32>,
        cancellation: Option<&crate::workflows::operation::CancellationToken>,
    ) -> crate::error::Result<std::path::PathBuf> {
        save_raw_image_with_cancellation(destination, image, dpi, cancellation)
    }

    fn checked_pdf_searchable_text_total(
        &self,
        current: usize,
        text: &str,
        page_index: usize,
    ) -> crate::error::Result<usize> {
        checked_pdf_searchable_text_total(current, text, page_index)
    }

    #[allow(clippy::too_many_arguments)]
    fn publish_pdf(
        &self,
        destination: &std::path::Path,
        images: &[crate::domain::image::ImageBuffer],
        dpi: u32,
        title: &str,
        password: Option<&str>,
        searchable_pages: Option<Vec<String>>,
        cancellation: Option<&crate::workflows::operation::CancellationToken>,
    ) -> crate::error::Result<std::path::PathBuf> {
        save_pdf_with_options_and_cancellation(
            destination,
            images,
            &PdfOptions {
                dpi,
                title: title.into(),
                password: password.map(str::to_owned),
                searchable_pages,
            },
            cancellation,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn publish_pdf_from_paths(
        &self,
        paths: &[std::path::PathBuf],
        destination: &std::path::Path,
        dpi: u32,
        title: &str,
        password: Option<&str>,
        searchable_pages: Option<Vec<String>>,
        profile: Option<&serde_json::Value>,
        cancellation: Option<&crate::workflows::operation::CancellationToken>,
    ) -> crate::error::Result<std::path::PathBuf> {
        let options = PdfOptions {
            dpi,
            title: title.into(),
            password: password.map(str::to_owned),
            searchable_pages,
        };
        match profile {
            Some(profile) => save_pdf_from_paths_with_options_and_transform_and_cancellation(
                paths,
                destination,
                &options,
                |image| apply_scanner_profile(image, profile),
                cancellation,
            ),
            None => save_pdf_from_paths_with_options_and_cancellation(
                paths,
                destination,
                &options,
                cancellation,
            ),
        }
    }

    fn publish_tiff_from_paths(
        &self,
        paths: &[std::path::PathBuf],
        destination: &std::path::Path,
        dpi: Option<u32>,
        profile: Option<&serde_json::Value>,
        cancellation: Option<&crate::workflows::operation::CancellationToken>,
    ) -> crate::error::Result<std::path::PathBuf> {
        match profile {
            Some(profile) => save_multipage_tiff_from_paths_with_transform_and_cancellation(
                paths,
                destination,
                dpi,
                |image| apply_scanner_profile(image, profile),
                cancellation,
            ),
            None => save_multipage_tiff_with_cancellation(paths, destination, dpi, cancellation),
        }
    }

    fn publish_contact_sheet(
        &self,
        paths: &[std::path::PathBuf],
        destination: &std::path::Path,
        cancellation: Option<&crate::workflows::operation::CancellationToken>,
    ) -> crate::error::Result<std::path::PathBuf> {
        save_index_contact_sheet_with_cancellation(
            paths,
            destination,
            4,
            160,
            None,
            4,
            cancellation,
        )
    }
}
