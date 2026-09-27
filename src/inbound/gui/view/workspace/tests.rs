use super::*;

#[test]
fn preparation_fits_wide_and_narrow_windows() {
    for width in [560.0, 1280.0, 1536.0] {
        let context = egui::Context::default();
        crate::inbound::gui::view::apply_theme(&context);
        let mut app = OpenScanlineApp::new(Some(std::path::Path::new(
            "target/gui-layout-test-absent.json",
        )));
        app.state.config_load_error = None;
        app.state.output_dir = "Documents/Scans".into();
        app.state.output_name = "September-invoices".into();
        app.state.output_fmt = "pdf".into();
        app.state.searchable_pdf = true;
        app.state.batch_pages = 6;
        let mut batch = true;
        let mut bounds = egui::Rect::NOTHING;
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 1024.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.set_width((width - 48.0).min(1080.0));
                    prepare(ui, &mut app, &mut batch);
                    bounds = ui.min_rect();
                });
            },
        );
        assert!(
            bounds.right() <= width,
            "content exceeds {width}px viewport: {bounds:?}"
        );
        assert!(!app.job_active(), "rendering must never dispatch a scan");
    }
}

#[test]
fn invalid_duplex_does_not_enable_scan() {
    let mut app = OpenScanlineApp::new(Some(std::path::Path::new(
        "target/gui-layout-test-absent.json",
    )));
    app.state.duplex = true;
    app.state.media = "document".into();
    app.state.batch_pages = 3;
    assert!(!can_start(&app.state, true));
    app.state.batch_pages = 6;
    assert!(!can_start(&app.state, false));
    assert!(can_start(&app.state, true));
    app.state.media = "flatbed".into();
    assert!(!can_start(&app.state, true));
    assert!(!app.job_active());
}

#[test]
fn incompatible_output_options_disable_scan_until_explicitly_corrected() {
    let mut app = OpenScanlineApp::new(Some(std::path::Path::new(
        "target/gui-layout-test-absent.json",
    )));
    app.state.output_fmt = "png".into();
    app.state.searchable_pdf = true;
    assert!(!can_start(&app.state, false));
    app.state.searchable_pdf = false;
    app.state.pdf_password = "runtime-only-test".into();
    assert!(!can_start(&app.state, false));
    app.state.pdf_password.clear();
    assert!(can_start(&app.state, false));
    app.state.output_fmt = "jxl".into();
    assert!(!can_start(&app.state, true));
}

#[test]
fn disabled_start_ignores_pointer_activation() {
    let context = egui::Context::default();
    let mut app = OpenScanlineApp::new(Some(std::path::Path::new(
        "target/gui-layout-test-absent.json",
    )));
    app.state.duplex = true;
    app.state.media = "document".into();
    app.state.batch_pages = 3;
    let mut button = egui::Rect::NOTHING;
    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(560.0, 640.0),
        )),
        ..Default::default()
    };
    for click in [false, true] {
        if click {
            input.events = vec![
                egui::Event::PointerMoved(button.center()),
                egui::Event::PointerButton {
                    pos: button.center(),
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: button.center(),
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ];
        }
        let _ = context.run(input.clone(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let valid = can_start(&app.state, true);
                let response = start_action(ui, &mut app, true, valid);
                assert!(!response.enabled());
                button = response.rect;
            });
        });
        assert!(!app.job_active());
    }
}
