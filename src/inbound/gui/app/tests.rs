use super::*;
use std::time::{Duration, Instant};

fn install_test_job<F>(app: &mut OpenScanlineApp, worker: F) -> Arc<AtomicBool>
where
    F: FnOnce(Arc<AtomicBool>, Sender<GuiJobEvent>) + Send + 'static,
{
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let (sender, receiver) = mpsc::channel();
    let handle = std::thread::spawn(move || worker(worker_cancel, sender));
    app.state.cancel_requested = Arc::clone(&cancel);
    app.state.scanning = true;
    app.job = Some(GuiJob {
        receiver,
        handle: Some(handle),
        cancel: Arc::clone(&cancel),
        pending_pdf_password: None,
    });
    cancel
}

fn drain_until_idle(app: &mut OpenScanlineApp, context: &egui::Context) {
    for _ in 0..500 {
        app.drain_job_events(context);
        if !app.job_active() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("test job did not finish");
}

fn test_directory(label: &str) -> std::path::PathBuf {
    let directory =
        std::env::temp_dir().join(format!("open_scanline_gui_{label}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

fn install_cancelled_test_job(app: &mut OpenScanlineApp) -> Arc<AtomicBool> {
    install_test_job(app, move |cancel, sender| {
        while !cancel.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        sender.send(GuiJobEvent::Cancelled).unwrap();
    })
}

#[test]
fn scan_plus_updates_state_only_after_the_worker_completes() {
    let directory = test_directory("async_scan_plus");
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.state.output_dir = directory.display().to_string();
    app.state.output_name = "async".into();

    app.start_scan(false, true);
    assert!(app.job_active());
    assert_eq!(app.state.frame_index, 0);
    for _ in 0..100 {
        app.drain_job_events(&context);
        if !app.job_active() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    assert!(!app.job_active());
    assert_eq!(app.state.frame_index, 1);
    assert!(app
        .state
        .last_image
        .as_ref()
        .is_some_and(|path| path.is_file()));
    assert!(app
        .state
        .status
        .contains(&app.state.translator.t("status.done")));
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn pdf_scan_keeps_a_raster_working_image_for_follow_up_actions() {
    let directory = test_directory("pdf_working_image");
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.state.output_dir = directory.display().to_string();
    app.state.output_name = "document".into();
    app.state.output_fmt = "pdf".into();

    app.start_scan(false, false);
    drain_until_idle(&mut app, &context);

    let published = directory.join("document_scan_000.pdf");
    let working = app.state.last_image.clone().expect("working image path");
    assert!(published.is_file());
    assert_eq!(
        std::fs::read(&published).unwrap().get(..4),
        Some(b"%PDF".as_slice())
    );
    assert_eq!(
        working.extension().and_then(|value| value.to_str()),
        Some("png")
    );
    assert!(crate::infrastructure::media::load_image(&working).is_ok());

    app.state.ocr_engine = "offline".into();
    app.start_ocr();
    drain_until_idle(&mut app, &context);
    assert!(app
        .state
        .status
        .contains(crate::infrastructure::media::ocr::OFFLINE_OCR_ENGINE));
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn multipage_pdf_session_retains_every_temporary_scan_source() {
    let directory = test_directory("pdf_multipage_sources");
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.state.output_dir = directory.display().to_string();
    app.state.output_name = "document".into();
    app.state.output_fmt = "pdf".into();
    app.state.multipage = true;
    app.state.multipage_format = "pdf".into();

    app.start_scan(false, false);
    drain_until_idle(&mut app, &context);
    let first = app.state.last_image.clone().expect("first raster source");
    app.start_save_plus();
    drain_until_idle(&mut app, &context);

    app.start_scan(false, false);
    drain_until_idle(&mut app, &context);
    let second = app.state.last_image.clone().expect("second raster source");
    assert_ne!(first, second);
    assert!(first.is_file(), "first raster must remain session-owned");
    assert!(second.is_file());

    app.start_save_plus();
    drain_until_idle(&mut app, &context);
    let document = lopdf::Document::load(directory.join("document_multipage.pdf")).unwrap();
    assert_eq!(document.get_pages().len(), 2);

    drop(app);
    assert!(!first.exists());
    assert!(!second.exists());
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn exported_container_save_retains_the_processable_source() {
    let directory = test_directory("container_source");
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    write_test_image(&source);
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.state.output_dir = directory.display().to_string();
    app.state.output_name = "saved".into();
    app.state.output_fmt = "pdf".into();
    app.state.last_image = Some(source.clone());

    app.start_save_plus();
    drain_until_idle(&mut app, &context);

    assert_eq!(app.state.last_image.as_ref(), Some(&source));
    assert!(directory.join("saved_save_000.pdf").is_file());
    let _ = std::fs::remove_dir_all(directory);
}

fn write_test_image(path: &Path) {
    crate::infrastructure::media::save_image(
        path,
        &crate::domain::image::ImageBuffer::new(
            1,
            1,
            crate::domain::image::PixelFormat::Rgb8,
            vec![20, 30, 40],
        )
        .unwrap(),
        None,
        None,
    )
    .unwrap();
}

#[test]
fn save_plus_starts_immediately_and_applies_the_worker_result() {
    let directory = test_directory("async_save");
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    write_test_image(&source);
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.state.output_dir = directory.display().to_string();
    app.state.output_name = "saved".into();
    app.state.last_image = Some(source.clone());

    app.start_save_plus();
    assert!(app.job_active());
    assert_eq!(app.state.last_image.as_ref(), Some(&source));
    assert_eq!(app.state.frame_index, 0);
    drain_until_idle(&mut app, &context);

    assert_eq!(app.state.frame_index, 1);
    assert!(app
        .state
        .last_image
        .as_ref()
        .is_some_and(|path| path.is_file()));
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn failed_file_export_restores_consumed_password() {
    let directory = test_directory("async_save_password");
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    write_test_image(&source);
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.state.output_dir = directory.display().to_string();
    app.state.output_name = "failed".into();
    app.state.output_fmt = "pdf".into();
    app.state.pdf_password = "save-secret".into();
    app.state.scanner_profile_path = directory.join("missing-profile.json").display().to_string();
    app.state.last_image = Some(source);

    app.start_save_plus();
    assert!(app.job_active());
    assert!(app.state.pdf_password.is_empty());
    drain_until_idle(&mut app, &context);

    assert_eq!(app.state.pdf_password, "save-secret");
    assert!(app.state.status.contains("Error"));
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn offline_ocr_starts_in_a_job_and_applies_its_result() {
    let directory = test_directory("async_ocr");
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    write_test_image(&source);
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.state.last_image = Some(source);
    app.state.ocr_engine = "offline".into();

    app.start_ocr();
    assert!(app.job_active());
    drain_until_idle(&mut app, &context);

    assert!(app
        .state
        .status
        .contains(crate::infrastructure::media::ocr::OFFLINE_OCR_ENGINE));
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn second_file_action_during_an_active_job_does_not_consume_its_password() {
    let directory = test_directory("async_action_guard");
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    write_test_image(&source);
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.state.output_dir = directory.display().to_string();
    app.state.output_name = "guard".into();
    app.state.output_fmt = "pdf".into();
    app.state.pdf_password = "unconsumed-secret".into();
    app.state.last_image = Some(source);
    install_cancelled_test_job(&mut app);

    app.start_save_plus();

    assert!(app.job_active());
    assert_eq!(app.state.pdf_password, "unconsumed-secret");
    app.cancel_job();
    drain_until_idle(&mut app, &context);
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn cancelled_job_resets_cancellation_for_the_next_reprocess() {
    let directory = test_directory("async_cancel_reset");
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("source.png");
    write_test_image(&source);
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.state.output_dir = directory.display().to_string();
    app.state.output_name = "reprocessed".into();
    app.state.last_image = Some(source);
    install_cancelled_test_job(&mut app);

    app.cancel_job();
    drain_until_idle(&mut app, &context);
    assert!(!app.state.cancel_requested.load(Ordering::SeqCst));

    app.start_reprocess();
    assert!(app.job_active());
    drain_until_idle(&mut app, &context);
    assert!(app
        .state
        .last_image
        .as_ref()
        .is_some_and(|path| path.is_file()));
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn terminal_drain_joins_before_clearing_the_job() {
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    let exited = Arc::new(AtomicBool::new(false));
    let worker_exited = Arc::clone(&exited);
    install_test_job(&mut app, move |_cancel, sender| {
        sender.send(GuiJobEvent::Cancelled).unwrap();
        std::thread::sleep(Duration::from_millis(30));
        worker_exited.store(true, Ordering::SeqCst);
    });

    let started = Instant::now();
    drain_until_idle(&mut app, &context);

    assert!(started.elapsed() >= Duration::from_millis(25));
    assert!(exited.load(Ordering::SeqCst));
    assert!(!app.state.scanning);
    assert!(!app.job_active());
}

#[test]
fn invalid_job_args_do_not_activate_or_replace_cancellation_state() {
    let mut app = OpenScanlineApp::new(None);
    app.state.output_name = "../outside".into();
    let original_cancel = Arc::clone(&app.state.cancel_requested);

    app.start_scan(false, false);
    assert!(!app.job_active());
    assert!(!app.state.scanning);
    assert!(Arc::ptr_eq(&app.state.cancel_requested, &original_cancel));

    app.start_batch();
    assert!(!app.job_active());
    assert!(!app.state.scanning);
    assert!(Arc::ptr_eq(&app.state.cancel_requested, &original_cancel));
}

#[test]
fn failed_encrypted_scan_restores_password() {
    let directory = test_directory("failed_encrypted_scan");
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.state.output_dir = directory.display().to_string();
    app.state.output_name = "encrypted_scan".into();
    app.state.output_fmt = "pdf".into();
    app.state.pdf_password = "scan-secret".into();
    app.state.scanner_profile_path = directory.join("missing-profile.json").display().to_string();

    app.start_scan(false, false);
    drain_until_idle(&mut app, &context);

    assert_eq!(app.state.pdf_password, "scan-secret");
    assert!(app.state.status.contains("Error"));
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn failed_encrypted_batch_restores_password() {
    let directory = test_directory("failed_encrypted_batch");
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.state.output_dir = directory.display().to_string();
    app.state.output_name = "encrypted_batch".into();
    app.state.output_fmt = "pdf".into();
    app.state.pdf_password = "batch-secret".into();
    app.state.scanner_profile_path = directory.join("missing-profile.json").display().to_string();

    app.start_batch();
    drain_until_idle(&mut app, &context);

    assert_eq!(app.state.pdf_password, "batch-secret");
    assert!(app.state.status.contains("Error"));
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn cancelled_encrypted_scan_restores_password_without_overwriting_replacement() {
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.state.pdf_password = "scan-secret".into();
    let export = app.state.take_export_options();
    let pending_password = export.pdf_password.clone();
    let cancel = install_cancelled_test_job(&mut app);
    app.job.as_mut().unwrap().pending_pdf_password = pending_password;
    app.state.pdf_password = "replacement-secret".into();

    app.cancel_job();
    assert!(cancel.load(Ordering::SeqCst));
    drain_until_idle(&mut app, &context);

    assert_eq!(app.state.pdf_password, "replacement-secret");
}

#[test]
fn cancelled_encrypted_batch_restores_password() {
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.state.pdf_password = "batch-secret".into();
    let export = app.state.take_export_options();
    let pending_password = export.pdf_password.clone();
    let cancel = install_cancelled_test_job(&mut app);
    app.job.as_mut().unwrap().pending_pdf_password = pending_password;

    app.cancel_job();
    assert!(cancel.load(Ordering::SeqCst));
    drain_until_idle(&mut app, &context);

    assert_eq!(app.state.pdf_password, "batch-secret");
}

#[test]
fn successful_encrypted_scan_consumes_password() {
    let directory = test_directory("successful_encrypted_scan");
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.state.output_dir = directory.display().to_string();
    app.state.output_name = "encrypted_scan".into();
    app.state.output_fmt = "pdf".into();
    app.state.pdf_password = "success-secret".into();

    app.start_scan(false, false);
    drain_until_idle(&mut app, &context);

    assert!(app.state.pdf_password.is_empty());
    assert!(directory.join("encrypted_scan_scan_000.pdf").is_file());
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn close_request_defers_close_until_cancelled_job_is_joined() {
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    let cancel = install_cancelled_test_job(&mut app);

    context.begin_pass(egui::RawInput::default());
    app.request_close(&context);
    let output = context.end_pass();
    assert!(output.viewport_output[&egui::ViewportId::ROOT]
        .commands
        .contains(&egui::ViewportCommand::CancelClose));
    assert!(app.closing);
    assert!(app.job_active());
    assert!(cancel.load(Ordering::SeqCst));

    drain_until_idle(&mut app, &context);
    context.begin_pass(egui::RawInput::default());
    app.finish_deferred_close(&context);
    let output = context.end_pass();
    assert!(output.viewport_output[&egui::ViewportId::ROOT]
        .commands
        .contains(&egui::ViewportCommand::Close));
}

#[test]
fn worker_panic_becomes_a_visible_failure() {
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    let cancel = Arc::new(AtomicBool::new(false));
    app.state.scanning = true;
    app.job = Some(OpenScanlineApp::spawn_job(cancel, None, |_| {
        panic!("test worker panic")
    }));

    drain_until_idle(&mut app, &context);

    assert!(app.state.status.contains("background scan worker panicked"));
    assert!(!app.state.scanning);
}

#[test]
fn drop_signals_and_joins_before_late_worker_output() {
    let marker = std::env::temp_dir().join(format!(
        "open_scanline_gui_drop_marker_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&marker);
    let worker_marker = marker.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    let mut app = OpenScanlineApp::new(None);
    install_test_job(&mut app, move |cancel, sender| {
        while !cancel.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        worker_cancelled.store(true, Ordering::SeqCst);
        sender.send(GuiJobEvent::Cancelled).unwrap();
        std::fs::write(worker_marker, "worker finished").unwrap();
    });

    drop(app);

    assert!(cancelled.load(Ordering::SeqCst));
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "worker finished");
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "worker finished");
    let _ = std::fs::remove_file(marker);
}

fn batch_report(directory: &Path, pages: u32) -> report::JobReport {
    let args = crate::workflows::capture::batch::BatchScanArgs {
        out_dir: directory.to_path_buf(),
        pages,
        ..crate::workflows::capture::batch::BatchScanArgs::default()
    };
    report::JobReport::batch("report-test".into(), &args)
}

#[test]
fn single_scan_report_distinguishes_pdf_documents_from_tiff_images() {
    for (extension, expected) in [
        ("pdf", report::OutputKind::Document),
        ("tif", report::OutputKind::Image),
        ("png", report::OutputKind::Image),
    ] {
        let path = PathBuf::from(format!("scan.{extension}"));
        let args = crate::workflows::capture::single::ScanToFileArgs {
            out: path.clone(),
            ..Default::default()
        };
        let report = report::JobReport::scan("Scan".into(), &args, path);
        assert_eq!(report.requested_outputs.last().unwrap().kind, expected);
    }
}

fn published_page(path: PathBuf) -> GuiJobEvent {
    GuiJobEvent::BatchWorkflow(
        crate::workflows::capture::batch::BatchWorkflowEvent::Published(
            crate::workflows::capture::batch::BatchPublishedOutput {
                kind: crate::workflows::capture::batch::BatchPublishedOutputKind::Page,
                path,
            },
        ),
    )
}

#[test]
fn prepublication_progress_never_confirms_an_existing_requested_file() {
    let directory = test_directory("report_progress_truth");
    std::fs::create_dir_all(&directory).unwrap();
    let old_page = directory.join("page_001.png");
    std::fs::write(&old_page, b"old output").unwrap();
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.job_report = Some(batch_report(&directory, 1));
    install_test_job(&mut app, move |_cancel, sender| {
        sender
            .send(GuiJobEvent::Progress(ScanProgress::new(
                "batch", 0.0, "page 1/1",
            )))
            .unwrap();
        sender
            .send(GuiJobEvent::Failed("publish failed".into()))
            .unwrap();
    });

    drain_until_idle(&mut app, &context);

    let report = app.job_report().unwrap();
    assert!(report.published_files.is_empty());
    assert_eq!(report.sides_complete, 0);
    assert_eq!(report.terminal, Some(report::JobTerminal::Failed));
    assert_eq!(report.error.as_deref(), Some("publish failed"));
    assert!(
        old_page.exists(),
        "fixture proves existence is not evidence"
    );
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn cancellation_retains_only_confirmed_batch_publications() {
    let directory = test_directory("report_cancelled_partial");
    let page = directory.join("page_001.png");
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.job_report = Some(batch_report(&directory, 2));
    let sent_page = page.clone();
    install_test_job(&mut app, move |_cancel, sender| {
        sender.send(published_page(sent_page)).unwrap();
        sender.send(GuiJobEvent::Cancelled).unwrap();
    });

    drain_until_idle(&mut app, &context);

    let report = app.job_report().unwrap();
    assert_eq!(report.sides_complete, 1);
    assert_eq!(report.published_files.len(), 1);
    assert_eq!(report.published_files[0].path, page);
    assert_eq!(report.terminal, Some(report::JobTerminal::Cancelled));
    assert!(report.error.is_none());
}

#[test]
fn failure_retains_confirmed_batch_publications_and_error() {
    let directory = test_directory("report_failed_partial");
    let page = directory.join("page_001.png");
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.job_report = Some(batch_report(&directory, 2));
    let sent_page = page.clone();
    install_test_job(&mut app, move |_cancel, sender| {
        sender.send(published_page(sent_page)).unwrap();
        sender
            .send(GuiJobEvent::Failed("document publication failed".into()))
            .unwrap();
    });

    drain_until_idle(&mut app, &context);

    let report = app.job_report().unwrap();
    assert_eq!(report.sides_complete, 1);
    assert_eq!(report.published_files[0].path, page);
    assert_eq!(report.terminal, Some(report::JobTerminal::Failed));
    assert_eq!(report.error.as_deref(), Some("document publication failed"));
}

#[test]
fn successful_mock_batch_reports_saved_count_and_limit_reason() {
    let directory = test_directory("report_batch_success");
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.state.output_dir = directory.display().to_string();
    app.state.output_name = "completed".into();
    app.state.output_fmt = "pdf".into();
    app.state.multipage = false;
    app.state.batch_pages = 2;

    app.start_batch();
    drain_until_idle(&mut app, &context);

    let report = app.job_report().unwrap();
    assert_eq!(report.sides_complete, 2);
    assert_eq!(report.side_limit, 2);
    assert_eq!(
        report.terminal,
        Some(report::JobTerminal::Completed(Some(
            report::BatchStopReason::LimitReached
        )))
    );
    assert_eq!(
        report
            .published_files
            .iter()
            .filter(|file| file.kind == report::OutputKind::Page)
            .count(),
        2
    );
    assert_eq!(
        report
            .published_files
            .iter()
            .filter(|file| file.kind == report::OutputKind::Document)
            .count(),
        1
    );
    assert!(report
        .published_files
        .iter()
        .all(|file| file.path.is_file()));
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn saved_raw_and_final_scan_outputs_are_both_confirmed() {
    let directory = test_directory("report_scan_raw");
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.state.output_dir = directory.display().to_string();
    app.state.output_name = "raw-and-final".into();
    app.state.save_raw = true;

    app.start_scan(false, false);
    drain_until_idle(&mut app, &context);

    let report = app.job_report().unwrap();
    assert_eq!(report.sides_complete, 1);
    assert_eq!(report.terminal, Some(report::JobTerminal::Completed(None)));
    assert_eq!(report.published_files.len(), 2);
    assert!(report
        .published_files
        .iter()
        .all(|file| file.path.is_file()));
    assert!(directory.join("raw-and-final_raw_000.tif").is_file());
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn scan_plus_keeps_each_indexed_raw_output() {
    let directory = test_directory("indexed_scan_raw");
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.state.output_dir = directory.display().to_string();
    app.state.output_name = "archive".into();
    app.state.save_raw = true;

    app.start_scan(false, true);
    drain_until_idle(&mut app, &context);
    let first_path = directory.join("archive_raw_000.tif");
    let first_contents = std::fs::read(&first_path).unwrap();

    app.state.width = 96;
    app.state.height = 64;
    app.start_scan(false, true);
    drain_until_idle(&mut app, &context);

    assert_eq!(app.state.frame_index, 2);
    assert_eq!(std::fs::read(&first_path).unwrap(), first_contents);
    assert!(directory.join("archive_raw_001.tif").is_file());
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn preview_clears_completed_export_report_and_returns_to_editor_flow() {
    let directory = test_directory("report_preview");
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(None);
    app.state.output_dir = directory.display().to_string();
    app.job_report = Some(batch_report(&directory, 1));

    app.start_scan(true, false);
    assert!(app.job_report().is_none());
    drain_until_idle(&mut app, &context);

    assert!(app.job_report().is_none());
    assert!(app
        .state
        .last_image
        .as_ref()
        .is_some_and(|path| path.is_file()));
    let _ = std::fs::remove_dir_all(directory);
}
