use super::state::GuiState;
use super::*;
use crate::config::{load_config, save_config, AppConfig};
use crate::core::ScanMode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[test]
fn unsafe_output_name_cannot_escape_selected_directory_or_mutate_sentinel() {
    let root = std::env::temp_dir().join(format!(
        "open_scanline_gui_output_name_{}",
        std::process::id()
    ));
    let safe_dir = root.join("safe");
    std::fs::create_dir_all(&safe_dir).unwrap();
    let outside = root.join("outside_scan_000.png");
    std::fs::write(&outside, "outside sentinel").unwrap();

    let mut state = GuiState::new(None);
    state.output_dir = safe_dir.display().to_string();
    state.output_name = "../outside".into();
    assert!(state.out_path("scan").is_err());
    assert!(state.out_path_plain("save").is_err());
    assert!(state.batch_args(safe_dir.join("batch"), 1).is_err());

    state.do_scan(false);
    assert!(state.status.contains("invalid"));
    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "outside sentinel"
    );
}

#[test]
fn open_save_plus_cancel_real_handlers() {
    use crate::core::{ImageBuffer, PipelinePrefs, PixelFormat};
    use crate::scan::{run_scan_to_file, ScanToFileArgs};
    let dir = std::env::temp_dir().join("open_scanline_gui_actions");
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("opened.png");
    run_scan_to_file(ScanToFileArgs {
        out: src.clone(),
        width: 24,
        height: 16,
        seed: 3,
        ..ScanToFileArgs::default()
    })
    .unwrap();
    assert!(src.is_file());
    assert_eq!(open_image_file(&src).expect("open_image_file"), src);
    let mut state = GuiState::new(None);
    state.output_dir = dir.display().to_string();
    state.output_name = "gui_act".into();
    state.frame_index = 0;
    state.do_open_file(Some(&src));
    assert_eq!(state.last_image.as_ref(), Some(&src));
    assert!(state.status.contains(&state.translator.t("status.done")));
    let before_frame = state.frame_index;
    state.do_save_plus();
    assert_eq!(state.frame_index, before_frame + 1);
    assert!(state.status.contains(&state.translator.t("status.done")));
    let saved = state.last_image.clone().expect("saved path");
    assert!(saved.is_file(), "Save+ must write a real file");
    let magic = std::fs::read(&saved).unwrap();
    assert!(
        magic.starts_with(b"\x89PNG")
            || magic.starts_with(b"\xff\xd8")
            || magic.starts_with(b"%PDF")
    );
    state.scanning = false;
    state.do_cancel();
    assert!(state.cancel_requested.load(Ordering::SeqCst));
    assert_eq!(state.status, state.translator.t("status.ready"));
    let flag = Arc::new(AtomicBool::new(true));
    let err = run_scan_to_file(ScanToFileArgs {
        device: Some("mock".into()),
        out: dir.join("cancelled.png"),
        width: 16,
        height: 12,
        seed: 1,
        dpi: 150,
        mode: ScanMode::Reflective,
        duplex: false,
        pipeline: PipelinePrefs::default(),
        invert_colors: false,
        use_preview: false,
        config: None,
        on_progress: None,
        cancel_check: Some(Box::new(move || flag.load(Ordering::SeqCst))),
        raw_out: None,
    });
    assert!(err.is_err());
    assert!(err
        .unwrap_err()
        .to_string()
        .to_lowercase()
        .contains("cancel"));
    let _ = ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![0, 0, 0]);
}

#[test]
fn typed_gui_state_builds_pipeline_preferences() {
    let mut state = GuiState::new(None);
    state.crop_text = "0,0,50,40".into();
    state.rotate = 90;
    state.auto_deskew = true;
    state.white_balance = true;
    state.saturation = 35.0;
    state.hue = -45.0;
    state.curves_text = "0:0,128:160,255:255".into();
    state.infrared_clean = "medium".into();
    state.sharpen_amount = 0.5;
    state.descreen = true;
    let prefs = state.pipeline_prefs();
    assert!(prefs.auto_deskew);
    assert!(prefs.white_balance);
    assert!((prefs.sharpen_amount - 0.5).abs() < 1e-9);
    assert_eq!(prefs.saturation, 35.0);
    assert_eq!(prefs.hue, -45.0);
    assert_eq!(prefs.curves, Some(vec![[0, 0], [128, 160], [255, 255]]));
    assert_eq!(prefs.infrared_clean.as_deref(), Some("medium"));
    assert!(prefs.descreen);
}

#[test]
fn gui_scan_args_do_not_reapply_stale_config_pipeline_effects() {
    use crate::imaging::load_image;
    use crate::scan::{run_scan_to_file, ScanToFileArgs};

    let directory = std::env::temp_dir().join(format!(
        "open_scanline_gui_stale_scan_config_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("create GUI scan directory");

    let mut state = GuiState::new(None);
    state.output_dir = directory.display().to_string();
    state.output_name = "gui".into();
    state.width = 32;
    state.height = 24;
    state.config.invert_colors = true;
    state.config.white_balance = true;
    state.invert_colors = false;
    state.white_balance = false;

    let gui_args = state.scan_args(false).expect("build GUI scan arguments");
    assert!(gui_args.config.is_none());
    assert!(!gui_args.invert_colors);
    assert!(!gui_args.pipeline.invert);
    assert!(!gui_args.pipeline.white_balance);

    let expected = directory.join("expected.png");
    let expected_args = ScanToFileArgs {
        device: gui_args.device.clone(),
        out: expected.clone(),
        width: gui_args.width,
        height: gui_args.height,
        seed: gui_args.seed,
        dpi: gui_args.dpi,
        mode: gui_args.mode,
        duplex: gui_args.duplex,
        pipeline: gui_args.pipeline.clone(),
        invert_colors: gui_args.invert_colors,
        use_preview: gui_args.use_preview,
        config: None,
        on_progress: None,
        cancel_check: None,
        raw_out: None,
    };

    let gui_output = gui_args.out.clone();
    run_scan_to_file(gui_args).expect("scan using current GUI pipeline");
    run_scan_to_file(expected_args).expect("scan without saved pipeline effects");
    assert_eq!(
        load_image(gui_output).expect("load GUI scan"),
        load_image(expected).expect("load expected scan"),
        "saved invert and white-balance settings must not affect unchecked GUI controls"
    );
}

#[test]
fn gui_state_constructor_normalizes_configured_defaults() {
    let path = std::env::temp_dir().join(format!(
        "open_scanline_gui_state_constructor_{}.json",
        std::process::id()
    ));
    let config = AppConfig {
        last_device_id: String::new(),
        default_width: 16,
        default_height: 16,
        default_dpi: crate::core::MIN_SCAN_DPI,
        output_dir: String::new(),
        language: "pt-BR".into(),
        ..AppConfig::default()
    };
    save_config(&config, &path).expect("write GUI constructor config");

    let state = GuiState::new(Some(&path));
    let expected_device = state
        .devices
        .first()
        .cloned()
        .unwrap_or_else(|| "mock".into());
    assert_eq!(state.config_path, path);
    assert_eq!(state.config, config);
    assert_eq!(state.device, expected_device);
    assert_eq!(
        (state.width, state.height, state.dpi),
        (16, 16, crate::core::MIN_SCAN_DPI)
    );
    assert_eq!(state.output_dir, ".");
    assert_eq!(state.language, "pb");
    assert_eq!(state.translator.language(), "pb");
    assert_eq!(state.status, state.translator.t("ready"));
    assert!(!state.cancel_requested.load(Ordering::SeqCst));

    let _ = std::fs::remove_file(path);
}

#[test]
fn gui_batch_args_keep_configured_dimensions_and_source() {
    let mut state = GuiState::new(None);
    state.width = 1200;
    state.height = 900;
    state.media = "film".into();
    state.duplex = false;

    let args = state.batch_args(std::env::temp_dir(), 2).unwrap();

    assert_eq!((args.width, args.height), (1200, 900));
    assert_eq!(args.mode, ScanMode::Film);
    assert!(!args.duplex);
}

#[test]
fn gui_pdf_batch_selection_creates_a_real_multipage_pdf_destination() {
    let mut state = GuiState::new(None);
    state.output_name = "document".into();
    state.output_fmt = "pdf".into();
    state.multipage = false;
    state.searchable_pdf = true;
    let directory = std::env::temp_dir().join("open_scanline_gui_pdf_batch_args");

    let args = state.batch_args(directory.clone(), 2).unwrap();

    assert_eq!(args.format, "png");
    assert_eq!(
        args.multipage_out,
        Some(directory.join("document_multipage.pdf"))
    );
}

#[test]
fn gui_offline_ocr_runs_without_tesseract() {
    use crate::ocr::OFFLINE_OCR_ENGINE;
    use crate::scan::{run_scan_to_file, ScanToFileArgs};

    let path = std::env::temp_dir().join(format!(
        "open_scanline_gui_offline_ocr_{}.png",
        std::process::id()
    ));
    run_scan_to_file(ScanToFileArgs {
        out: path.clone(),
        width: 16,
        height: 12,
        ..ScanToFileArgs::default()
    })
    .expect("write OCR input");

    let mut state = GuiState::new(None);
    state.last_image = Some(path.clone());
    state.ocr_engine = "offline".into();
    state.ocr_language = "deu".into();
    assert_eq!(state.ocr_options(), ("deu", true));
    state.do_ocr();

    assert!(state.status.contains(OFFLINE_OCR_ENGINE));
    state.ocr_engine = "tesseract".into();
    state.ocr_language.clear();
    assert_eq!(state.ocr_options(), ("eng", false));
    let _ = std::fs::remove_file(path);
}

#[test]
fn gui_export_password_is_runtime_only_and_consumed_by_snapshot() {
    let path = std::env::temp_dir().join(format!(
        "open_scanline_gui_runtime_export_{}.json",
        std::process::id()
    ));
    let mut state = GuiState::new(Some(&path));
    state.pdf_password = "temporary-password".into();
    state.searchable_pdf = true;
    state.ocr_engine = "tesseract".into();
    state.ocr_language = "deu".into();
    state.scanner_profile_path = "/tmp/scanner-profile.json".into();

    let export = state.take_export_options();
    assert_eq!(export.pdf_password.as_deref(), Some("temporary-password"));
    assert!(export.searchable_pdf);
    assert_eq!(export.ocr_language, "deu");
    assert_eq!(
        export.scanner_profile.as_deref(),
        Some(std::path::Path::new("/tmp/scanner-profile.json"))
    );
    assert!(state.pdf_password.is_empty());

    state.do_save_config();
    assert!(!std::fs::read_to_string(&path)
        .unwrap()
        .contains("temporary-password"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn gui_failed_nonmultipage_save_and_reprocess_restore_password() {
    let dir = gui_test_directory("nonmultipage_password_retry");
    let source = dir.join("source.png");
    save_rgb(&source, &[10, 20, 30]);

    let mut state = GuiState::new(None);
    state.output_dir = dir.display().to_string();
    state.output_name = "retry".into();
    state.output_fmt = "pdf".into();
    state.scanner_profile_path = dir.join("missing-profile.json").display().to_string();
    state.last_image = Some(source.clone());

    state.pdf_password = "save-secret".into();
    state.do_save_plus();
    assert_eq!(state.pdf_password, "save-secret");
    assert_eq!(state.last_image.as_ref(), Some(&source));

    state.pdf_password = "reprocess-secret".into();
    state.do_reprocess();
    assert_eq!(state.pdf_password, "reprocess-secret");
    assert_eq!(state.last_image.as_ref(), Some(&source));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn gui_successful_nonmultipage_save_consumes_password() {
    let dir = gui_test_directory("nonmultipage_password_success");
    let source = dir.join("source.png");
    save_rgb(&source, &[10, 20, 30]);

    let mut state = GuiState::new(None);
    state.output_dir = dir.display().to_string();
    state.output_name = "success".into();
    state.output_fmt = "pdf".into();
    state.pdf_password = "success-secret".into();
    state.last_image = Some(source);
    state.do_save_plus();

    assert!(state.pdf_password.is_empty());
    assert!(dir.join("success_save_000.pdf").is_file());

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn gui_save_plus_rebuilds_a_searchable_encrypted_two_page_pdf() {
    use std::process::Command;

    let dir = gui_test_directory("multipage_pdf");
    let first = dir.join("first.png");
    let second = dir.join("second.png");
    save_rgb(&first, &[20, 30, 40]);
    save_rgb(&second, &[60, 70, 80]);

    let mut state = GuiState::new(None);
    state.output_dir = dir.display().to_string();
    state.output_name = "combined".into();
    state.multipage = true;
    state.multipage_format = "pdf".into();
    state.searchable_pdf = true;
    state.pdf_password = "runtime-only-secret".into();
    state.last_image = Some(first.clone());

    state.do_save_plus();
    assert_eq!(state.last_image.as_ref(), Some(&first));
    state.do_open_file(Some(&second));
    state.do_save_plus();

    let output = dir.join("combined_multipage.pdf");
    let info = Command::new("pdfinfo")
        .args(["-upw", "runtime-only-secret"])
        .arg(&output)
        .output()
        .expect("run pdfinfo");
    assert!(
        info.status.success(),
        "pdfinfo: {}",
        String::from_utf8_lossy(&info.stderr)
    );
    let info = String::from_utf8_lossy(&info.stdout);
    assert!(
        info.lines().any(|line| line.trim() == "Pages:           2"),
        "pdfinfo: {info}"
    );
    assert!(
        info.lines()
            .any(|line| line.trim_start().starts_with("Encrypted:") && line.contains("yes")),
        "pdfinfo: {info}"
    );
    let text = Command::new("pdftotext")
        .args(["-upw", "runtime-only-secret"])
        .arg(&output)
        .arg("-")
        .output()
        .expect("run pdftotext");
    assert!(
        text.status.success() && !text.stdout.is_empty(),
        "searchable PDF did not expose OCR text: {}",
        String::from_utf8_lossy(&text.stderr)
    );
    assert!(state.pdf_password.is_empty());
    assert_eq!(state.frame_index, 2);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn gui_multipage_tiff_applies_the_scanner_profile_to_every_final_page() {
    use crate::icc::{save_profile_json, PROFILE_FORMAT, PROFILE_VERSION};
    use serde_json::json;
    use tiff::decoder::{Decoder, DecodingResult};

    let dir = gui_test_directory("multipage_profile");
    let source = dir.join("source.png");
    let profile = dir.join("profile.json");
    save_rgb(&source, &[100, 50, 0]);
    save_profile_json(
        &profile,
        &json!({
            "format": PROFILE_FORMAT,
            "version": PROFILE_VERSION,
            "kind": "scanner_it8",
            "matrix": [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            "gamma": [1.0, 1.0, 1.0]
        }),
    )
    .expect("write scanner profile");

    let mut state = GuiState::new(None);
    state.output_dir = dir.display().to_string();
    state.output_name = "profiled".into();
    state.multipage = true;
    state.multipage_format = "tif".into();
    state.scanner_profile_path = profile.display().to_string();
    state.last_image = Some(source);
    state.do_save_plus();

    let output = dir.join("profiled_multipage.tif");
    let mut decoder = Decoder::new(std::io::BufReader::new(
        std::fs::File::open(output).expect("open profiled TIFF"),
    ))
    .expect("decode profiled TIFF");
    match decoder.read_image().expect("read profiled TIFF page") {
        DecodingResult::U8(bytes) => assert_eq!(bytes, vec![50, 100, 0]),
        _ => panic!("profiled TIFF did not contain 8-bit RGB pixels"),
    }

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn gui_multipage_destination_change_starts_a_new_source_accumulator() {
    let first_dir = gui_test_directory("multipage_destination_first");
    let second_dir = gui_test_directory("multipage_destination_second");
    let first = first_dir.join("first.png");
    let second = second_dir.join("second.png");
    save_rgb(&first, &[1, 2, 3]);
    save_rgb(&second, &[4, 5, 6]);

    let mut state = GuiState::new(None);
    state.multipage = true;
    state.multipage_format = "pdf".into();
    state.output_name = "document".into();
    state.output_dir = first_dir.display().to_string();
    state.last_image = Some(first);
    state.do_save_plus();
    state.output_dir = second_dir.display().to_string();
    state.last_image = Some(second);
    state.do_save_plus();

    assert_eq!(
        lopdf::Document::load(first_dir.join("document_multipage.pdf"))
            .unwrap()
            .get_pages()
            .len(),
        1
    );
    assert_eq!(
        lopdf::Document::load(second_dir.join("document_multipage.pdf"))
            .unwrap()
            .get_pages()
            .len(),
        1
    );

    let _ = std::fs::remove_dir_all(first_dir);
    let _ = std::fs::remove_dir_all(second_dir);
}

#[test]
fn gui_multipage_save_failure_does_not_advance_the_frame_or_create_a_directory() {
    let dir = std::env::temp_dir().join(format!(
        "open_scanline_gui_multipage_failure_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let source_dir = gui_test_directory("multipage_failure_source");
    let source = source_dir.join("source.png");
    save_rgb(&source, &[10, 20, 30]);

    let mut state = GuiState::new(None);
    state.output_dir = dir.display().to_string();
    state.output_name = "failure".into();
    state.multipage = true;
    state.multipage_format = "pdf".into();
    state.scanner_profile_path = source_dir
        .join("missing-profile.json")
        .display()
        .to_string();
    state.last_image = Some(source.clone());
    state.frame_index = 7;

    state.do_save_plus();

    assert_eq!(state.frame_index, 7);
    assert_eq!(state.last_image.as_ref(), Some(&source));
    assert!(
        !dir.exists(),
        "invalid profile must not create an output directory"
    );
    assert!(state.status.contains("Error"));

    let _ = std::fs::remove_dir_all(source_dir);
}

#[test]
fn gui_multipage_failed_encrypted_save_keeps_password_for_same_destination_retry() {
    use crate::icc::{save_profile_json, PROFILE_FORMAT, PROFILE_VERSION};
    use serde_json::json;
    use std::process::Command;

    let dir = gui_test_directory("multipage_retry_password");
    let source = dir.join("source.png");
    let profile = dir.join("profile.json");
    let output = dir.join("retry_multipage.pdf");
    save_rgb(&source, &[10, 20, 30]);

    let mut state = GuiState::new(None);
    state.output_dir = dir.display().to_string();
    state.output_name = "retry".into();
    state.multipage = true;
    state.multipage_format = "pdf".into();
    state.pdf_password = "retry-secret".into();
    state.scanner_profile_path = profile.display().to_string();
    state.last_image = Some(source.clone());

    state.do_save_plus();
    assert_eq!(state.pdf_password, "retry-secret");
    assert_eq!(state.last_image.as_ref(), Some(&source));
    assert!(!output.exists());

    save_profile_json(
        &profile,
        &json!({
            "format": PROFILE_FORMAT,
            "version": PROFILE_VERSION,
            "kind": "scanner_it8",
            "matrix": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            "gamma": [1.0, 1.0, 1.0]
        }),
    )
    .expect("write corrected scanner profile");

    state.do_save_plus();
    let info = Command::new("pdfinfo")
        .args(["-upw", "retry-secret"])
        .arg(&output)
        .output()
        .expect("run pdfinfo");
    assert!(
        info.status.success(),
        "retry must retain encryption: {}",
        String::from_utf8_lossy(&info.stderr)
    );
    assert!(state.pdf_password.is_empty());

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn gui_multipage_destination_switch_after_failure_keeps_encryption() {
    use crate::icc::{save_profile_json, PROFILE_FORMAT, PROFILE_VERSION};
    use serde_json::json;
    use std::process::Command;

    let failed_dir = gui_test_directory("multipage_switch_failure");
    let destination_dir = gui_test_directory("multipage_switch_destination");
    let source = failed_dir.join("source.png");
    let profile = failed_dir.join("profile.json");
    save_rgb(&source, &[40, 50, 60]);

    let mut state = GuiState::new(None);
    state.output_dir = failed_dir.display().to_string();
    state.output_name = "document".into();
    state.multipage = true;
    state.multipage_format = "pdf".into();
    state.pdf_password = "switch-secret".into();
    state.scanner_profile_path = profile.display().to_string();
    state.last_image = Some(source);
    state.do_save_plus();
    assert_eq!(state.pdf_password, "switch-secret");

    save_profile_json(
        &profile,
        &json!({
            "format": PROFILE_FORMAT,
            "version": PROFILE_VERSION,
            "kind": "scanner_it8",
            "matrix": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            "gamma": [1.0, 1.0, 1.0]
        }),
    )
    .expect("write corrected scanner profile");
    state.output_dir = destination_dir.display().to_string();
    state.do_save_plus();

    let output = destination_dir.join("document_multipage.pdf");
    let info = Command::new("pdfinfo")
        .args(["-upw", "switch-secret"])
        .arg(&output)
        .output()
        .expect("run pdfinfo");
    assert!(
        info.status.success(),
        "destination switch must not drop encryption: {}",
        String::from_utf8_lossy(&info.stderr)
    );
    assert!(state.pdf_password.is_empty());

    let _ = std::fs::remove_dir_all(failed_dir);
    let _ = std::fs::remove_dir_all(destination_dir);
}

fn gui_test_directory(label: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("open_scanline_gui_{label}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create GUI test directory");
    dir
}

fn save_rgb(path: &std::path::Path, data: &[u8; 3]) {
    crate::imaging::save_image(
        path,
        &crate::core::ImageBuffer::new(1, 1, crate::core::PixelFormat::Rgb8, data.to_vec())
            .expect("construct RGB source"),
        None,
        None,
    )
    .expect("write RGB source");
}

#[test]
fn gui_save_config_roundtrips_crop_and_levels() {
    let path = std::env::temp_dir().join(format!(
        "open_scanline_gui_config_{}.json",
        std::process::id()
    ));
    let mut state = GuiState::new(Some(&path));
    state.crop_text = "1,2,300,400".into();
    state.levels_black = 12;
    state.levels_white = 230;
    state.levels_gamma = 1.4;
    state.saturation = 25.0;
    state.hue = -90.0;
    state.curves_text = "0:0,128:160,255:255".into();
    state.media = "document".into();
    state.duplex = true;
    state.auto_orient = true;
    state.auto_crop = true;
    state.infrared_clean = "heavy".into();
    state.descreen = true;
    state.descreen_dpi = 450;
    state.output_name = "configured".into();
    state.output_fmt = "tif".into();
    state.multipage = true;
    state.ocr_engine = "tesseract".into();
    state.ocr_language = "fra".into();
    state.do_save_config();

    let reloaded = load_config(Some(&path)).expect("reload saved GUI config");
    assert_eq!(reloaded.crop, Some([1, 2, 300, 400]));
    assert_eq!(reloaded.levels_black, 12);
    assert_eq!(reloaded.levels_white, 230);
    assert!((reloaded.levels_gamma - 1.4).abs() < f64::EPSILON);
    assert_eq!(reloaded.saturation, 25.0);
    assert_eq!(reloaded.hue, -90.0);
    assert_eq!(reloaded.curves, Some(vec![[0, 0], [128, 160], [255, 255]]));
    assert_eq!(reloaded.scan_mode, ScanMode::Document);
    assert!(reloaded.duplex);
    assert!(reloaded.auto_orient);
    assert!(reloaded.auto_crop);
    assert_eq!(reloaded.infrared_clean.as_deref(), Some("heavy"));
    assert!(reloaded.descreen);
    assert_eq!(reloaded.descreen_dpi, 450);
    assert_eq!(reloaded.output_name, "configured");
    assert_eq!(reloaded.output_format, "tif");
    assert!(reloaded.multipage);
    assert_eq!(reloaded.ocr_engine, "tesseract");
    assert_eq!(reloaded.ocr_language, "fra");

    let reloaded_state = GuiState::new(Some(&path));
    assert_eq!(reloaded_state.saturation, 25.0);
    assert_eq!(reloaded_state.hue, -90.0);
    assert_eq!(reloaded_state.curves_text, "0:0,128:160,255:255");

    state.crop_text = "not,a,crop".into();
    state.do_save_config();
    assert!(state.status.contains("invalid crop"));
    assert_eq!(state.config.crop, Some([1, 2, 300, 400]));
    assert_eq!(
        load_config(Some(&path)).unwrap().crop,
        Some([1, 2, 300, 400])
    );
    state.crop_text.clear();
    state.curves_text = "0:0,255:300".into();
    state.do_save_config();
    assert!(state.status.contains("invalid curves"));
    assert_eq!(
        state.config.curves,
        Some(vec![[0, 0], [128, 160], [255, 255]])
    );
    assert_eq!(
        load_config(Some(&path)).unwrap().curves,
        state.config.curves
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn gui_invalid_config_blocks_save_until_explicit_recovery() {
    let dir = gui_test_directory("invalid_config_recovery");
    let path = dir.join("config.json");
    let invalid = b"{ invalid config bytes";
    std::fs::write(&path, invalid).expect("write malformed config");

    let mut state = GuiState::new(Some(&path));
    assert!(state.config_recovery_required);
    assert!(state.status.contains(&state.translator.t("error.config")));
    state.output_name = "replacement".into();
    state.do_save_config();
    assert_eq!(
        std::fs::read(&path).expect("read blocked config"),
        invalid,
        "blocked save must preserve malformed config bytes"
    );

    state.do_reset_config_recovery();
    assert!(!state.config_recovery_required);
    state.do_save_config();
    assert_eq!(
        load_config(Some(&path))
            .expect("load recovered config")
            .output_name,
        "replacement"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn gui_missing_config_starts_ready_with_defaults() {
    let dir = gui_test_directory("missing_config_ready");
    let path = dir.join("config.json");

    let state = GuiState::new(Some(&path));
    assert_eq!(state.config, AppConfig::default());
    assert!(!state.config_recovery_required);
    assert_eq!(state.status, state.translator.t("ready"));

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn invalid_curve_text_blocks_gui_scan_and_reprocess() {
    let directory = std::env::temp_dir().join(format!(
        "open_scanline_gui_invalid_curves_{}",
        std::process::id()
    ));
    let mut state = GuiState::new(None);
    state.output_dir = directory.display().to_string();
    state.curves_text = "128:128,0:0".into();

    state.do_scan(false);
    assert!(state
        .status
        .contains("curve x values must be strictly increasing"));
    assert!(!directory.join("scan_scan_000.png").exists());

    state.last_image = Some(directory.join("source.png"));
    state.do_reprocess();
    assert!(state
        .status
        .contains("curve x values must be strictly increasing"));
}

#[test]
fn reprocess_uses_full_filter_prefs() {
    use crate::scan::{run_scan_to_file, ScanToFileArgs};
    let dir = std::env::temp_dir().join("open_scanline_gui_reprocess");
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("base.png");
    run_scan_to_file(ScanToFileArgs {
        out: src.clone(),
        width: 24,
        height: 16,
        seed: 5,
        ..ScanToFileArgs::default()
    })
    .unwrap();
    let mut state = GuiState::new(None);
    state.output_dir = dir.display().to_string();
    state.output_name = "re".into();
    state.last_image = Some(src.clone());
    state.infrared_clean = "heavy".into();
    state.descreen = true;
    state.restore_colors = true;
    state.film_type = "generic_color_negative".into();
    let before = std::fs::read(&src).unwrap();
    state.do_reprocess();
    assert!(state.status.contains(&state.translator.t("status.done")));
    let out = state.last_image.expect("reprocess path");
    assert!(out.is_file());
    assert_ne!(
        std::fs::read(&out).unwrap(),
        before,
        "reprocess with filters must change image bytes"
    );
}

#[test]
fn auto_levels_scan_kwargs_reach_pipeline() {
    use crate::core::{ImageBuffer, PixelFormat};
    use crate::pipeline::apply_pipeline;
    let mut data = vec![100u8; 32 * 32 * 3];
    for index in 0..64 {
        data[index * 3..index * 3 + 3].copy_from_slice(&[20, 20, 20]);
    }
    for index in 0..64 {
        let offset = data.len() - (index + 1) * 3;
        data[offset..offset + 3].copy_from_slice(&[200, 200, 200]);
    }
    let image = ImageBuffer::new(32, 32, PixelFormat::Rgb8, data.clone()).unwrap();
    let mut state = GuiState::new(None);
    state.auto_levels = true;
    let prefs = state.pipeline_prefs();
    assert!(prefs.auto_levels);
    assert_ne!(apply_pipeline(&image, &prefs).unwrap().data, data);
}

#[test]
fn filter_kwargs_reach_apply_pipeline() {
    use crate::core::{ImageBuffer, PixelFormat};
    use crate::pipeline::apply_pipeline;
    let mut data = vec![80u8; 12 * 12 * 3];
    data[..3].copy_from_slice(&[255, 255, 255]);
    let image = ImageBuffer::new(12, 12, PixelFormat::Rgb8, data.clone()).unwrap();
    let mut state = GuiState::new(None);
    state.infrared_clean = "heavy".into();
    state.descreen = true;
    let out = apply_pipeline(&image, &state.pipeline_prefs()).unwrap();
    assert_eq!(out.width, 12);
    assert_ne!(out.data, data);
}

#[test]
fn parse_crop_and_ext() {
    assert_eq!(parse_crop_string("").unwrap(), None);
    assert_eq!(parse_crop_string("1,2,3,4").unwrap(), Some([1, 2, 3, 4]));
    assert_eq!(normalize_image_ext("JPEG", "png"), "jpg");
}

#[test]
fn multipage_save_destination_uses_selected_container_format() {
    let mut state = GuiState::new(None);
    state.output_dir = "/tmp/open-scanline-gui-multipage-destination".into();
    state.output_name = "document".into();
    state.multipage = true;

    state.multipage_format = "pdf".into();
    assert_eq!(
        state
            .multipage_save_destination()
            .unwrap()
            .file_name()
            .unwrap(),
        "document_multipage.pdf"
    );

    state.multipage_format = "tiff".into();
    assert_eq!(
        state
            .multipage_save_destination()
            .unwrap()
            .file_name()
            .unwrap(),
        "document_multipage.tif"
    );

    state.multipage_format = "png".into();
    assert!(state.multipage_save_destination().is_err());
}

#[test]
fn image_buffer_to_rgba_for_preview_surface() {
    use crate::core::{ImageBuffer, PixelFormat};
    use crate::imaging::image_buffer_to_rgba;
    let image = ImageBuffer::new(2, 1, PixelFormat::Rgb8, vec![255, 0, 0, 0, 255, 0]).unwrap();
    let (width, height, rgba) = image_buffer_to_rgba(&image).unwrap();
    assert_eq!((width, height), (2, 1));
    assert_eq!(rgba.len(), 8);
    assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
    assert_eq!(&rgba[4..], &[0, 255, 0, 255]);
}

#[test]
fn preview_cache_invalidates_on_same_path_rewrite() {
    use crate::imaging::image_buffer_to_rgba;
    use crate::scan::{run_scan_to_file, ScanToFileArgs};
    use std::path::Path;
    use std::thread;
    use std::time::Duration;
    let dir = std::env::temp_dir().join("open_scanline_preview_cache");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("preview.png");
    run_scan_to_file(ScanToFileArgs {
        out: path.clone(),
        width: 16,
        height: 12,
        ..ScanToFileArgs::default()
    })
    .unwrap();
    let first = preview_file_fingerprint(&path).expect("fp1");
    let image = crate::imaging::load_image(&path).unwrap();
    let (first_width, first_height, first_rgba) = image_buffer_to_rgba(&image).unwrap();
    assert_eq!((first_width, first_height), (16, 12));
    thread::sleep(Duration::from_millis(50));
    run_scan_to_file(ScanToFileArgs {
        out: path.clone(),
        width: 32,
        height: 24,
        seed: 2,
        ..ScanToFileArgs::default()
    })
    .unwrap();
    let second = preview_file_fingerprint(&path).expect("fp2");
    let image = crate::imaging::load_image(&path).unwrap();
    let (width, height, second_rgba) = image_buffer_to_rgba(&image).unwrap();
    assert_eq!((width, height), (32, 24));
    assert_ne!(first_rgba.len(), second_rgba.len());
    assert!(preview_texture_needs_reload(
        Some(&path),
        Some(&path),
        Some(first)
    ));
    assert!(!preview_texture_needs_reload(
        Some(&path),
        Some(&path),
        Some(second)
    ));
    assert!(preview_texture_needs_reload(
        Some(&path),
        Some(Path::new("other.png")),
        Some(second)
    ));
}
