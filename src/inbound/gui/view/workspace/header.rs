use super::*;
use crate::inbound::gui::app::report::JobTerminal;

pub(in crate::inbound::gui::view) fn render_header(
    ctx: &egui::Context,
    app: &mut OpenScanlineApp,
) -> bool {
    let mut tools =
        ctx.data_mut(|data| data.get_temp::<bool>(egui::Id::new(TOOLS)).unwrap_or(false));
    egui::TopBottomPanel::top("workspace-header")
        .frame(
            egui::Frame::new()
                .fill(ctx.style().visuals.panel_fill)
                .inner_margin(egui::Margin::symmetric(30, 12)),
        )
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| navigation(ui, app, &mut tools));
        });
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(TOOLS), tools));
    tools
}

fn navigation(ui: &mut egui::Ui, app: &mut OpenScanlineApp, tools: &mut bool) {
    let stage = app
        .job_report()
        .map_or("New scan", |report| match report.terminal {
            None => "Scanning",
            Some(JobTerminal::Completed(_)) => "Saved",
            Some(JobTerminal::Cancelled) => "Cancelled",
            Some(JobTerminal::Failed) => "Scan stopped",
        });
    if ui.add(nav_button(stage, !*tools)).clicked() {
        *tools = false;
    }
    if ui
        .add_enabled(!app.job_active(), nav_button("Image tools", *tools))
        .clicked()
    {
        *tools = true;
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        actions(ui, app, tools)
    });
}

fn nav_button(label: &str, selected: bool) -> egui::Button<'_> {
    egui::Button::new(if selected {
        RichText::new(label).strong()
    } else {
        RichText::new(label)
    })
    .frame(false)
}

fn actions(ui: &mut egui::Ui, app: &mut OpenScanlineApp, tools: &mut bool) {
    if app.job_active() {
        cancel(ui, app);
    } else if ui
        .add(egui::Button::new("Use an image file").frame(false))
        .clicked()
    {
        open_file(app, tools);
    }
    ui.add_enabled_ui(!app.job_active(), |ui| {
        ui.menu_button("Advanced actions", |ui| advanced_actions(ui, app));
    });
}

fn cancel(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
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
            }),
        )
        .clicked()
    {
        app.cancel_job();
    }
}

fn open_file(app: &mut OpenScanlineApp, tools: &mut bool) {
    let previous_revision = app.state.preview_revision;
    let previous_input = std::mem::take(&mut app.state.open_path_input);
    app.state.do_open_file(None);
    if app.state.preview_revision != previous_revision {
        *tools = true;
    } else if app.state.open_path_input.is_empty() {
        app.state.open_path_input = previous_input;
    }
}

fn advanced_actions(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    let ctx = ui.ctx().clone();
    super::super::render_file_menu(ui, &ctx, app);
    super::super::render_edit_menu(ui, app);
    super::super::render_scan_menu(ui, app);
    super::super::render_view_menu(ui, &mut app.state);
    super::super::render_profile_menu(ui, &mut app.state);
    super::super::render_help_menu(ui, &mut app.state);
}
