//! Shared image process path used by CLI `process` and GUI extended actions.

use crate::atomic_write::validate_output_leaf;
use crate::core::{PipelinePrefs, Result, ScanError};
use crate::device::CancellationToken;
use crate::export::{
    apply_export_profile, prepare_export_options, save_final_image_with_cancellation, ExportOptions,
};
use crate::imaging::{load_image, supported_extensions};
use crate::pipeline::apply_pipeline;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessOptions {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub pipeline: PipelinePrefs,
    pub quality: Option<u8>,
}

/// Run a typed load, process, and save request.
pub fn process_image_file(options: &ProcessOptions) -> Result<PathBuf> {
    process_image_file_with_export_options(options, &ExportOptions::default())
}

/// Process an image while allowing command-backed export stages to observe cancellation.
pub fn process_image_file_with_token(
    options: &ProcessOptions,
    token: CancellationToken,
) -> Result<PathBuf> {
    process_image_file_with_export_options_and_token(options, &ExportOptions::default(), token)
}

/// Process an image with runtime-only PDF/OCR/profile export controls.
pub fn process_image_file_with_export_options(
    options: &ProcessOptions,
    export: &ExportOptions,
) -> Result<PathBuf> {
    process_image_file_with_export_options_and_cancellation(options, export, None)
}

/// Process with export options and a shared cancellation token.
pub fn process_image_file_with_export_options_and_token(
    options: &ProcessOptions,
    export: &ExportOptions,
    token: CancellationToken,
) -> Result<PathBuf> {
    process_image_file_with_export_options_and_cancellation(options, export, Some(&token))
}

fn process_image_file_with_export_options_and_cancellation(
    options: &ProcessOptions,
    export: &ExportOptions,
    cancellation: Option<&CancellationToken>,
) -> Result<PathBuf> {
    // Validate destination semantics and profile before writing any output.
    validate_process_destination(&options.dst)?;
    let export = prepare_export_options(&options.dst, export)?;
    let image = load_image(&options.src)?;
    let image = apply_pipeline(&image, &options.pipeline)?;
    let image = apply_export_profile(&image, &export)?;
    save_final_image_with_cancellation(
        &options.dst,
        &image,
        None,
        options.quality,
        &export,
        cancellation,
    )
}

fn validate_process_destination(destination: &std::path::Path) -> Result<()> {
    validate_output_leaf(destination, "process output")?;
    let extension = destination
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| ScanError::Invalid("process output has no supported extension".into()))?;
    if !supported_extensions().contains(&extension.as_str()) {
        return Err(ScanError::Invalid(format!(
            "unsupported process output extension '.{extension}'"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{ImageBuffer, PixelFormat};
    use crate::export::{ExportOptions, OcrEngine};
    use crate::icc::{profile_scanner_it8, save_profile_json};
    use crate::imaging::save_image;
    use lopdf::Document;

    #[test]
    fn process_roundtrip_png() {
        let dir = std::env::temp_dir().join("open_scanline_process_test");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.png");
        let dst = dir.join("dst.png");
        let data = vec![10u8, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120];
        let img = ImageBuffer::new(2, 2, PixelFormat::Rgb8, data).unwrap();
        save_image(&src, &img, None, None).unwrap();
        let out = process_image_file(&ProcessOptions {
            src: src.clone(),
            dst: dst.clone(),
            pipeline: PipelinePrefs {
                invert: true,
                ..PipelinePrefs::default()
            },
            quality: None,
        })
        .unwrap();
        assert!(out.is_file());
        let loaded = load_image(&out).unwrap();
        assert_eq!(loaded.width, 2);
        assert_eq!(loaded.height, 2);
        assert_eq!(loaded.data[0], 255 - 10);
    }

    #[test]
    fn process_sharpen_changes_pixels() {
        let dir = std::env::temp_dir().join("open_scanline_process_sharpen");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.png");
        let dst = dir.join("dst.png");
        // Edge pattern so sharpen has effect
        let w = 16u32;
        let h = 16u32;
        let mut data = vec![128u8; (w * h * 3) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 3) as usize;
                let v = if x < 8 { 40 } else { 220 };
                data[i] = v;
                data[i + 1] = v;
                data[i + 2] = v;
            }
        }
        let img = ImageBuffer::new(w, h, PixelFormat::Rgb8, data.clone()).unwrap();
        save_image(&src, &img, None, None).unwrap();
        let out = process_image_file(&ProcessOptions {
            src: src.clone(),
            dst: dst.clone(),
            pipeline: PipelinePrefs {
                sharpen_amount: 1.5,
                ..PipelinePrefs::default()
            },
            quality: None,
        })
        .unwrap();
        let loaded = load_image(&out).unwrap();
        assert_eq!(loaded.width, w);
        assert_ne!(
            loaded.data, data,
            "sharpen_amount>0 must alter edge pixels via real process path"
        );
    }

    #[test]
    fn process_prefs_filter_ops_change_pixels() {
        let dir = std::env::temp_dir().join("open_scanline_process_prefs");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.png");
        let dst = dir.join("dst.png");
        let w = 16u32;
        let h = 16u32;
        let mut data = vec![40u8; (w * h * 3) as usize];
        data[0] = 250;
        data[1] = 250;
        data[2] = 250;
        let img = ImageBuffer::new(w, h, PixelFormat::Rgb8, data.clone()).unwrap();
        save_image(&src, &img, None, None).unwrap();
        let prefs = PipelinePrefs {
            infrared_clean: Some("medium".into()),
            descreen: true,
            descreen_dpi: 75,
            ..PipelinePrefs::default()
        };
        let out = process_image_file(&ProcessOptions {
            src,
            dst,
            pipeline: prefs,
            quality: None,
        })
        .unwrap();
        let loaded = load_image(&out).unwrap();
        assert_eq!(loaded.width, w);
        assert_ne!(
            loaded.data, data,
            "process_image_file must apply filter prefs via real pipeline"
        );
    }

    #[test]
    fn process_deskew_expands_canvas() {
        let dir = std::env::temp_dir().join("open_scanline_process_deskew");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.png");
        let dst = dir.join("dst.png");
        let w = 40u32;
        let h = 30u32;
        let mut data = vec![255u8; (w * h * 3) as usize];
        for y in 10..20 {
            for x in 5..35 {
                let i = ((y * w + x) * 3) as usize;
                data[i] = 10;
                data[i + 1] = 10;
                data[i + 2] = 10;
            }
        }
        let img = ImageBuffer::new(w, h, PixelFormat::Rgb8, data).unwrap();
        save_image(&src, &img, None, None).unwrap();
        let out = process_image_file(&ProcessOptions {
            src: src.clone(),
            dst: dst.clone(),
            pipeline: PipelinePrefs {
                deskew_angle: 8.0,
                ..PipelinePrefs::default()
            },
            quality: None,
        })
        .unwrap();
        let loaded = load_image(&out).unwrap();
        assert!(
            loaded.width > w || loaded.height > h,
            "explicit deskew degrees must expand canvas (got {}x{})",
            loaded.width,
            loaded.height
        );
    }

    #[test]
    fn export_options_apply_profile_and_keep_default_wrapper_compatible() {
        let dir = std::env::temp_dir().join("open_scanline_process_export_profile");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.png");
        let normal = dir.join("normal.png");
        let profiled = dir.join("profiled.png");
        let profile_path = dir.join("scanner-profile.json");
        let image = ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![160, 80, 40]).unwrap();
        save_image(&src, &image, None, None).unwrap();
        let mut profile =
            profile_scanner_it8(&ImageBuffer::new(6, 5, PixelFormat::Rgb8, vec![255; 90]).unwrap())
                .unwrap();
        profile["matrix"][0][0] = serde_json::json!(0.5);
        save_profile_json(&profile_path, &profile).unwrap();
        let request = ProcessOptions {
            src: src.clone(),
            dst: normal.clone(),
            pipeline: PipelinePrefs::default(),
            quality: None,
        };
        process_image_file(&request).unwrap();
        process_image_file_with_export_options(
            &ProcessOptions {
                dst: profiled.clone(),
                ..request
            },
            &ExportOptions {
                scanner_profile: Some(profile_path),
                ..ExportOptions::default()
            },
        )
        .unwrap();
        assert_eq!(load_image(normal).unwrap().data[0], 160);
        assert_eq!(load_image(profiled).unwrap().data[0], 80);
    }

    #[test]
    fn searchable_encrypted_process_pdf_uses_offline_ocr() {
        let dir = std::env::temp_dir().join("open_scanline_process_export_pdf");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.png");
        let dst = dir.join("out.pdf");
        save_image(
            &src,
            &ImageBuffer::new(8, 8, PixelFormat::Rgb8, vec![255; 192]).unwrap(),
            None,
            None,
        )
        .unwrap();
        process_image_file_with_export_options(
            &ProcessOptions {
                src,
                dst: dst.clone(),
                pipeline: PipelinePrefs::default(),
                quality: None,
            },
            &ExportOptions {
                pdf_password: Some("secret".into()),
                searchable_pdf: true,
                ocr_language: "eng".into(),
                ocr_engine: OcrEngine::Offline,
                scanner_profile: None,
            },
        )
        .unwrap();
        let document = Document::load_with_password(&dst, "secret").unwrap();
        assert!(document
            .extract_text(&[1])
            .unwrap()
            .contains("no text recognized"));
    }

    #[test]
    fn pdf_only_options_reject_non_pdf_before_output_mutation() {
        let dir = std::env::temp_dir().join("open_scanline_process_export_non_pdf");
        std::fs::create_dir_all(&dir).unwrap();
        let dst = dir.join("unchanged.png");
        std::fs::write(&dst, b"original").unwrap();
        let error = process_image_file_with_export_options(
            &ProcessOptions {
                src: dir.join("missing.png"),
                dst: dst.clone(),
                pipeline: PipelinePrefs::default(),
                quality: None,
            },
            &ExportOptions {
                searchable_pdf: true,
                ..ExportOptions::default()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("require a .pdf"));
        assert_eq!(std::fs::read(dst).unwrap(), b"original");
    }

    #[test]
    fn process_rejects_directory_or_unsupported_destination_before_loading_source() {
        let dir = std::env::temp_dir().join(format!(
            "open_scanline_process_destination_preflight_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for destination in [&dir, &dir.join("out.unsupported")] {
            let error = process_image_file(&ProcessOptions {
                src: dir.join("missing-source.png"),
                dst: destination.to_path_buf(),
                pipeline: PipelinePrefs::default(),
                quality: None,
            })
            .unwrap_err();
            assert!(
                error.to_string().contains("process output")
                    || error.to_string().contains("unsupported process output"),
                "unexpected preflight error: {error}"
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
