use super::*;
use egui::Stroke;

const HEIGHT: f32 = 52.0;

pub(in crate::inbound::gui::view) fn render_header(
    ctx: &egui::Context,
    app: &mut OpenScanlineApp,
) -> bool {
    let mut tools =
        ctx.data_mut(|data| data.get_temp::<bool>(egui::Id::new(TOOLS)).unwrap_or(false));
    egui::TopBottomPanel::top("workspace-header")
        .exact_height(HEIGHT)
        .frame(
            egui::Frame::new()
                .fill(color::PLATEN)
                .inner_margin(egui::Margin::symmetric(space::XL as i8, 0)),
        )
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| navigation(ui, app, &mut tools));
        });
    ctx.data_mut(|data| data.insert_temp(egui::Id::new(TOOLS), tools));
    tools
}

fn navigation(ui: &mut egui::Ui, app: &mut OpenScanlineApp, tools: &mut bool) {
    wordmark(ui);
    ui.add_space(space::XXL);
    if tab(ui, "Scan", !*tools, true).clicked() {
        *tools = false;
    }
    if tab(ui, "Image tools", *tools, !app.job_active()).clicked() {
        *tools = true;
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        actions(ui, app, tools)
    });
}

/// A page outline crossed by the lamp line, then the name.
fn wordmark(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 20.0), egui::Sense::hover());
    let page = egui::Rect::from_center_size(rect.center(), egui::vec2(12.0, 16.0));
    let painter = ui.painter();
    painter.rect_stroke(
        page,
        0.0,
        Stroke::new(1.5_f32, color::INK),
        egui::StrokeKind::Inside,
    );
    let y = page.top() + page.height() * 0.62;
    painter.hline(
        page.left() - 3.0..=page.right() + 3.0,
        y,
        Stroke::new(theme::FOCUS, color::LAMP),
    );
    if ui.available_width() > 420.0 {
        ui.label(RichText::new("Open Scanline").font(theme::bold(15.0)));
    }
}

/// Header tab: ink underline marks the current mode.
fn tab(ui: &mut egui::Ui, label: &str, selected: bool, enabled: bool) -> egui::Response {
    let text = if selected {
        RichText::new(label).font(theme::bold(size::BODY))
    } else {
        RichText::new(label).color(color::GRAPHITE)
    };
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(text).frame(false).selected(false),
    );
    focus_ring(ui, &response);
    if selected {
        let rect = response.rect;
        let y = ui.max_rect().bottom() - 1.0;
        ui.painter()
            .hline(rect.x_range(), y, Stroke::new(3.0_f32, color::INK));
    }
    response
}

fn actions(ui: &mut egui::Ui, app: &mut OpenScanlineApp, tools: &mut bool) {
    // The menu opener reads as a header command, not a framed button.
    let widgets = &mut ui.visuals_mut().widgets;
    widgets.inactive.bg_stroke = Stroke::NONE;
    widgets.inactive.weak_bg_fill = color::PLATEN;
    ui.menu_button("More", |ui| advanced_actions(ui, app));
    if app.job_active() && (*tools || app.job_report().is_none()) {
        cancel(ui, app);
    } else if quiet(ui, "Open image…", !app.job_active()).clicked() {
        open_file(app, tools);
    }
}

fn cancel(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    let cancelling = app
        .state
        .cancel_requested
        .load(std::sync::atomic::Ordering::Relaxed);
    let label = if cancelling {
        "Cancelling…"
    } else {
        "Cancel"
    };
    if ui.add_enabled(!cancelling, secondary(label)).clicked() {
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
    ui.add_enabled_ui(!app.job_active(), |ui| {
        super::super::render_file_menu(ui, &ctx, app);
        super::super::render_edit_menu(ui, app);
        super::super::render_scan_menu(ui, app);
        super::super::render_view_menu(ui, &mut app.state);
        super::super::render_profile_menu(ui, &mut app.state);
    });
    super::super::render_help_menu(ui, &mut app.state);
}
