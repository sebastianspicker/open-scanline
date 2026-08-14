use super::codecs::{
    create_output_temp, save_jpeg_xl_with_program, save_jpeg_xl_with_program_and_cancellation,
    JPEG_MAGIC, TIFF_MAGIC_LE,
};
use super::tiff::load_tiff_frames;
use super::*;
use crate::core::{ImageBuffer, PixelFormat};
use crate::device::CancellationToken;
use std::env;
use std::path::{Path, PathBuf};

fn scratch() -> PathBuf {
    let p = env::temp_dir().join("open_scanline_imaging_test");
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn sample_rgb(w: u32, h: u32) -> ImageBuffer {
    let mut data = vec![0u8; (w * h * 3) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 3) as usize;
            data[i] = (x.saturating_mul(255) / w.max(1)) as u8;
            data[i + 1] = (y.saturating_mul(255) / h.max(1)) as u8;
            data[i + 2] = 42;
        }
    }
    ImageBuffer::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

#[test]
fn png_magic_helpers() {
    assert!(is_png_magic(PNG_MAGIC));
    assert!(!is_png_magic(b"notapng!"));
    assert_eq!(detect_format_bytes(PNG_MAGIC), Some("png"));
    assert_eq!(detect_format_bytes(JPEG_MAGIC), Some("jpeg"));
    assert_eq!(detect_format_bytes(b"II"), Some("tiff"));
    assert_eq!(detect_format_bytes(b"MM"), Some("tiff"));
    assert_eq!(
        detect_format_bytes(b"II\x2a\x00\x00\x00\x00\x00"),
        Some("tiff")
    );
    assert_eq!(detect_format_bytes(b"GIF87a......"), Some("gif"));
    assert_eq!(detect_format_bytes(b"GIF89a......"), Some("gif"));
    assert_eq!(detect_format_bytes(b"BM.........."), Some("bmp"));
    assert_eq!(detect_format_bytes(b"RIFF....WEBP"), Some("webp"));
    assert_eq!(detect_format_bytes(b"%PDF-1.5"), Some("pdf"));
    assert_eq!(detect_format_bytes(b"\xff\x0a"), Some("jxl"));
    assert_eq!(
        detect_format_bytes(b"\0\0\0\x0cJXL \r\n\x87\n"),
        Some("jxl")
    );
    assert_eq!(detect_format_bytes(b"RIFF....WAVE"), None);
    assert_eq!(detect_format_bytes(b"not a known container"), None);
}

#[test]
fn save_load_png_roundtrip() {
    let dir = scratch();
    let path = dir.join("roundtrip.png");
    let img = sample_rgb(16, 12);
    save_image(&path, &img, Some(150), None).unwrap();
    validate_png_magic(&path).unwrap();
    let head = read_magic(&path).unwrap();
    assert!(is_png_magic(&head));
    assert_eq!(detect_format(&path).unwrap(), Some("png"));

    let loaded = load_image(&path).unwrap();
    assert_eq!(loaded.width, 16);
    assert_eq!(loaded.height, 12);
    assert_eq!(loaded.pixel_format, PixelFormat::Rgb8);
    assert_eq!(loaded.data.len(), 16 * 12 * 3);
    // Lossless PNG should match exactly.
    assert_eq!(loaded.data, img.data);
}

#[test]
fn save_pdf_writes_valid_header() {
    let img = sample_rgb(32, 24);
    let path = scratch().join("page.pdf");
    let out = save_pdf_with_options(
        &path,
        &[img],
        &PdfOptions {
            title: "test".into(),
            ..PdfOptions::default()
        },
    )
    .unwrap();
    assert!(out.is_file());
    let head = std::fs::read(&out).unwrap();
    assert!(head.starts_with(b"%PDF-"));
    assert!(head.len() > 100);
}

/// Shared save_image path must route .pdf to real PDF (not PNG under .pdf name).
#[test]
fn save_image_pdf_extension_writes_pdf_magic() {
    let img = sample_rgb(20, 16);
    let path = scratch().join("via_save_image.pdf");
    let out = save_image(&path, &img, Some(150), None).unwrap();
    assert!(out.is_file());
    let head = std::fs::read(&out).unwrap();
    assert!(
        head.starts_with(b"%PDF-"),
        "save_image(.pdf) must write PDF magic, got {:?}",
        &head[..head.len().min(8)]
    );
    assert!(!is_png_magic(&head), "must not be PNG masquerading as PDF");
}

#[test]
fn convert_image_to_pdf_writes_pdf_magic() {
    let img = sample_rgb(12, 10);
    let png = scratch().join("src_for_pdf.png");
    let pdf = scratch().join("conv_out.pdf");
    save_image(&png, &img, Some(150), None).unwrap();
    convert_image(&png, &pdf, Some(150)).unwrap();
    let head = std::fs::read(&pdf).unwrap();
    assert!(
        head.starts_with(b"%PDF-"),
        "convert→pdf got {:?}",
        &head[..8.min(head.len())]
    );
}

#[test]
fn save_load_jpeg_tiff_convert_multipage() {
    let dir = scratch();
    let jpg = dir.join("r.jpg");
    let tif = dir.join("r.tif");
    let png = dir.join("c.png");
    let img = sample_rgb(8, 8);
    save_image(&jpg, &img, Some(72), Some(85)).unwrap();
    assert_eq!(&read_magic(&jpg).unwrap()[..2], JPEG_MAGIC);
    let loaded = load_image(&jpg).unwrap();
    assert_eq!((loaded.width, loaded.height), (8, 8));

    save_image(&tif, &img, Some(150), None).unwrap();
    assert_eq!(detect_format(&tif).unwrap(), Some("tiff"));
    convert_image(&tif, &png, Some(150)).unwrap();
    validate_png_magic(&png).unwrap();

    let p1 = dir.join("mp1.png");
    let p2 = dir.join("mp2.png");
    let multi = dir.join("multi.tif");
    save_image(&p1, &sample_rgb(4, 4), None, None).unwrap();
    save_image(&p2, &sample_rgb(4, 4), None, None).unwrap();
    save_multipage_tiff(&[p1, p2], &multi, Some(100)).unwrap();
    assert_eq!(&read_magic(&multi).unwrap()[..2], TIFF_MAGIC_LE);
    let page = load_image(&multi).unwrap();
    assert_eq!((page.width, page.height), (4, 4));
}

#[test]
fn every_builtin_image_encoder_writes_matching_magic_and_roundtrips() {
    let dir = scratch();
    let image = sample_rgb(10, 7);
    for (extension, expected) in [
        ("png", "png"),
        ("jpg", "jpeg"),
        ("tif", "tiff"),
        ("webp", "webp"),
        ("bmp", "bmp"),
        ("gif", "gif"),
    ] {
        let path = dir.join(format!("format-roundtrip.{extension}"));
        save_image(&path, &image, Some(150), Some(90)).unwrap();
        assert_eq!(detect_format(&path).unwrap(), Some(expected));
        let loaded = load_image(&path).unwrap();
        assert_eq!((loaded.width, loaded.height), (10, 7));
    }
    let error = save_image(dir.join("unknown.xyz"), &image, None, None).unwrap_err();
    assert!(error.to_string().contains("unsupported extension"));
}

#[test]
fn missing_cjxl_is_an_explicit_optional_dependency_error() {
    let path = scratch().join("missing-cjxl.jxl");
    let error = save_jpeg_xl_with_program(
        &path,
        &sample_rgb(4, 4),
        90,
        Path::new("definitely-not-an-installed-cjxl"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("optional 'cjxl' executable"));
}

#[test]
fn failed_jpeg_xl_export_preserves_existing_output_and_cleans_sibling_temps() {
    let dir = scratch().join(format!("jxl-atomic-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("existing.jxl");
    std::fs::write(&path, b"previous JXL output").unwrap();

    let error = save_jpeg_xl_with_program(
        &path,
        &sample_rgb(4, 4),
        90,
        Path::new("definitely-not-an-installed-cjxl"),
    )
    .unwrap_err();

    assert!(error.to_string().contains("optional 'cjxl' executable"));
    assert_eq!(std::fs::read(&path).unwrap(), b"previous JXL output");
    assert_eq!(
        std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().contains("open-scanline"))
            .count(),
        0,
        "failed export must not leak sibling temporary files"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn jpeg_xl_cancellation_kills_fake_hanging_cjxl() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch();
    let script = dir.join(format!("fake-cjxl-cancel-{}.sh", std::process::id()));
    std::fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let token = CancellationToken::new();
    let trigger = token.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        trigger.cancel();
    });
    let started = std::time::Instant::now();
    let error = save_jpeg_xl_with_program_and_cancellation(
        &dir.join("cancelled.jxl"),
        &sample_rgb(4, 4),
        90,
        &script,
        Some(&token),
    )
    .unwrap_err();
    canceller.join().unwrap();
    std::fs::remove_file(script).unwrap();

    assert!(
        matches!(error, crate::core::ScanError::Cancelled(message) if message == "JPEG XL export cancelled")
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

#[cfg(unix)]
#[test]
fn failed_jpeg_xl_export_preserves_existing_destination_mode() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch().join(format!("jxl-mode-failure-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("existing.jxl");
    std::fs::write(&path, b"previous JXL output").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();

    save_jpeg_xl_with_program(
        &path,
        &sample_rgb(4, 4),
        90,
        Path::new("definitely-not-an-installed-cjxl"),
    )
    .unwrap_err();

    assert_eq!(std::fs::read(&path).unwrap(), b"previous JXL output");
    assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o640);
    assert_eq!(
        std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().contains("open-scanline"))
            .count(),
        0,
        "failed export must not leak sibling temporary files"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn sibling_temp_reservation_keeps_the_effective_extension_and_cleans_up() {
    let dir = scratch().join(format!("temp-extension-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let destination = dir.join("scan.tiff");
    let temp = create_output_temp(&destination).unwrap();
    let temporary_path = temp.path().to_path_buf();

    assert_eq!(temporary_path.parent(), destination.parent());
    assert_eq!(temporary_path.extension(), destination.extension());
    assert!(temporary_path.is_file());
    drop(temp);
    assert!(!temporary_path.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn atomic_output_temps_are_private_and_preserve_existing_destination_mode() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch().join(format!("atomic-modes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let fresh = dir.join("fresh.png");
    let temp = create_output_temp(&fresh).unwrap();
    assert_eq!(
        temp.path().metadata().unwrap().permissions().mode() & 0o777,
        0o600
    );
    drop(temp);

    let image = sample_rgb(4, 3);
    for (path, mode) in [
        (dir.join("image.png"), 0o640),
        (dir.join("document.pdf"), 0o604),
        (dir.join("multipage.tiff"), 0o660),
    ] {
        std::fs::write(&path, b"previous output").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        match path.extension().and_then(|extension| extension.to_str()) {
            Some("png") => {
                save_image(&path, &image, None, None).unwrap();
            }
            Some("pdf") => {
                save_pdf_with_options(&path, std::slice::from_ref(&image), &PdfOptions::default())
                    .unwrap();
            }
            Some("tiff") => {
                let source = dir.join("source.png");
                save_image(&source, &image, None, None).unwrap();
                save_multipage_tiff(&[source], &path, None).unwrap();
            }
            _ => unreachable!(),
        }
        assert_eq!(
            path.metadata().unwrap().permissions().mode() & 0o777,
            mode,
            "{} did not preserve destination permissions",
            path.display()
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn contact_sheet_raw_and_tiff_append_are_real_outputs() {
    let dir = scratch();
    let first = dir.join("sheet-a.png");
    let second = dir.join("sheet-b.png");
    save_image(&first, &sample_rgb(6, 4), None, None).unwrap();
    save_image(&second, &sample_rgb(6, 4), None, None).unwrap();
    let sheet = save_index_contact_sheet(&[first, second], dir.join("contact"), 2, 20, Some(16), 2)
        .unwrap();
    assert_eq!(detect_format(&sheet).unwrap(), Some("bmp"));

    let raw = save_raw_image(dir.join("raw-page"), &sample_rgb(5, 3), Some(300)).unwrap();
    assert_eq!(detect_format(&raw).unwrap(), Some("tiff"));
    let appended = dir.join("appended.tif");
    let _ = std::fs::remove_file(&appended);
    append_page_to_multipage(&appended, &sample_rgb(5, 3), None, Some(150)).unwrap();
    append_page_to_multipage(&appended, &sample_rgb(5, 3), None, Some(150)).unwrap();
    assert_eq!(load_tiff_frames(&appended).unwrap().len(), 2);
}

#[test]
fn cancelled_contact_sheet_preserves_existing_destination() {
    let dir = scratch();
    let first = dir.join("cancel-sheet-a.png");
    let second = dir.join("cancel-sheet-b.png");
    save_image(&first, &sample_rgb(6, 4), None, None).unwrap();
    save_image(&second, &sample_rgb(6, 4), None, None).unwrap();
    let destination = dir.join("cancel-sheet.bmp");
    std::fs::write(&destination, b"previous contact sheet").unwrap();
    let token = CancellationToken::new();
    token.cancel();

    let error = save_index_contact_sheet_with_cancellation(
        &[first, second],
        &destination,
        2,
        20,
        Some(16),
        2,
        Some(&token),
    )
    .unwrap_err();

    assert!(matches!(error, crate::core::ScanError::Cancelled(_)));
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        b"previous contact sheet"
    );
}
