use super::*;

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
