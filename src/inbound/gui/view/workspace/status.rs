use super::*;
use crate::inbound::gui::app::report::{
    BatchStopReason, JobPhase, JobReport, JobTerminal, OutputKind, PublishedFile, RequestedOutput,
};

pub(super) fn render(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    let report = app.job_report().expect("report selected by workspace");
    stage(ui, stage_label(report), report.terminal.is_none());
    title(ui, &report.title, "Untitled scan");
    lead(ui, headline(report));
    if let Some(error) = &report.error {
        ui.add_space(space::L);
        banner(ui, "This job stopped with an error", error);
    }
    ui.add_space(space::XXL);
    columns(ui, |ui, left| {
        if left {
            capture(ui, report);
        } else {
            manifest(ui, report);
        }
    });
}

fn stage_label(report: &JobReport) -> &'static str {
    match report.terminal {
        None => "Scanning",
        Some(JobTerminal::Completed(_)) => "Saved",
        Some(JobTerminal::Cancelled) => "Cancelled",
        Some(JobTerminal::Failed) => "Stopped",
    }
}

fn headline(report: &JobReport) -> String {
    let done = report.sides_complete;
    let limit = report.side_limit;
    match report.terminal {
        None => format!(
            "{done} of up to {limit} sides saved. {}",
            phase(report.phase)
        ),
        Some(JobTerminal::Completed(reason)) => completed(report, reason),
        Some(JobTerminal::Cancelled) => {
            format!("Cancelled after {done} of {limit} sides. Files marked saved are kept.")
        }
        Some(JobTerminal::Failed) => {
            format!("Stopped after {done} of {limit} sides. Files marked saved are kept.")
        }
    }
}

fn phase(phase: JobPhase) -> &'static str {
    match phase {
        JobPhase::Preparing => "Preparing the scanner…",
        JobPhase::Acquiring => "Scanning the next side…",
        JobPhase::Processing => "Processing the last side…",
        JobPhase::Publishing => "Writing files…",
        JobPhase::Finished => "Finishing…",
    }
}

fn completed(report: &JobReport, reason: Option<BatchStopReason>) -> String {
    let done = report.sides_complete;
    match reason {
        Some(BatchStopReason::LimitReached) if feeder_may_hold_more(report) => format!(
            "All {done} sides saved. Stopped at the side limit; check the feeder for sheets left over."
        ),
        Some(BatchStopReason::LimitReached) => {
            format!("All {done} sides saved. Stopped at the side limit.")
        }
        Some(BatchStopReason::FeederExhausted) => format!(
            "{done} sides saved. The feeder was empty before the {}-side limit.",
            report.side_limit
        ),
        None => "The image is saved.".into(),
    }
}

fn feeder_may_hold_more(report: &JobReport) -> bool {
    report.mode == crate::domain::acquisition::ScanMode::Document
        && report.device != "mock"
        && !report.device.starts_with("file:")
}

fn capture(ui: &mut egui::Ui, report: &JobReport) {
    section(ui, "1", "Sides");
    sheets::rail(ui, report);
    ui.add_space(space::XL);
    detail(ui, "Scanner", &report.device, true);
    detail(ui, "Source", &report.source, false);
    let sides = if report.duplex {
        "Both sides (duplex)"
    } else {
        "One side"
    };
    detail(ui, "Sides", sides, false);
    detail(
        ui,
        "Resolution",
        &format!("{} dpi requested", report.dpi),
        false,
    );
    let area = format!("{} × {} px requested", report.width, report.height);
    detail(ui, "Scan area", &area, false);
}

fn detail(ui: &mut egui::Ui, label: &str, value: &str, mono: bool) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 26.0),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            let label_width = 120.0_f32.min(ui.available_width() / 2.5);
            text_cell(ui, label_width, RichText::new(label).color(color::GRAPHITE));
            let value = RichText::new(value);
            let value = if mono {
                value.font(theme::mono(size::MONO))
            } else {
                value
            };
            text_cell(ui, ui.available_width(), value);
        },
    );
}

fn text_cell(ui: &mut egui::Ui, width: f32, text: RichText) -> egui::Response {
    // add_sized uses centered_and_justified layout, which centers short labels
    // and spreads wrapped path characters. A normal vertical child keeps the
    // cell width while allowing its text to grow naturally in height.
    ui.allocate_ui_with_layout(
        egui::vec2(width, 26.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(width);
            ui.add(egui::Label::new(text).wrap())
        },
    )
    .inner
}

fn manifest(ui: &mut egui::Ui, report: &JobReport) {
    section(ui, "2", "Files");
    for kind in [
        OutputKind::Document,
        OutputKind::Image,
        OutputKind::RawImage,
    ] {
        for file in files(report, kind) {
            saved_row(ui, file, role(kind));
        }
    }
    missing(ui, report);
    pages(ui, report);
    for file in files(report, OutputKind::ContactSheet) {
        saved_row(ui, file, "Contact sheet");
    }
    ui.add_space(space::M);
    note(
        ui,
        "Nothing was uploaded. Files marked saved are on this computer.",
    );
}

fn files(report: &JobReport, kind: OutputKind) -> impl Iterator<Item = &PublishedFile> {
    report
        .published_files
        .iter()
        .filter(move |file| file.kind == kind)
}

fn role(kind: OutputKind) -> &'static str {
    match kind {
        OutputKind::Document => "Document",
        OutputKind::RawImage => "Unprocessed copy",
        OutputKind::ContactSheet => "Contact sheet",
        OutputKind::Image | OutputKind::Page => "Image",
    }
}

/// Requested documents and images that do not exist (yet).
fn missing(ui: &mut egui::Ui, report: &JobReport) {
    let running = report.terminal.is_none();
    for requested in report.requested_outputs.iter().filter(|output| {
        output.kind != OutputKind::Page
            && !report
                .published_files
                .iter()
                .any(|file| file.path == output.path)
    }) {
        let status = match (running, requested.kind) {
            (true, OutputKind::Document) => "Written after the last side",
            (true, _) => "Waiting",
            (false, _) => "Not created",
        };
        ledger_row(ui, &file_name(&requested.path), status, false);
    }
}

fn unpublished_outputs(
    report: &JobReport,
    kind: OutputKind,
) -> impl Iterator<Item = &RequestedOutput> {
    report.requested_outputs.iter().filter(move |output| {
        output.kind == kind
            && !report
                .published_files
                .iter()
                .any(|file| file.path == output.path)
    })
}

fn pages(ui: &mut egui::Ui, report: &JobReport) {
    let saved: Vec<_> = files(report, OutputKind::Page).collect();
    if let Some(first) = saved.first() {
        saved_pages(ui, &saved, first);
    } else if report.side_limit > 1 {
        let message = if report.terminal.is_none() {
            "No page images saved yet."
        } else {
            "No page images were saved."
        };
        note(ui, message);
    }
    if report.terminal.is_none() && report.sides_complete < report.side_limit {
        note(
            ui,
            format!(
                "Up to {} more to come.",
                report.side_limit - report.sides_complete
            ),
        );
    } else if report.terminal.is_some() {
        missing_pages(ui, report);
    }
}

fn saved_pages(ui: &mut egui::Ui, saved: &[&PublishedFile], first: &PublishedFile) {
    ui.add_space(space::M);
    if let Some(folder) = first.path.parent() {
        ui.label(label_text(&format!("{} page images in", saved.len())));
        ui.label(
            RichText::new(display_path(folder))
                .font(theme::mono(size::MONO_SMALL))
                .color(color::GRAPHITE),
        )
        .on_hover_text(folder.display().to_string());
        ui.add_space(space::XS);
    }
    egui::ScrollArea::vertical()
        .id_salt("published-pages")
        .max_height(300.0)
        .min_scrolled_height(saved.len().min(8) as f32 * 32.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for file in saved {
                ledger_row(ui, &file_name(&file.path), "Saved", true);
            }
        });
}

fn missing_pages(ui: &mut egui::Ui, report: &JobReport) {
    let Some((name, status)) = missing_page_summary(report) else {
        return;
    };
    ui.add_space(space::M);
    ledger_row(ui, &name, &status, false);
}

fn missing_page_summary(report: &JobReport) -> Option<(String, String)> {
    let missing: Vec<_> = unpublished_outputs(report, OutputKind::Page).collect();
    let first = missing.first()?;
    let name = if missing.len() == 1 {
        file_name(&first.path)
    } else {
        let last = missing.last().expect("non-empty missing page list");
        format!("{} … {}", file_name(&first.path), file_name(&last.path))
    };
    let status = if missing.len() == 1 {
        "Not created".to_owned()
    } else {
        format!("{} not created", missing.len())
    };
    Some((name, status))
}

fn saved_row(ui: &mut egui::Ui, file: &PublishedFile, role: &str) {
    ledger_row(ui, &file_name(&file.path), &format!("{role}, saved"), true)
        .on_hover_text(file.path.display().to_string());
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// Mark column, monospaced name, status on the right.
fn ledger_row(ui: &mut egui::Ui, name: &str, status: &str, saved: bool) -> egui::Response {
    let response = ui
        .allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 30.0),
            egui::Layout::left_to_right(egui::Align::Min),
            |ui| {
                ui.spacing_mut().item_spacing.x = space::S;
                let (mark, _) =
                    ui.allocate_exact_size(egui::vec2(16.0, 20.0), egui::Sense::hover());
                if saved {
                    check(ui.painter(), mark.center(), 11.0, color::INK);
                } else {
                    ui.painter().hline(
                        mark.x_range().shrink(4.0),
                        mark.center().y,
                        egui::Stroke::new(theme::HAIRLINE, color::GRAPHITE),
                    );
                }
                let status_width = 190.0_f32.min(ui.available_width() * 0.45);
                text_cell(
                    ui,
                    ui.available_width() - status_width,
                    RichText::new(name).font(theme::mono(size::MONO)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    ui.label(
                        RichText::new(status)
                            .font(theme::body(size::SMALL))
                            .color(color::GRAPHITE),
                    );
                });
            },
        )
        .response;
    let y = response.rect.bottom() + 1.0;
    ui.painter().hline(
        response.rect.x_range(),
        y,
        egui::Stroke::new(theme::HAIRLINE, color::RULE),
    );
    ui.add_space(2.0);
    response
}

pub(super) fn actions(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    let Some(folder) = save_folder(app.job_report()) else {
        return;
    };
    let running = app.job_active();
    action_layout(ui, &folder, running, |ui| {
        if running {
            cancel(ui, app);
        } else if ui.add(primary("Start another scan")).clicked() {
            app.clear_job_report();
        }
    });
    if running {
        note(
            ui,
            "Cancelling keeps saved pages. The document may not be created.",
        );
    }
}

fn action_layout<R>(
    ui: &mut egui::Ui,
    folder: &std::path::Path,
    running: bool,
    action: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    if ui.available_width() >= 700.0 {
        ui.horizontal(|ui| {
            let destination_width = (ui.available_width() - 220.0).max(160.0);
            ui.allocate_ui_with_layout(
                egui::vec2(destination_width, size::ACTION),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| destination(ui, folder, running, true),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), action)
                .inner
        })
        .inner
    } else {
        destination(ui, folder, running, false);
        ui.add_space(space::S);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), action)
            .inner
    }
}

fn destination(ui: &mut egui::Ui, folder: &std::path::Path, running: bool, truncate: bool) {
    ui.horizontal_wrapped(|ui| {
        ui.label(label_text(if running { "Saving to" } else { "Saved to" }));
        let path =
            egui::Label::new(RichText::new(display_path(folder)).font(theme::mono(size::MONO)));
        let response = if truncate {
            ui.add(path.truncate())
        } else {
            ui.add(path.wrap())
        };
        response.on_hover_text(folder.display().to_string());
    });
}

fn cancel(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    let cancelling = app
        .state
        .cancel_requested
        .load(std::sync::atomic::Ordering::Relaxed);
    let label = if cancelling {
        "Cancelling…"
    } else {
        "Cancel scan"
    };
    if ui
        .add_enabled(
            !cancelling,
            secondary(label).min_size(egui::vec2(160.0, size::ACTION)),
        )
        .clicked()
    {
        app.cancel_job();
    }
}

fn save_folder(report: Option<&JobReport>) -> Option<std::path::PathBuf> {
    let outputs = &report?.requested_outputs;
    outputs
        .iter()
        .find(|output| output.kind == OutputKind::Document)
        .or_else(|| outputs.first())
        .and_then(|output| output.path.parent())
        .map(std::path::Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    fn cancelled_page_report() -> super::JobReport {
        let requested_outputs = (1..=6)
            .map(|index| super::RequestedOutput {
                kind: super::OutputKind::Page,
                path: std::path::PathBuf::from(format!("Scans/batch/page_{index:03}.png")),
            })
            .collect();
        let published_files = (1..=2)
            .map(|index| super::PublishedFile {
                kind: super::OutputKind::Page,
                path: std::path::PathBuf::from(format!("Scans/batch/page_{index:03}.png")),
            })
            .collect();
        super::JobReport {
            title: "Invoices".into(),
            device: "mock".into(),
            source: "Document feeder".into(),
            mode: crate::domain::acquisition::ScanMode::Document,
            width: 320,
            height: 240,
            dpi: 300,
            duplex: false,
            requested_outputs,
            published_files,
            sides_complete: 2,
            side_limit: 6,
            phase: super::JobPhase::Finished,
            terminal: Some(super::JobTerminal::Cancelled),
            error: None,
        }
    }

    #[test]
    fn cancelled_report_names_the_uncreated_page_range() {
        let report = cancelled_page_report();
        assert_eq!(
            super::missing_page_summary(&report),
            Some(("page_003.png … page_006.png".into(), "4 not created".into()))
        );
    }

    fn action_bounds(viewport_width: f32) -> (egui::Rect, egui::Rect) {
        let context = egui::Context::default();
        crate::inbound::gui::view::apply_theme(&context);
        let mut action = egui::Rect::NOTHING;
        let mut available = egui::Rect::NOTHING;
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(viewport_width, 280.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.set_width(viewport_width - 40.0);
                    available = egui::Rect::from_min_size(
                        ui.next_widget_position(),
                        egui::vec2(ui.available_width(), ui.available_height()),
                    );
                    action = super::action_layout(
                        ui,
                        std::path::Path::new(
                            "/Users/example/Documents/a-very-long-output-folder/Scans",
                        ),
                        true,
                        |ui| ui.add(super::primary("Cancel scan")).rect,
                    );
                });
            },
        );
        (action, available)
    }

    #[test]
    fn action_layout_keeps_the_action_inside_narrow_and_wide_viewports() {
        for viewport_width in [360.0, 760.0, 1280.0] {
            let (action, available) = action_bounds(viewport_width);
            assert!(action.left() >= available.left());
            assert!(action.right() <= available.right());
        }
    }

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
