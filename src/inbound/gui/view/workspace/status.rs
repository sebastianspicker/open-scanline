use super::*;
use crate::inbound::gui::app::report::{
    BatchStopReason, JobPhase, JobReport, JobTerminal, OutputKind, PublishedFile, RequestedOutput,
};

pub(super) fn render(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    let report = app.job_report().expect("report selected by workspace");
    stage(ui, &tr(stage_label(report)), report.terminal.is_none());
    title(ui, &report.title, &tr("ws.untitled_scan"));
    lead(ui, headline(report));
    if let Some(error) = &report.error {
        ui.add_space(space::L);
        banner(ui, &tr("ws.job_stopped"), error);
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
        None => "ws.stage_scanning",
        Some(JobTerminal::Completed(_)) => "ws.saved",
        Some(JobTerminal::Cancelled) => "ws.stage_cancelled",
        Some(JobTerminal::Failed) => "ws.stage_stopped",
    }
}

fn headline(report: &JobReport) -> String {
    let done = report.sides_complete;
    let limit = report.side_limit;
    match report.terminal {
        None => tr_args(
            "ws.headline_running",
            &[
                ("done", &done.to_string()),
                ("limit", &limit.to_string()),
                ("phase", &tr(phase(report.phase))),
            ],
        ),
        Some(JobTerminal::Completed(reason)) => completed(report, reason),
        Some(JobTerminal::Cancelled) => tr_args(
            "ws.headline_cancelled",
            &[("done", &done.to_string()), ("limit", &limit.to_string())],
        ),
        Some(JobTerminal::Failed) => tr_args(
            "ws.headline_stopped",
            &[("done", &done.to_string()), ("limit", &limit.to_string())],
        ),
    }
}

fn phase(phase: JobPhase) -> &'static str {
    match phase {
        JobPhase::Preparing => "ws.phase_preparing",
        JobPhase::Acquiring => "ws.phase_acquiring",
        JobPhase::Processing => "ws.phase_processing",
        JobPhase::Publishing => "ws.phase_publishing",
        JobPhase::Finished => "ws.phase_finishing",
    }
}

fn completed(report: &JobReport, reason: Option<BatchStopReason>) -> String {
    let done = report.sides_complete;
    match reason {
        Some(BatchStopReason::LimitReached) if feeder_may_hold_more(report) => {
            tr_args("ws.done_limit_feeder", &[("done", &done.to_string())])
        }
        Some(BatchStopReason::LimitReached) => {
            tr_args("ws.done_limit", &[("done", &done.to_string())])
        }
        Some(BatchStopReason::FeederExhausted) => tr_args(
            "ws.done_feeder_empty",
            &[
                ("done", &done.to_string()),
                ("limit", &report.side_limit.to_string()),
            ],
        ),
        None => tr("ws.image_saved"),
    }
}

fn feeder_may_hold_more(report: &JobReport) -> bool {
    report.mode == crate::domain::acquisition::ScanMode::Document
        && report.device != "mock"
        && !report.device.starts_with("file:")
}

fn capture(ui: &mut egui::Ui, report: &JobReport) {
    section(ui, "1", "ws.sec_sides");
    sheets::rail(ui, report);
    ui.add_space(space::XL);
    detail(ui, "ws.scanner", &report.device, true);
    detail(ui, "ws.source", &report.source, false);
    let sides = tr(if report.duplex {
        "ws.both_sides_duplex"
    } else {
        "ws.one_side"
    });
    detail(ui, "ws.sides", &sides, false);
    detail(
        ui,
        "ws.resolution",
        &tr_args("ws.dpi_requested", &[("dpi", &report.dpi.to_string())]),
        false,
    );
    let area = match (report.width, report.height) {
        (0, 0) => tr("ws.device_maximum"),
        (width, height) => {
            let axis = |pixels: u32| {
                if pixels == 0 {
                    tr("ws.max_axis")
                } else {
                    pixels.to_string()
                }
            };
            tr_args(
                "ws.px_requested",
                &[("width", &axis(width)), ("height", &axis(height))],
            )
        }
    };
    detail(ui, "ws.scan_area", &area, false);
}

fn detail(ui: &mut egui::Ui, label: &str, value: &str, mono: bool) {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 26.0),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            let label_width = 120.0_f32.min(ui.available_width() / 2.5);
            text_cell(
                ui,
                label_width,
                RichText::new(tr(label)).color(color::GRAPHITE),
            );
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
    section(ui, "2", "ws.sec_files");
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
        saved_row(ui, file, "ws.contact_sheet");
    }
    ui.add_space(space::M);
    note(ui, tr("ws.nothing_uploaded"));
}

fn files(report: &JobReport, kind: OutputKind) -> impl Iterator<Item = &PublishedFile> {
    report
        .published_files
        .iter()
        .filter(move |file| file.kind == kind)
}

fn role(kind: OutputKind) -> &'static str {
    match kind {
        OutputKind::Document => "ws.role_document",
        OutputKind::RawImage => "ws.role_raw_copy",
        OutputKind::ContactSheet => "ws.contact_sheet",
        OutputKind::Image | OutputKind::Page => "ws.role_image",
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
            (true, OutputKind::Document) => "ws.written_after_last",
            (true, _) => "ws.waiting",
            (false, _) => "ws.not_created",
        };
        ledger_row(ui, &file_name(&requested.path), &tr(status), false);
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
            "ws.no_pages_yet"
        } else {
            "ws.no_pages_saved"
        };
        note(ui, tr(message));
    }
    if report.terminal.is_none() && report.sides_complete < report.side_limit {
        note(
            ui,
            tr_args(
                "ws.more_to_come",
                &[(
                    "count",
                    &(report.side_limit - report.sides_complete).to_string(),
                )],
            ),
        );
    } else if report.terminal.is_some() {
        missing_pages(ui, report);
    }
}

fn saved_pages(ui: &mut egui::Ui, saved: &[&PublishedFile], first: &PublishedFile) {
    ui.add_space(space::M);
    if let Some(folder) = first.path.parent() {
        ui.label(label_text(&tr_args(
            "ws.page_images_in",
            &[("count", &saved.len().to_string())],
        )));
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
                ledger_row(ui, &file_name(&file.path), &tr("ws.saved"), true);
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
        tr("ws.not_created")
    } else {
        tr_args("ws.n_not_created", &[("count", &missing.len().to_string())])
    };
    Some((name, status))
}

fn saved_row(ui: &mut egui::Ui, file: &PublishedFile, role: &str) {
    ledger_row(
        ui,
        &file_name(&file.path),
        &tr_args("ws.role_saved", &[("role", &tr(role))]),
        true,
    )
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
        } else if ui.add(primary(&tr("ws.start_another"))).clicked() {
            app.clear_job_report();
        }
    });
    if running {
        note(ui, tr("ws.cancel_note"));
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
        ui.label(label_text(&tr(if running {
            "ws.saving_to"
        } else {
            "ws.saved_to"
        })));
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
    let label = tr(if cancelling {
        "ws.cancelling"
    } else {
        "ws.cancel_scan"
    });
    if ui
        .add_enabled(
            !cancelling,
            secondary(&label).min_size(egui::vec2(160.0, size::ACTION)),
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
