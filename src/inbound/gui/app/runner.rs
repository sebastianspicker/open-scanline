use std::path::Path;

#[cfg(feature = "gui")]
use super::OpenScanlineApp;

/// Launch desktop GUI. Uses eframe when the `gui` feature is enabled.
pub(in crate::inbound::gui) fn run(config_path: Option<&Path>) -> i32 {
    #[cfg(feature = "gui")]
    {
        let path_owned = config_path.map(Path::to_path_buf);
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1280.0, 860.0])
                .with_min_inner_size([560.0, 640.0])
                .with_title(format!("{} {}", crate::APP_NAME, crate::VERSION)),
            ..Default::default()
        };
        match eframe::run_native(
            crate::APP_NAME,
            options,
            Box::new(move |cc| {
                super::super::view::apply_theme(&cc.egui_ctx);
                Ok(Box::new(OpenScanlineApp::new(path_owned.as_deref())) as Box<dyn eframe::App>)
            }),
        ) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("GUI launch failed: {error}");
                eprintln!("GUI toolkit could not open a window in this environment");
                1
            }
        }
    }
    #[cfg(not(feature = "gui"))]
    {
        let _ = config_path;
        eprintln!("GUI unavailable: rebuild with the 'gui' feature and use a desktop session");
        1
    }
}
