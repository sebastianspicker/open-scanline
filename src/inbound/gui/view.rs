use super::app::OpenScanlineApp;
use crate::domain::processing::list_film_profiles;

mod panels;
mod theme;
mod workspace;
use panels::{render_preview, render_side_panel};
pub(in crate::inbound::gui) use theme::apply_theme;

#[cfg(feature = "gui")]
impl eframe::App for OpenScanlineApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_theme(egui::Theme::Light);
        let close_requested = ctx.input(|input| input.viewport().close_requested());
        let job_was_active = self.job_active() || !self.discovery_finished();
        if close_requested {
            self.request_close(ctx);
        }
        self.drain_job_events(ctx);
        self.poll_discovery(ctx);
        if close_requested && job_was_active {
            // Do not combine Close with this frame's CancelClose. eframe treats
            // CancelClose as authoritative for the current native close event.
            ctx.request_repaint();
        } else {
            self.finish_deferred_close(ctx);
        }
        let tools = workspace::render_header(ctx, self);
        if tools {
            render_toolbar(ctx, self);
            render_status(ctx, &self.state);
            render_side_panel(ctx, self);
            self.ensure_preview_texture(ctx);
            render_preview(ctx, &self.state, self.preview_tex.as_ref());
        } else {
            workspace::render(ctx, self);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.cancel_and_join_job();
    }
}

#[cfg(feature = "gui")]
fn render_file_menu(ui: &mut egui::Ui, ctx: &egui::Context, app: &mut OpenScanlineApp) {
    let mut exit_requested = false;
    let title = app.state.translator.t("menu.file");
    ui.menu_button(title, |ui| {
        if ui.button(app.state.translator.t("button.open")).clicked() {
            app.state.do_open_file(None);
        }
        ui.add_enabled_ui(!app.job_active(), |ui| {
            if ui.button(app.state.translator.t("button.save")).clicked() {
                app.start_save();
            }
            if ui
                .button(app.state.translator.t("button.save_plus"))
                .clicked()
            {
                app.start_save_plus();
            }
        });
        if ui
            .button(app.state.translator.t("button.save_config"))
            .clicked()
        {
            app.state.do_save_config();
        }
        exit_requested = ui.button(app.state.translator.t("menu.exit")).clicked();
    });
    if exit_requested {
        app.request_close(ctx);
    }
}

#[cfg(feature = "gui")]
fn render_edit_menu(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    let title = app.state.translator.t("menu.edit");
    ui.menu_button(title, |ui| {
        ui.add_enabled_ui(!app.job_active(), |ui| {
            if ui
                .button(app.state.translator.t("button.reprocess"))
                .clicked()
            {
                app.start_reprocess();
            }
        });
    });
}

#[cfg(feature = "gui")]
fn render_scan_menu(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    let title = app.state.translator.t("menu.scan");
    ui.menu_button(title, |ui| {
        let preview = app.state.translator.t("button.preview");
        let scan = app.state.translator.t("button.scan");
        let batch = app.state.translator.t("batch");
        ui.add_enabled_ui(!app.job_active(), |ui| {
            if ui.button(preview).clicked() {
                app.start_scan(true, false);
            }
            if ui.button(scan).clicked() {
                app.start_scan(false, false);
            }
            if ui.button(batch).clicked() {
                app.start_batch();
            }
        });
        panels::render_maintenance_actions(ui, &mut app.state);
        render_action_button(
            ui,
            &mut app.state,
            "filter.brightness",
            super::state::GuiState::do_exposure,
        );
        render_action_button(
            ui,
            &mut app.state,
            "devices",
            super::state::GuiState::refresh_devices,
        );
    });
}

#[cfg(feature = "gui")]
fn render_view_menu(ui: &mut egui::Ui, state: &mut super::state::GuiState) {
    ui.menu_button(state.translator.t("menu.view"), |ui| {
        render_action_button(ui, state, "button.zoom_in", |state| {
            state.zoom = (state.zoom * 1.25).min(4.0);
        });
        render_action_button(ui, state, "button.zoom_out", |state| {
            state.zoom = (state.zoom / 1.25).max(0.25);
        });
    });
}

#[cfg(feature = "gui")]
fn render_profile_menu(ui: &mut egui::Ui, state: &mut super::state::GuiState) {
    ui.menu_button(state.translator.t("output.icc_profile"), |ui| {
        render_action_button(
            ui,
            state,
            "output.icc_profile",
            super::state::GuiState::do_profile_scanner,
        );
        if ui.button(state.translator.t("film.profile")).clicked() {
            state.status = format!(
                "{}: {}",
                state.translator.t("film.profile"),
                list_film_profiles().len()
            );
        }
    });
}

#[cfg(feature = "gui")]
fn render_help_menu(ui: &mut egui::Ui, state: &mut super::state::GuiState) {
    ui.menu_button(state.translator.t("menu.help"), |ui| {
        ui.label(format!("{} {}", crate::APP_NAME, crate::VERSION));
        ui.label(
            state
                .translator
                .t_args("about.text", &[("version", crate::VERSION)]),
        );
    });
}

#[cfg(feature = "gui")]
fn render_toolbar(ctx: &egui::Context, app: &mut OpenScanlineApp) {
    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
        // Tool commands are secondary to the image, so they use compact controls.
        ui.spacing_mut().button_padding = egui::vec2(10.0, 4.0);
        ui.spacing_mut().interact_size.y = 30.0;
        ui.add_space(6.0);
        ui.add_enabled_ui(!app.job_active(), |ui| {
            render_toolbar_contents(ui, app);
        });
        if app.job_active() && ui.button(app.state.translator.t("button.cancel")).clicked() {
            app.cancel_job();
        }
    });
}

#[cfg(feature = "gui")]
fn render_scan_toolbar_actions(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    let preview = app.state.translator.t("button.preview");
    let scan = app.state.translator.t("button.scan");
    let scan_plus = app.state.translator.t("button.scan_plus");
    ui.add_enabled_ui(!app.job_active(), |ui| {
        if ui.button(preview).clicked() {
            app.start_scan(true, false);
        }
        if ui.button(scan).clicked() {
            app.start_scan(false, false);
        }
        if ui.button(scan_plus).clicked() {
            app.start_scan(false, true);
        }
    });
}

#[cfg(feature = "gui")]
fn render_save_toolbar_actions(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    ui.add_enabled_ui(!app.job_active(), |ui| {
        if ui.button(app.state.translator.t("button.save")).clicked() {
            app.start_save();
        }
        if ui
            .button(app.state.translator.t("button.save_plus"))
            .clicked()
        {
            app.start_save_plus();
        }
    });
    let cancel = app.state.translator.t("button.cancel");
    if ui
        .add_enabled(
            app.job_active() || app.state.discovery_loading,
            egui::Button::new(cancel),
        )
        .clicked()
    {
        app.cancel_job();
        app.cancel_discovery();
    }
}

#[cfg(feature = "gui")]
fn render_transform_toolbar_actions(ui: &mut egui::Ui, state: &mut super::state::GuiState) {
    if ui.button("Rotate left").clicked() {
        state.rotate = (state.rotate - 90).rem_euclid(360);
    }
    if ui.button("Rotate right").clicked() {
        state.rotate = (state.rotate + 90).rem_euclid(360);
    }
    render_action_button(ui, state, "button.zoom_in", |state| {
        state.zoom = (state.zoom * 1.25).min(4.0);
    });
    render_action_button(ui, state, "button.zoom_out", |state| {
        state.zoom = (state.zoom / 1.25).max(0.25);
    });
    render_action_button(ui, state, "button.prev_frame", |state| {
        state.frame_index = (state.frame_index - 1).max(0);
    });
    render_action_button(ui, state, "button.next_frame", |state| {
        state.frame_index += 1;
    });
}

#[cfg(feature = "gui")]
fn render_postprocess_toolbar_actions(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    ui.add_enabled_ui(!app.job_active(), |ui| {
        if ui
            .button(app.state.translator.t("button.reprocess"))
            .clicked()
        {
            app.start_reprocess();
        }
        if ui.button(app.state.translator.t("button.ocr")).clicked() {
            app.start_ocr();
        }
    });
    render_action_button(
        ui,
        &mut app.state,
        "button.save_config",
        super::state::GuiState::do_save_config,
    );
}

#[cfg(feature = "gui")]
fn render_action_button(
    ui: &mut egui::Ui,
    state: &mut super::state::GuiState,
    label_key: &str,
    action: impl FnOnce(&mut super::state::GuiState),
) {
    if ui.button(state.translator.t(label_key)).clicked() {
        action(state);
    }
}

#[cfg(feature = "gui")]
fn render_status(ctx: &egui::Context, state: &super::state::GuiState) {
    egui::TopBottomPanel::bottom("status")
        .frame(egui::Frame::side_top_panel(&ctx.style()).fill(ctx.style().visuals.faint_bg_color))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&state.status);
                ui.separator();
                ui.label(&state.hist_summary);
                ui.separator();
                ui.label(format!("×{:.2} · #{}", state.zoom, state.frame_index));
            });
        });
}

#[cfg(feature = "gui")]
fn render_toolbar_contents(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    ui.horizontal_wrapped(|ui| {
        render_action_button(ui, &mut app.state, "button.open", |state| {
            state.do_open_file(None)
        });
        ui.separator();
        render_scan_toolbar_actions(ui, app);
        ui.separator();
        render_save_toolbar_actions(ui, app);
        ui.separator();
        render_transform_toolbar_actions(ui, &mut app.state);
        ui.separator();
        render_postprocess_toolbar_actions(ui, app);
    });
    ui.horizontal(|ui| {
        ui.label(app.state.translator.t("button.open"));
        ui.text_edit_singleline(&mut app.state.open_path_input);
        if ui.button(app.state.translator.t("open")).clicked() {
            app.state.do_open_file(None);
        }
    });
}
