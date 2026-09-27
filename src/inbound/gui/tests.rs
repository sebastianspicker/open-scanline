use super::state::GuiState;
use super::*;
use crate::domain::acquisition::ScanMode;
use crate::infrastructure::config::json::load_config;
use crate::workflows::ports::acquisition::DeviceMaintenanceCapabilities;
use crate::workflows::publication::OcrEngine;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[test]
fn workspace_errors_are_structured_and_clear_after_success() {
    let mut state = GuiState::new(None);
    state.set_error("The destination is not writable");
    assert_eq!(
        state.error_message.as_deref(),
        Some("The destination is not writable")
    );
    assert!(state.status.contains("The destination is not writable"));

    state.set_image_success(std::path::PathBuf::from("completed.png"));
    assert!(state.error_message.is_none());
    assert_eq!(
        state.last_image,
        Some(std::path::PathBuf::from("completed.png"))
    );
}

#[test]
fn gui_ocrs_selection_flows_into_export_options() {
    let mut state = GuiState::new(None);
    state.ocr_engine = "ocrs".into();

    assert_eq!(state.selected_ocr_engine(), OcrEngine::Ocrs);
    assert_eq!(state.export_options().ocr_engine, OcrEngine::Ocrs);
}

#[test]
fn unsupported_maintenance_controls_do_not_dispatch_device_actions() {
    let mut state = GuiState::new(None);
    state.device = "missing:maintenance-fixture".into();
    state.maintenance_capabilities =
        DeviceMaintenanceCapabilities::unsupported("fixture unavailable");

    state.do_calibrate();
    assert_eq!(
        state.status,
        "calibrate missing:maintenance-fixture: unsupported (fixture unavailable)"
    );
    state.do_focus();
    assert_eq!(
        state.status,
        "focus missing:maintenance-fixture: unsupported (fixture unavailable)"
    );
}

#[test]
fn maintenance_cache_tracks_selection_and_inventory_refresh() {
    let mut state = GuiState::new(None);
    state.device = "file:/missing-maintenance-fixture".into();
    state.refresh_maintenance_capabilities();
    assert!(!state.calibration_available());
    assert!(!state.focus_available());

    state.device = "mock".into();
    state.refresh_maintenance_capabilities();
    assert!(!state.calibration_available());
    assert!(!state.focus_available());
    assert!(state.discovery_requested);
    let inventory = state.devices.clone();
    state.refresh_devices();
    assert_eq!(state.devices, inventory);
    assert!(state.discovery_loading);
    assert!(state.discovery_refresh);
}

#[test]
fn gui_config_save_persists_the_ocrs_engine_choice() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "open_scanline_gui_ocrs_config_{}_{nonce}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).expect("unique scratch directory");
    let path = directory.join("config.json");
    let mut state = GuiState::new(Some(&path));
    state.ocr_engine = "ocrs".into();

    state.do_save_config();

    assert_eq!(load_config(Some(&path)).unwrap().ocr_engine, "ocrs");
    let _ = std::fs::remove_dir_all(directory);
}

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
    let _ = std::fs::remove_dir_all(root);
}

fn assert_cancelled_scan(dir: &std::path::Path) {
    use crate::domain::image::{ImageBuffer, PixelFormat};
    use crate::domain::processing::PipelinePrefs;
    use crate::inbound::api::scan::{run_scan_to_file, ScanToFileArgs};

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
    assert!(err
        .unwrap_err()
        .to_string()
        .to_lowercase()
        .contains("cancel"));
    let _ = ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![0, 0, 0]);
}

#[test]
fn gui_open_save_plus_and_cancel_traverse_the_shared_handlers() {
    use crate::inbound::api::scan::{run_scan_to_file, ScanToFileArgs};
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
    assert_eq!(open_image_file(&src).unwrap(), src);
    let mut state = GuiState::new(None);
    state.output_dir = dir.display().to_string();
    state.output_name = "gui_act".into();
    state.do_open_file(Some(&src));
    state.do_save_plus();
    assert!(state.last_image.as_ref().is_some_and(|path| path.is_file()));
    state.scanning = false;
    state.do_cancel();
    assert!(state.cancel_requested.load(Ordering::SeqCst));
    assert_cancelled_scan(&dir);
    let _ = std::fs::remove_dir_all(dir);
}
