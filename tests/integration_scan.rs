//! Integration tests drive real shipped library entry points (not re-implementations).

use open_scanline::core::{ImageBuffer, PipelinePrefs, PixelFormat, Rect, Rotate, ScanRequest};
use open_scanline::device::{
    calibrate_device, exposure_from_preview, find_scanners, focus_device, list_all_devices,
    list_backends, open_device, DeviceSession,
};
use open_scanline::features::feature_matrix;
use open_scanline::film::list_film_profiles;
use open_scanline::i18n::Translator;
use open_scanline::icc::{make_it8_target_image, profile_scanner_it8};
use open_scanline::imaging::{load_image, save_image, save_pdf_with_options, PdfOptions};
use open_scanline::ocr::ocr_file;
use open_scanline::pipeline::{
    adjust_brightness_contrast, crop, descreen, histogram, infrared_clean, invert, rotate,
    white_balance,
};
use open_scanline::scan::{run_scan_to_file, ScanToFileArgs};
use std::path::PathBuf;

fn scratch_dir() -> PathBuf {
    let p = std::env::temp_dir().join("open_scanline_integration");
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn mock_scan_orchestration_writes_png() {
    let out = scratch_dir().join("orch_scan.png");
    let path = run_scan_to_file(ScanToFileArgs {
        out: out.clone(),
        width: 64,
        height: 48,
        seed: 7,
        ..ScanToFileArgs::default()
    })
    .expect("run_scan_to_file");
    assert!(path.is_file());
    let meta = std::fs::metadata(&path).unwrap();
    assert!(meta.len() > 0, "png must be non-empty");
    let head = std::fs::read(&path).unwrap();
    assert!(head.starts_with(b"\x89PNG\r\n\x1a\n"), "valid PNG magic");
    let img = load_image(&path).expect("load written png");
    assert_eq!(img.width, 64);
    assert_eq!(img.height, 48);
    assert_eq!(img.pixel_format, PixelFormat::Rgb8);
}

/// Shared scan path: invert once changes pixels; both sources still not identity.
#[test]
fn mock_scan_invert_changes_pixels_not_double() {
    let dir = scratch_dir();
    let plain = dir.join("inv_plain.png");
    let inv = dir.join("inv_on.png");
    let seed = 99u32;
    run_scan_to_file(ScanToFileArgs {
        out: plain.clone(),
        width: 40,
        height: 30,
        seed,
        ..ScanToFileArgs::default()
    })
    .unwrap();
    let prefs = PipelinePrefs {
        invert: true,
        ..PipelinePrefs::default()
    };
    run_scan_to_file(ScanToFileArgs {
        out: inv.clone(),
        width: 40,
        height: 30,
        seed,
        pipeline: prefs,
        ..ScanToFileArgs::default()
    })
    .unwrap();
    let a = load_image(&plain).unwrap();
    let b = load_image(&inv).unwrap();
    assert_ne!(a.data, b.data, "invert must change pixels");
    assert_eq!(b.data[0], 255 - a.data[0]);
    // both pipeline.invert + invert_colors → still single invert
    let both = dir.join("inv_both.png");
    let p2 = PipelinePrefs {
        invert: true,
        ..PipelinePrefs::default()
    };
    run_scan_to_file(ScanToFileArgs {
        device: Some("mock".into()),
        out: both.clone(),
        width: 40,
        height: 30,
        seed,
        pipeline: p2,
        invert_colors: true,
        ..ScanToFileArgs::default()
    })
    .unwrap();
    let c = load_image(&both).unwrap();
    assert_eq!(c.data, b.data, "must not double-invert to identity");
}

#[test]
fn pipeline_ops_on_real_buffer() {
    let mut data = vec![0u8; 4 * 2 * 3];
    data[0] = 255;
    data[1] = 0;
    data[2] = 0;
    data[3] = 0;
    data[4] = 255;
    data[5] = 0;
    data[6] = 0;
    data[7] = 0;
    data[8] = 255;
    for value in data.iter_mut().skip(9) {
        *value = 128;
    }
    let img = ImageBuffer::new(4, 2, PixelFormat::Rgb8, data).unwrap();
    let cropped = crop(&img, Rect::new(0, 0, 2, 2)).unwrap();
    assert_eq!(cropped.width, 2);
    assert_eq!(cropped.height, 2);
    let rotated = rotate(&cropped, Rotate::R90).unwrap();
    assert_eq!(rotated.width, 2);
    assert_eq!(rotated.height, 2);
    let bright = adjust_brightness_contrast(&rotated, 0, 0).unwrap();
    assert_eq!(bright.data, rotated.data);
    let inv = invert(&bright).unwrap();
    let inv2 = invert(&inv).unwrap();
    assert_eq!(inv2.data, bright.data);
    let wb = white_balance(&img).unwrap();
    assert_eq!(wb.width, img.width);
    let h = histogram(&img).unwrap();
    assert_eq!(h["count"], 8);
    let cleaned = infrared_clean(&img, Some("light")).unwrap();
    assert_eq!(cleaned.width, img.width);
    let ds = descreen(&img, 75).unwrap();
    assert_eq!(ds.height, img.height);
}

#[test]
fn imaging_save_load_roundtrip() {
    let w: u32 = 16;
    let h: u32 = 12;
    let mut data = vec![0u8; (w * h * 3) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 3) as usize;
            data[i] = (x * 255 / (w - 1)) as u8;
            data[i + 1] = (y * 255 / (h - 1)) as u8;
            data[i + 2] = 42;
        }
    }
    let img = ImageBuffer::new(w, h, PixelFormat::Rgb8, data).unwrap();
    let path = scratch_dir().join("roundtrip.png");
    save_image(&path, &img, Some(150), None).unwrap();
    let loaded = load_image(&path).unwrap();
    assert_eq!(loaded.width, w);
    assert_eq!(loaded.height, h);
    assert_eq!(loaded.pixel_format, PixelFormat::Rgb8);
    assert_eq!(loaded.data.len(), (w * h * 3) as usize);

    let pdf = scratch_dir().join("roundtrip.pdf");
    save_pdf_with_options(
        &pdf,
        &[img],
        &PdfOptions {
            title: "integration".into(),
            ..PdfOptions::default()
        },
    )
    .unwrap();
    let head = std::fs::read(&pdf).unwrap();
    assert!(head.starts_with(b"%PDF-"));
}

#[test]
fn mock_device_gradient_matches_contract() {
    let session = open_device("mock").unwrap();
    let req = ScanRequest {
        device_id: "mock".into(),
        width: 10,
        height: 5,
        dpi_x: 150,
        dpi_y: 150,
        seed: 99,
        pixel_format: PixelFormat::Rgb8,
        pipeline: PipelinePrefs::default(),
        ..Default::default()
    };
    let img = session.scan(&req).unwrap();
    assert_eq!(img.data[0], 0);
    assert_eq!(img.data[1], 0);
    assert_eq!(img.data[2], 99);
    let last = ((4) * 10 + 9) * 3;
    assert_eq!(img.data[last], 255);
    assert_eq!(img.data[last + 1], 255);
    assert_eq!(img.data[last + 2], 99);
}

#[test]
fn backends_list_includes_mock() {
    let backends = list_backends();
    assert!(backends.iter().any(|b| b.id == "mock" && b.available));
    assert!(backends.iter().any(|b| b.id == "wia"));
    assert!(backends.iter().any(|b| b.id == "sane"));
    assert!(backends.iter().any(|b| b.id == "escl"));
    let devices = list_all_devices();
    assert!(devices.iter().any(|d| d.id == "mock"));
    let found = find_scanners(true);
    assert!(found.iter().any(|d| d.id == "mock"));
}

#[test]
fn device_hooks_and_film_icc_i18n() {
    let cal = calibrate_device("mock");
    assert_eq!(cal["ok"], true);
    let foc = focus_device("mock", 0.4, 0.6);
    assert_eq!(foc["ok"], true);
    let req = ScanRequest {
        width: 32,
        height: 24,
        ..Default::default()
    };
    let exp = exposure_from_preview("mock", &req);
    assert_eq!(exp["ok"], true);

    let films = list_film_profiles();
    assert!(films.len() >= 50);
    let it8 = make_it8_target_image(60, 50).unwrap();
    let prof = profile_scanner_it8(&it8).unwrap();
    assert_eq!(prof["ok"], true);

    let translator = Translator::new("en");
    assert_eq!(translator.t("scan"), "Scan");
}

#[test]
fn feature_matrix_reports_runtime_availability() {
    let m = feature_matrix();
    let features = m["features"].as_array().unwrap();
    assert!(features.len() >= 20);
    assert!(features.iter().all(|feature| {
        feature["id"].is_string()
            && feature["available"].is_boolean()
            && feature["implementation"].is_string()
    }));
}

#[test]
fn ocr_offline_on_scanned_file() {
    let out = scratch_dir().join("ocr_src.png");
    run_scan_to_file(ScanToFileArgs {
        out: out.clone(),
        width: 80,
        height: 40,
        seed: 3,
        ..ScanToFileArgs::default()
    })
    .unwrap();
    let result = ocr_file(&out, "eng", true).unwrap();
    assert_eq!(result.engine, "offline-template");
    assert!(!result.text.is_empty());
}

#[test]
fn mock_calibrate_changes_scan_pixels() {
    let session = open_device("mock").unwrap();
    let req = ScanRequest {
        width: 24,
        height: 16,
        seed: 11,
        pixel_format: PixelFormat::Rgb8,
        ..Default::default()
    };
    let before = session.scan(&req).unwrap();
    let cal = session.calibrate();
    assert_eq!(cal["ok"], true);
    let after = session.scan(&req).unwrap();
    assert_ne!(before.data, after.data);
    session.close();
}

#[test]
fn backend_sessions_open_and_scan_sim() {
    for id in ["wia:sim", "sane:sim", "escl:sim"] {
        let session = open_device(id).expect(id);
        let req = ScanRequest {
            width: 16,
            height: 12,
            seed: 2,
            pixel_format: PixelFormat::Rgb8,
            ..Default::default()
        };
        let img = session.scan(&req).expect("scan");
        assert_eq!(img.width, 16);
        // Shared path via run_scan_to_file
        let out = scratch_dir().join(format!("{}_scan.png", id.replace(':', "_")));
        let path = run_scan_to_file(ScanToFileArgs {
            device: Some(id.into()),
            out: out.clone(),
            width: 16,
            height: 12,
            seed: 2,
            ..ScanToFileArgs::default()
        })
        .expect("run_scan_to_file backend");
        assert!(path.is_file());
        session.close();
    }
}

#[test]
fn filter_prefs_on_shared_scan_path() {
    let dir = scratch_dir();
    let plain_out = dir.join("filter_scan_plain.png");
    let filter_out = dir.join("filter_scan.png");
    let w = 32u32;
    let h = 24u32;
    let seed = 4u32;
    run_scan_to_file(ScanToFileArgs {
        out: plain_out.clone(),
        width: w,
        height: h,
        seed,
        ..ScanToFileArgs::default()
    })
    .unwrap();
    let prefs = PipelinePrefs {
        infrared_clean: Some("medium".into()),
        descreen: true,
        descreen_dpi: 75,
        ..PipelinePrefs::default()
    };
    let path = run_scan_to_file(ScanToFileArgs {
        out: filter_out,
        width: w,
        height: h,
        seed,
        pipeline: prefs,
        ..ScanToFileArgs::default()
    })
    .unwrap();
    assert!(path.is_file());
    let plain = load_image(&plain_out).unwrap();
    let img = load_image(&path).unwrap();
    assert_eq!(img.width, w);
    assert_eq!(img.height, h);
    assert_eq!(plain.width, w);
    assert_eq!(plain.height, h);
    assert_ne!(
        plain.data, img.data,
        "filter prefs (IR+descreen) must change pixels vs plain scan"
    );
}

/// Shared scan → save_image path with .pdf extension must write real PDF magic.
#[test]
fn mock_scan_to_pdf_via_shared_path() {
    use open_scanline::imaging::convert_image;
    let pdf = scratch_dir().join("orch_scan.pdf");
    let path = run_scan_to_file(ScanToFileArgs {
        out: pdf,
        width: 40,
        height: 30,
        seed: 5,
        ..ScanToFileArgs::default()
    })
    .expect("run_scan_to_file pdf");
    assert!(path.is_file());
    let head = std::fs::read(&path).unwrap();
    assert!(
        head.starts_with(b"%PDF-"),
        "scan to .pdf must be PDF, got {:?}",
        &head[..head.len().min(8)]
    );
    assert!(
        !head.starts_with(b"\x89PNG"),
        "must not be PNG bytes under .pdf name"
    );

    // convert shared path png → pdf
    let png = scratch_dir().join("orch_for_conv.png");
    run_scan_to_file(ScanToFileArgs {
        out: png.clone(),
        width: 20,
        height: 16,
        ..ScanToFileArgs::default()
    })
    .unwrap();
    let conv = scratch_dir().join("orch_conv.pdf");
    convert_image(&png, &conv, Some(150)).unwrap();
    let ch = std::fs::read(&conv).unwrap();
    assert!(ch.starts_with(b"%PDF-"), "convert→pdf magic");
}
