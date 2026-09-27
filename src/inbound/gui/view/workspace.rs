use super::super::{app::OpenScanlineApp, state::GuiState};
use egui::{Color32, RichText};

mod header;
mod settings;
mod status;

const TOOLS: &str = "light-workspace-tools";
const BATCH: &str = "light-workspace-batch";
const MUTED: Color32 = Color32::from_rgb(91, 101, 117);

pub(super) use header::render_header;

pub(super) fn render(ctx: &egui::Context, app: &mut OpenScanlineApp) {
    let mut batch = ctx
        .data_mut(|data| data.get_temp::<bool>(egui::Id::new(BATCH)))
        .unwrap_or(app.state.multipage || app.state.batch_pages > 1 || app.state.duplex);
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(ctx.style().visuals.panel_fill)
                .inner_margin(egui::Margin::symmetric(24, 0)),
        )
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    canvas(ui, app, &mut batch);
                });
        });
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(BATCH), batch));
}

fn prepare(ui: &mut egui::Ui, app: &mut OpenScanlineApp, batch: &mut bool) {
    prepare_title(ui, &app.state, *batch);
    ui.add_space(if ui.ctx().screen_rect().height() >= 900.0 {
        34.0
    } else {
        22.0
    });
    ui.add_enabled_ui(!app.job_active(), |ui| {
        columns(ui, |ui, left| {
            if left {
                settings::scan(ui, &mut app.state, batch);
            } else {
                settings::output(ui, &mut app.state, *batch);
            }
        });
    });
    ui.add_space(24.0);
    ui.separator();
    ui.add_space(10.0);
    let valid = can_start(&app.state, *batch);
    if ui.available_width() >= 850.0 {
        ui.horizontal(|ui| {
            destination(ui, app);
            start_action(ui, app, *batch, valid);
        });
    } else {
        ui.horizontal_wrapped(|ui| destination(ui, app));
        ui.add_space(10.0);
        ui.horizontal(|ui| start_action(ui, app, *batch, valid));
    }
    ui.add_space(8.0);
    if let Some(error) = &app.state.error_message {
        ui.colored_label(Color32::from_rgb(160, 62, 42), error);
    } else {
        ui.label(RichText::new(&app.state.status).small().color(MUTED));
    }
    if let Some(error) = &app.state.config_load_error {
        ui.colored_label(Color32::from_rgb(160, 62, 42), error);
        if ui.button("Configuration recovery").clicked() {
            app.state.active_tab = 5;
            ui.ctx()
                .data_mut(|data| data.insert_temp(egui::Id::new(TOOLS), true));
        }
    }
    if *batch {
        note(ui, "Page images are saved during scanning. The document is created after capture succeeds.");
    }
}

fn columns(ui: &mut egui::Ui, mut content: impl FnMut(&mut egui::Ui, bool)) {
    if ui.available_width() >= 760.0 {
        let width = (ui.available_width() - 96.0) / 2.0;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(width);
                content(ui, true);
            });
            ui.add_space(84.0);
            ui.vertical(|ui| {
                ui.set_width(width);
                content(ui, false);
            });
        });
    } else {
        content(ui, true);
        ui.add_space(30.0);
        content(ui, false);
    }
}

fn note(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).small().color(MUTED));
}
fn primary(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(Color32::WHITE))
        .fill(Color32::from_rgb(58, 103, 136))
        .min_size(egui::vec2(180.0, 48.0))
}
fn field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    let response = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(ui.available_width())
            .min_size(egui::vec2(0.0, 38.0))
            .margin(egui::Margin::symmetric(10, 8)),
    )
    .labelled_by(response.id);
}
fn choice(ui: &mut egui::Ui, id: &str, value: &mut String, choices: &[(&str, &str)]) {
    let label = match id {
        "workspace-source" => "Paper source",
        "workspace-format" => "Output format",
        "workspace-container" => "Document format",
        "workspace-ocr" => "Engine",
        _ => "Selection",
    };
    let label_response = ui.label(label);
    let selected = choices
        .iter()
        .find(|(key, _)| *key == value)
        .map_or(value.as_str(), |(_, label)| *label)
        .to_owned();
    egui::ComboBox::from_id_salt(id)
        .width(ui.available_width())
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for (key, label) in choices {
                ui.selectable_value(value, (*key).into(), *label);
            }
        })
        .response
        .labelled_by(label_response.id);
}

fn can_start(state: &GuiState, batch: bool) -> bool {
    let pdf =
        state.output_fmt == "pdf" || (batch && state.multipage && state.multipage_format == "pdf");
    !state.device.trim().is_empty()
        && !state.output_dir.trim().is_empty()
        && crate::domain::settings::validate_output_name(&state.output_name).is_ok()
        && (pdf || (!state.searchable_pdf && state.pdf_password.is_empty()))
        && !(batch && state.output_fmt == "jxl")
        && (!state.duplex
            || (batch
                && state.batch_pages.is_multiple_of(2)
                && state.scan_mode() == crate::domain::acquisition::ScanMode::Document))
}

fn destination(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    let label = ui.label("Save in");
    ui.add_enabled_ui(!app.job_active(), |ui| {
        let width = if ui.available_width() > 700.0 {
            300.0
        } else {
            190.0
        };
        ui.add(egui::TextEdit::singleline(&mut app.state.output_dir).desired_width(width))
            .labelled_by(label.id);
        if ui.link("Choose folder").clicked() {
            if let Some(folder) = rfd::FileDialog::new()
                .set_directory(&app.state.output_dir)
                .pick_folder()
            {
                app.state.output_dir = folder.display().to_string();
            }
        }
    });
}

fn start_action(
    ui: &mut egui::Ui,
    app: &mut OpenScanlineApp,
    batch: bool,
    valid: bool,
) -> egui::Response {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let response = ui.add_enabled(valid && !app.job_active(), primary("Start scan"));
        if response.clicked() {
            if batch {
                app.start_batch();
            } else {
                app.start_scan(false, false);
            }
        }
        if ui.available_width() > 140.0 {
            note(
                ui,
                if batch {
                    format!("Up to {} sides", app.state.batch_pages)
                } else {
                    "One image".into()
                },
            );
        }
        response
    })
    .inner
}

fn canvas(ui: &mut egui::Ui, app: &mut OpenScanlineApp, batch: &mut bool) {
    let width = ui.available_width().min(1080.0);
    let margin = ((ui.available_width() - width) / 2.0).max(0.0);
    ui.horizontal(|ui| {
        ui.add_space(margin);
        ui.vertical(|ui| {
            ui.set_width(width);
            ui.add_space(
                if width > 760.0 && ui.ctx().screen_rect().height() >= 900.0 {
                    62.0
                } else {
                    30.0
                },
            );
            if app.job_report().is_some() {
                status::render(ui, app);
            } else {
                prepare(ui, app, batch);
            }
            ui.add_space(32.0);
        });
    });
}

fn prepare_title(ui: &mut egui::Ui, state: &GuiState, batch: bool) {
    ui.label(
        RichText::new(if state.output_name.is_empty() {
            "New scan"
        } else {
            &state.output_name
        })
        .size(40.0),
    );
    ui.label(
        RichText::new(if batch {
            format!(
                "Create {} from up to {} image sides.",
                if state.output_fmt == "pdf" || (state.multipage && state.multipage_format == "pdf")
                {
                    "a PDF"
                } else if state.multipage {
                    "a TIFF document"
                } else {
                    "page images"
                },
                state.batch_pages
            )
        } else {
            format!(
                "Save one scanned image as {}.",
                state.output_fmt.to_uppercase()
            )
        })
        .size(20.0)
        .color(MUTED),
    );
}

#[cfg(test)]
mod tests;
