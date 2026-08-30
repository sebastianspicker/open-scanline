use super::state::GuiState;
use super::*;
use crate::domain::acquisition::ScanMode;
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
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn gui_open_save_plus_and_cancel_traverse_the_shared_handlers() {
    use crate::domain::image::{ImageBuffer, PixelFormat};
    use crate::domain::processing::PipelinePrefs;
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
    let _ = std::fs::remove_dir_all(dir);
}
