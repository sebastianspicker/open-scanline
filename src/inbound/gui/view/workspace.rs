use super::super::{app::OpenScanlineApp, state::GuiState};
use super::theme::{self, color, size, space};
use egui::RichText;

mod components;
mod header;
mod plan;
mod settings;
mod sheets;
mod status;

use components::*;

const TOOLS: &str = "light-workspace-tools";
const BATCH: &str = "light-workspace-batch";

pub(super) use header::render_header;

pub(super) fn render(ctx: &egui::Context, app: &mut OpenScanlineApp) {
    let mut batch = ctx
        .data_mut(|data| data.get_temp::<bool>(egui::Id::new(BATCH)))
        .unwrap_or(app.state.multipage || app.state.batch_pages > 1 || app.state.duplex);
    action_bar(ctx, |ui| {
        if app.job_report().is_some() {
            status::actions(ui, app);
        } else {
            prepare_actions(ui, app, batch);
        }
    });
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(color::PAPER)
                .inner_margin(egui::Margin::symmetric(space::XL as i8, 0)),
        )
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| sheet(ui, |ui| body(ui, app, &mut batch)));
        });
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(BATCH), batch));
}

fn body(ui: &mut egui::Ui, app: &mut OpenScanlineApp, batch: &mut bool) {
    ui.add_space(top_space(ui));
    if app.job_report().is_some() {
        status::render(ui, app);
    } else {
        prepare(ui, app, batch);
    }
    ui.add_space(space::XXXL);
}

/// Center content in a column no wider than the job sheet.
fn sheet(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    let width = ui.available_width().min(space::SHEET);
    let margin = ((ui.available_width() - width) / 2.0).max(0.0);
    ui.horizontal(|ui| {
        ui.add_space(margin);
        ui.vertical(|ui| {
            ui.set_width(width);
            content(ui);
        });
    });
}

fn top_space(ui: &egui::Ui) -> f32 {
    if ui.available_width() > space::STACK && ui.ctx().screen_rect().height() >= 900.0 {
        space::XXXL
    } else {
        space::XL
    }
}

/// The platen-colored strip that keeps the destination and primary action in
/// view at every window height.
fn action_bar(ctx: &egui::Context, content: impl FnOnce(&mut egui::Ui)) {
    egui::TopBottomPanel::bottom("workspace-actions")
        .frame(
            egui::Frame::new()
                .fill(color::PLATEN)
                .inner_margin(egui::Margin::symmetric(space::XL as i8, space::M as i8)),
        )
        .show(ctx, |ui| sheet(ui, content));
}

fn prepare(ui: &mut egui::Ui, app: &mut OpenScanlineApp, batch: &mut bool) {
    stage(ui, "New job", false);
    title(ui, &app.state.output_name, "Untitled scan");
    lead(ui, plan::sentence(&app.state, *batch));
    banners(ui, app);
    ui.add_space(space::XXL);
    ui.add_enabled_ui(!app.job_active(), |ui| {
        columns(ui, |ui, left| {
            if left {
                settings::scan(ui, &mut app.state, batch);
            } else {
                settings::output(ui, &mut app.state, *batch);
            }
        });
    });
    if app.state.error_message.is_none() && !app.state.status.is_empty() {
        ui.add_space(space::XL);
        note(ui, &app.state.status);
    }
}

fn banners(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    if let Some(error) = &app.state.error_message {
        ui.add_space(space::L);
        banner(ui, "The last job did not finish", error);
    }
    if let Some(error) = app.state.config_load_error.clone() {
        ui.add_space(space::L);
        banner(
            ui,
            "Settings could not be loaded; defaults are in use",
            &error,
        );
        if ui.add(secondary("Open settings recovery")).clicked() {
            app.state.active_tab = 5;
            ui.ctx()
                .data_mut(|data| data.insert_temp(egui::Id::new(TOOLS), true));
        }
    }
}

fn columns(ui: &mut egui::Ui, mut content: impl FnMut(&mut egui::Ui, bool)) {
    if ui.available_width() >= space::STACK {
        let gutter = space::XXXL + space::L;
        let spacing = ui.spacing().item_spacing.x;
        let width = (ui.available_width() - gutter - spacing) / 2.0;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(width);
                content(ui, true);
            });
            ui.add_space(gutter);
            ui.vertical(|ui| {
                ui.set_width(width);
                content(ui, false);
            });
        });
    } else {
        content(ui, true);
        ui.add_space(space::XXL);
        content(ui, false);
    }
}

/// Why Start is unavailable, in the order a person would fix it.
fn blocker(state: &GuiState, batch: bool) -> Option<&'static str> {
    if state.device.trim().is_empty() {
        Some("Choose a scanner to start.")
    } else if crate::domain::settings::validate_output_name(&state.output_name).is_err() {
        Some("Fix the file name to start.")
    } else if state.output_dir.trim().is_empty() {
        Some("Choose a folder to start.")
    } else {
        output_blocker(state, batch).or_else(|| duplex_blocker(state, batch))
    }
}

fn output_blocker(state: &GuiState, batch: bool) -> Option<&'static str> {
    let pdf = plan::writes_pdf(state, batch);
    if !pdf && (state.searchable_pdf || !state.pdf_password.is_empty()) {
        Some("Searchable text and passwords need PDF output.")
    } else if batch && state.output_fmt == "jxl" {
        Some("JPEG XL holds one image. Choose another format.")
    } else {
        None
    }
}

fn duplex_blocker(state: &GuiState, batch: bool) -> Option<&'static str> {
    if !state.duplex {
        None
    } else if !batch || state.scan_mode() != crate::domain::acquisition::ScanMode::Document {
        Some("Duplex needs the document feeder and multiple sides.")
    } else if !state.batch_pages.is_multiple_of(2) {
        Some("Duplex needs an even side limit.")
    } else {
        None
    }
}

fn can_start(state: &GuiState, batch: bool) -> bool {
    blocker(state, batch).is_none()
}

fn prepare_actions(ui: &mut egui::Ui, app: &mut OpenScanlineApp, batch: bool) {
    let valid = can_start(&app.state, batch);
    if ui.available_width() >= 700.0 {
        ui.horizontal(|ui| {
            destination(ui, app);
            start_action(ui, app, batch, valid);
        });
    } else {
        ui.horizontal_wrapped(|ui| destination(ui, app));
        ui.add_space(space::S);
        start_action(ui, app, batch, valid);
    }
}

fn destination(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    let label = ui.label(label_text("Save to"));
    ui.add_enabled_ui(!app.job_active(), |ui| {
        let width = (ui.available_width() - 330.0).clamp(160.0, 380.0);
        ui.add(
            egui::TextEdit::singleline(&mut app.state.output_dir)
                .font(theme::mono(size::MONO))
                .desired_width(width)
                .margin(egui::Margin::symmetric(10, 10)),
        )
        .labelled_by(label.id);
        if ui.add(secondary("Choose…")).clicked() {
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
    let reason = blocker(&app.state, batch);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let response = ui
            .add_enabled(valid && !app.job_active(), primary("Start scan"))
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() {
            if batch {
                app.start_batch();
            } else {
                app.start_scan(false, false);
            }
        }
        if let Some(reason) = reason {
            ui.label(
                RichText::new(reason)
                    .font(theme::body(size::SMALL))
                    .color(color::LAMP_INK),
            );
        } else if ui.available_width() > 120.0 {
            note(ui, plan::limit_label(&app.state, batch));
        }
        response
    })
    .inner
}

#[cfg(test)]
mod tests;
