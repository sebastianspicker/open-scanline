use super::*;
use crate::inbound::gui::app::report::{
    BatchStopReason, JobPhase, JobReport, JobTerminal, OutputKind,
};

pub(super) fn render(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    let report = app.job_report().expect("report selected by workspace");
    ui.label(RichText::new(&report.title).size(40.0));
    let headline = match &report.terminal {
        Some(JobTerminal::Completed(_)) => "Scan complete",
        Some(JobTerminal::Cancelled) => "Scan cancelled",
        Some(JobTerminal::Failed) => "Scan stopped",
        None => "Scanning",
    };
    ui.label(
        RichText::new(format!(
            "{} of {} image sides saved.",
            report.sides_complete, report.side_limit
        ))
        .size(20.0)
        .color(MUTED),
    );
    ui.add_space(if ui.ctx().screen_rect().height() >= 900.0 {
        34.0
    } else {
        22.0
    });
    columns(ui, |ui, left| {
        if left {
            scan_summary(ui, report, headline);
        } else {
            manifest(ui, report);
        }
    });
    let destination = report
        .requested_outputs
        .first()
        .and_then(|output| output.path.parent())
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    ui.add_space(32.0);
    ui.separator();
    ui.add_space(14.0);
    if ui.available_width() >= 850.0 {
        ui.horizontal(|ui| {
            ui.label("Save in");
            text_cell(ui, 340.0, RichText::new(&destination));
            result_actions(ui, app);
        });
    } else {
        ui.label(format!("Save in  {destination}"));
        ui.horizontal(|ui| result_actions(ui, app));
    }
    if app.job_active() {
        note(
            ui,
            "Cancellation keeps completed outputs. A final document may not be created.",
        );
    }
}

fn result_actions(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if app.job_active() {
            let cancelling = app
                .state
                .cancel_requested
                .load(std::sync::atomic::Ordering::Relaxed);
            if ui
                .add_enabled(
                    !cancelling,
                    egui::Button::new(if cancelling {
                        "Cancelling…"
                    } else {
                        "Cancel scan"
                    })
                    .min_size(egui::vec2(180.0, 48.0)),
                )
                .clicked()
            {
                app.cancel_job();
            }
        } else {
            if ui.add(primary("Done")).clicked() {
                app.clear_job_report();
            }
            if ui.button("New scan").clicked() {
                app.clear_job_report();
            }
        }
    });
}

fn text_cell(ui: &mut egui::Ui, width: f32, text: RichText) -> egui::Response {
    // add_sized uses centered_and_justified layout, which centers short labels
    // and spreads wrapped path characters. A normal vertical child keeps the
    // cell width while allowing its text to grow naturally in height.
    ui.allocate_ui_with_layout(
        egui::vec2(width, 28.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(width);
            ui.add(egui::Label::new(text).wrap())
        },
    )
    .inner
}

fn file_row(ui: &mut egui::Ui, path: &std::path::Path, status: &str) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 28.0),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            text_cell(
                ui,
                (ui.available_width() - 90.0).max(40.0),
                RichText::new(path.file_name().unwrap_or_default().to_string_lossy()),
            );
            note(ui, status);
        },
    );
}

fn summary(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 28.0),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            text_cell(
                ui,
                155.0_f32.min(ui.available_width() / 2.0),
                RichText::new(label).color(MUTED),
            );
            text_cell(ui, ui.available_width(), RichText::new(value));
        },
    );
}

fn scan_summary(ui: &mut egui::Ui, report: &JobReport, headline: &str) {
    ui.heading(headline);
    ui.add_space(10.0);
    capture_progress(ui, report);
    ui.add_space(2.0);
    summary(ui, "Scanner", &report.device);
    summary(ui, "Source", &report.source);
    summary(
        ui,
        "Sides",
        if report.duplex {
            "Both sides, duplex"
        } else {
            "One side"
        },
    );
    summary(ui, "Requested resolution", &format!("{} dpi", report.dpi));
    summary(ui, "Side limit", &report.side_limit.to_string());
    note(
        ui,
        format!(
            "Requested image area: {} × {} px",
            report.width, report.height
        ),
    );
    ui.add_space(10.0);
    if let Some(error) = &report.error {
        ui.colored_label(Color32::from_rgb(160, 62, 42), error);
    }
    terminal_note(ui, report);
}

fn manifest(ui: &mut egui::Ui, report: &JobReport) {
    ui.heading(if report.terminal.is_some() {
        "Saved files"
    } else {
        "Output"
    });
    ui.add_space(10.0);
    if report.published_files.is_empty() {
        note(ui, "No files saved yet.");
    }
    for kind in [
        OutputKind::Document,
        OutputKind::Image,
        OutputKind::RawImage,
        OutputKind::ContactSheet,
    ] {
        for file in report
            .published_files
            .iter()
            .filter(|file| file.kind == kind)
        {
            file_row(ui, &file.path, "Saved");
            ui.add_space(2.0);
        }
    }
    published_pages(ui, report);
    for requested in report
        .requested_outputs
        .iter()
        .filter(|output| output.kind != OutputKind::Page)
    {
        if !report
            .published_files
            .iter()
            .any(|file| file.path == requested.path)
        {
            ui.label(
                requested
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .as_ref(),
            );
            note(ui, "Not saved by this scan");
        }
    }
    ui.add_space(20.0);
    ui.separator();
    note(ui, "Files are stored on this computer.");
}

fn capture_progress(ui: &mut egui::Ui, report: &JobReport) {
    if report.terminal.is_none() {
        let progress_label = ui.label("Capture progress");
        ui.add(
            egui::ProgressBar::new(report.sides_complete as f32 / report.side_limit.max(1) as f32)
                .desired_height(12.0),
        )
        .labelled_by(progress_label.id);
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(match report.phase {
                JobPhase::Preparing => "Preparing scan…",
                JobPhase::Acquiring => "Acquiring image sides…",
                JobPhase::Processing => "Processing the current image…",
                JobPhase::Publishing => "Writing output…",
                JobPhase::Finished => "Finishing…",
            });
        });
        ui.add_space(24.0);
        ui.separator();
    }
}

fn terminal_note(ui: &mut egui::Ui, report: &JobReport) {
    let Some(terminal) = report.terminal else {
        return;
    };
    let message = match terminal {
        JobTerminal::Completed(Some(BatchStopReason::LimitReached)) => limit_message(report),
        JobTerminal::Completed(Some(BatchStopReason::FeederExhausted)) => "The feeder is empty.",
        JobTerminal::Completed(None) => "The requested scan has finished.",
        JobTerminal::Cancelled => {
            "Cancellation has finished. Confirmed saved files are listed here."
        }
        JobTerminal::Failed => "Check the error and saved files before starting another scan.",
    };
    note(ui, message);
}

fn limit_message(report: &JobReport) -> &'static str {
    if report.mode == crate::domain::acquisition::ScanMode::Document
        && report.device != "mock"
        && !report.device.starts_with("file:")
    {
        "Stopped at the side limit. Check the feeder for remaining sheets."
    } else {
        "Stopped at the side limit."
    }
}

fn published_pages(ui: &mut egui::Ui, report: &JobReport) {
    let pages: Vec<_> = report
        .published_files
        .iter()
        .filter(|file| file.kind == OutputKind::Page)
        .collect();
    if let Some(first) = pages.first() {
        ui.separator();
        note(ui, format!("{} page images", pages.len()));
        if let Some(folder) = first.path.parent() {
            ui.label(
                RichText::new(folder.file_name().unwrap_or_default().to_string_lossy())
                    .small()
                    .color(MUTED),
            )
            .on_hover_text(folder.display().to_string());
        }
        egui::ScrollArea::vertical()
            .id_salt("published-pages")
            .max_height(280.0)
            .min_scrolled_height(pages.len().min(6) as f32 * 32.0)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                for file in pages {
                    file_row(ui, &file.path, "Saved");
                }
            });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn status_text_is_left_aligned_and_grows_when_wrapped() {
        let context = egui::Context::default();
        crate::inbound::gui::view::apply_theme(&context);
        let _ = context.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let left = ui.next_widget_position().x;
                let short = super::text_cell(ui, 155.0, egui::RichText::new("Scanner"));
                assert!((short.rect.left() - left).abs() < 0.1);
                let long = super::text_cell(ui, 155.0, egui::RichText::new("/Users/example/Documents/a-long-output-folder/a-long-scanned-document-filename.pdf"));
                assert!((long.rect.left() - left).abs() < 0.1);
                assert!(long.rect.height() > 56.0, "wrapped text must grow beyond one row");
                assert!(long.rect.width() <= 155.0);
            });
        });
    }
}
