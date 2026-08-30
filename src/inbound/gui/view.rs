use super::app::OpenScanlineApp;
use crate::domain::processing::histogram;
use crate::domain::processing::list_film_profiles;
use crate::inbound::i18n;
use crate::infrastructure::media::load_image;

#[cfg(feature = "gui")]
impl eframe::App for OpenScanlineApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let close_requested = ctx.input(|input| input.viewport().close_requested());
        let job_was_active = self.job_active();
        if close_requested {
            self.request_close(ctx);
        }
        self.drain_job_events(ctx);
        if close_requested && job_was_active {
            // Do not combine Close with this frame's CancelClose. eframe treats
            // CancelClose as authoritative for the current native close event.
            ctx.request_repaint();
        } else {
            self.finish_deferred_close(ctx);
        }
        render_menu(ctx, self);
        render_toolbar(ctx, self);
        render_status(ctx, &self.state);
        render_side_panel(ctx, self);
        self.ensure_preview_texture(ctx);
        render_preview(ctx, &self.state, self.preview_tex.as_ref());
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.cancel_and_join_job();
    }
}

#[cfg(feature = "gui")]
fn render_menu(ctx: &egui::Context, app: &mut OpenScanlineApp) {
    egui::TopBottomPanel::top("menu").show(ctx, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            render_file_menu(ui, ctx, app);
            render_edit_menu(ui, app);
            render_scan_menu(ui, app);
            render_view_menu(ui, &mut app.state);
            render_profile_menu(ui, &mut app.state);
            render_help_menu(ui, &mut app.state);
        });
    });
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
        render_action_button(
            ui,
            &mut app.state,
            "color.auto_color",
            super::state::GuiState::do_calibrate,
        );
        render_action_button(
            ui,
            &mut app.state,
            "filter.sharpen",
            super::state::GuiState::do_focus,
        );
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
        ui.horizontal_wrapped(|ui| {
            render_action_button(ui, &mut app.state, "button.open", |state| {
                state.do_open_file(None)
            });
            render_scan_toolbar_actions(ui, app);
            render_save_toolbar_actions(ui, app);
            render_transform_toolbar_actions(ui, &mut app.state);
            render_postprocess_toolbar_actions(ui, app);
        });
        ui.horizontal(|ui| {
            ui.label(app.state.translator.t("button.open"));
            ui.text_edit_singleline(&mut app.state.open_path_input);
            if ui.button(app.state.translator.t("open")).clicked() {
                app.state.do_open_file(None);
            }
        });
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
        .add_enabled(app.job_active(), egui::Button::new(cancel))
        .clicked()
    {
        app.cancel_job();
    }
}

#[cfg(feature = "gui")]
fn render_transform_toolbar_actions(ui: &mut egui::Ui, state: &mut super::state::GuiState) {
    render_action_button(ui, state, "button.rotate_l", |state| {
        state.rotate = (state.rotate - 90).rem_euclid(360);
    });
    render_action_button(ui, state, "button.rotate_r", |state| {
        state.rotate = (state.rotate + 90).rem_euclid(360);
    });
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
    egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
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
fn render_side_panel(ctx: &egui::Context, app: &mut OpenScanlineApp) {
    egui::SidePanel::left("tabs")
        .resizable(true)
        .default_width(280.0)
        .show(ctx, |ui| {
            ui.heading(app.state.translator.t("menu.image"));
            for (index, key) in [
                "tab.input",
                "tab.crop",
                "tab.filter",
                "tab.color",
                "tab.output",
                "menu.language",
            ]
            .iter()
            .enumerate()
            {
                if ui
                    .selectable_label(app.state.active_tab == index, app.state.translator.t(key))
                    .clicked()
                {
                    app.state.active_tab = index;
                }
            }
            ui.separator();
            match app.state.active_tab {
                0 => render_input_tab(ui, app),
                1 => render_crop_tab(ui, &mut app.state),
                2 => render_filter_tab(ui, &mut app.state),
                3 => render_color_tab(ui, &mut app.state),
                4 => render_output_tab(ui, &mut app.state),
                _ => render_preferences_tab(ui, &mut app.state),
            }
        });
}

#[cfg(feature = "gui")]
fn render_input_tab(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    let batch_enabled = !app.job_active();
    let mut start_batch = false;
    {
        let state = &mut app.state;
        ui.label(state.translator.t("input.device"));
        egui::ComboBox::from_id_salt("device")
            .selected_text(&state.device)
            .show_ui(ui, |ui| {
                for device in state.devices.clone() {
                    ui.selectable_value(&mut state.device, device.clone(), device);
                }
            });
        if ui.button(state.translator.t("devices")).clicked() {
            state.refresh_devices();
        }
        ui.label(state.translator.t("input.mode"));
        ui.horizontal(|ui| {
            for media in ["flatbed", "adf", "film"] {
                ui.selectable_value(&mut state.media, media.into(), media);
            }
        });
        let document_source = state.scan_mode() == crate::domain::acquisition::ScanMode::Document;
        if !document_source {
            state.duplex = false;
        }
        ui.add_enabled_ui(document_source, |ui| {
            ui.checkbox(&mut state.duplex, "Duplex");
        });
        ui.add(egui::Slider::new(&mut state.width, 16..=4096).text("W"));
        ui.add(egui::Slider::new(&mut state.height, 16..=4096).text("H"));
        ui.add(egui::Slider::new(&mut state.dpi, 50..=1200).text(state.translator.t("input.dpi")));
        ui.add(
            egui::Slider::new(&mut state.batch_pages, 1..=99)
                .text(state.translator.t("batch.pages")),
        );
        ui.horizontal(|ui| {
            if ui.button(state.translator.t("color.auto_color")).clicked() {
                state.do_calibrate();
            }
            if ui.button(state.translator.t("filter.sharpen")).clicked() {
                state.do_focus();
            }
            if ui
                .add_enabled(
                    batch_enabled,
                    egui::Button::new(state.translator.t("batch")),
                )
                .clicked()
            {
                start_batch = true;
            }
        });
    }
    if start_batch {
        app.start_batch();
    }
}

#[cfg(feature = "gui")]
fn render_crop_tab(ui: &mut egui::Ui, state: &mut super::state::GuiState) {
    ui.label(state.translator.t("crop.region"));
    ui.text_edit_singleline(&mut state.crop_text);
    ui.checkbox(&mut state.flip_h, "↔");
    ui.checkbox(&mut state.flip_v, "↕");
    ui.label("Rotation");
    ui.horizontal(|ui| {
        for degrees in [0, 90, 180, 270] {
            ui.selectable_value(&mut state.rotate, degrees, format!("{degrees}°"));
        }
    });
    ui.checkbox(&mut state.auto_orient, state.translator.t("auto_orient"));
    ui.checkbox(&mut state.auto_crop, state.translator.t("auto_crop"));
    ui.checkbox(&mut state.auto_deskew, state.translator.t("deskew"));
    ui.add(
        egui::Slider::new(&mut state.deskew_angle, -45.0..=45.0).text(state.translator.t("deskew")),
    );
}

#[cfg(feature = "gui")]
fn render_filter_tab(ui: &mut egui::Ui, state: &mut super::state::GuiState) {
    ui.label(state.translator.t("filter.sharpen"));
    ui.add(
        egui::Slider::new(&mut state.sharpen_amount, 0.0..=3.0)
            .text(state.translator.t("filter.sharpen")),
    );
    ui.checkbox(&mut state.descreen, "Descreen");
    if state.descreen {
        ui.add(egui::Slider::new(&mut state.descreen_dpi, 50..=1200).text("Descreen DPI"));
    }
    ui.checkbox(
        &mut state.restore_colors,
        state.translator.t("color.auto_color"),
    );
    ui.checkbox(
        &mut state.restore_fading,
        state.translator.t("filter.edge_fade"),
    );
    ui.checkbox(&mut state.flatten, "Flatten background");
    ui.checkbox(&mut state.hole_punch, "Remove hole punches");
    ui.checkbox(&mut state.invert_colors, state.translator.t("color.invert"));
    render_choice(
        ui,
        state.translator.t("filter.dust_removal"),
        &mut state.infrared_clean,
        ["off", "light", "medium", "heavy"],
    );
    render_choice(
        ui,
        state.translator.t("filter.denoise"),
        &mut state.grain_reduction,
        ["off", "light", "medium", "heavy"],
    );
    render_choice(
        ui,
        state.translator.t("color.auto_color"),
        &mut state.colorize_mode,
        ["off", "sepia", "cool", "warm"],
    );
}

#[cfg(feature = "gui")]
fn render_choice<const N: usize>(
    ui: &mut egui::Ui,
    label: String,
    value: &mut String,
    choices: [&str; N],
) {
    ui.label(label);
    ui.horizontal(|ui| {
        for choice in choices {
            ui.selectable_value(value, choice.into(), choice);
        }
    });
}

#[cfg(feature = "gui")]
fn render_color_tab(ui: &mut egui::Ui, state: &mut super::state::GuiState) {
    ui.add(
        egui::Slider::new(&mut state.brightness, -100..=100)
            .text(state.translator.t("filter.brightness")),
    );
    ui.add(
        egui::Slider::new(&mut state.contrast, -100..=100)
            .text(state.translator.t("filter.contrast")),
    );
    ui.add(egui::Slider::new(&mut state.saturation, -100.0..=100.0).text("Saturation"));
    ui.add(egui::Slider::new(&mut state.hue, -180.0..=180.0).text("Hue (degrees)"));
    ui.label("Tone curve (x:y,x:y; x values must increase)");
    ui.text_edit_singleline(&mut state.curves_text);
    ui.checkbox(&mut state.desaturate, state.translator.t("film.bw"));
    ui.checkbox(
        &mut state.white_balance,
        state.translator.t("white_balance"),
    );
    ui.checkbox(&mut state.auto_levels, state.translator.t("auto_levels"));
    ui.add(
        egui::Slider::new(&mut state.levels_black, 0..=254)
            .text(state.translator.t("color.levels")),
    );
    ui.add(
        egui::Slider::new(&mut state.levels_white, 1..=255)
            .text(state.translator.t("color.levels")),
    );
    ui.add(
        egui::Slider::new(&mut state.levels_gamma, 0.2..=3.0)
            .text(state.translator.t("color.gamma")),
    );
    ui.label(state.translator.t("film.profile"));
    egui::ComboBox::from_id_salt("film")
        .selected_text(if state.film_type.is_empty() {
            "(none)"
        } else {
            &state.film_type
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut state.film_type, String::new(), "(none)");
            for profile in list_film_profiles() {
                ui.selectable_value(
                    &mut state.film_type,
                    profile.id.clone(),
                    format!("{} — {}", profile.id, profile.name),
                );
            }
        });
}

#[cfg(feature = "gui")]
fn render_output_tab(ui: &mut egui::Ui, state: &mut super::state::GuiState) {
    ui.label(state.translator.t("output.output_dir"));
    ui.text_edit_singleline(&mut state.output_dir);
    ui.label(state.translator.t("output.filename_template"));
    ui.text_edit_singleline(&mut state.output_name);
    ui.label(state.translator.t("output.format"));
    ui.horizontal(|ui| {
        for format in ["png", "jpg", "tif", "webp", "bmp", "gif", "pdf", "jxl"] {
            ui.selectable_value(&mut state.output_fmt, format.into(), format);
        }
    });
    ui.checkbox(
        &mut state.multipage,
        state.translator.t("batch.multipage_pdf"),
    );
    ui.checkbox(
        &mut state.save_raw,
        format!("{} RAW TIFF", state.translator.t("output.save")),
    );
    ui.checkbox(
        &mut state.contact_sheet,
        format!("{} BMP", state.translator.t("batch")),
    );
    ui.checkbox(&mut state.searchable_pdf, "Searchable PDF");
    ui.label("PDF password (runtime only)");
    ui.add(egui::TextEdit::singleline(&mut state.pdf_password).password(true));
    ui.label("Scanner profile path");
    ui.text_edit_singleline(&mut state.scanner_profile_path);
    if state.multipage {
        ui.horizontal(|ui| {
            ui.label(state.translator.t("output.format"));
            for format in ["pdf", "tif"] {
                ui.selectable_value(&mut state.multipage_format, format.into(), format);
            }
        });
    }
}

#[cfg(feature = "gui")]
fn render_preferences_tab(ui: &mut egui::Ui, state: &mut super::state::GuiState) {
    ui.label(state.translator.t("menu.language"));
    for language in i18n::available_languages() {
        if ui
            .selectable_label(state.language == language, i18n::language_name(&language))
            .clicked()
        {
            state.language = state.translator.set_language(&language);
            state.status = state.translator.t("ready");
        }
    }
    ui.separator();
    ui.label("OCR engine");
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.ocr_engine, "offline".into(), "Built-in offline");
        ui.selectable_value(&mut state.ocr_engine, "tesseract".into(), "Tesseract");
    });
    ui.label(state.translator.t("ocr.language"));
    ui.text_edit_singleline(&mut state.ocr_language);
    ui.label(format!(
        "{}: {}",
        state.translator.t("save_config"),
        state.config_path.display()
    ));
    if ui
        .button(state.translator.t("button.save_config"))
        .clicked()
    {
        state.do_save_config();
    }
    if state.config_recovery_required
        && ui
            .button("Reset configuration (replace invalid file)")
            .clicked()
    {
        state.do_reset_config_recovery();
    }
}

#[cfg(feature = "gui")]
fn render_preview(
    ctx: &egui::Context,
    state: &super::state::GuiState,
    texture: Option<&egui::TextureHandle>,
) {
    let last_image = state.last_image.clone();
    let translator = state.translator.clone();
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading(format!("{} {}", crate::APP_NAME, crate::VERSION));
        ui.label(translator.t("status.ready"));
        let Some(path) = last_image else {
            ui.label(translator.t("preview"));
            return;
        };
        ui.label(format!(
            "{}: {}",
            translator.t("output.save"),
            path.display()
        ));
        if let Some(texture) = texture {
            let size = texture.size_vec2() * state.zoom;
            let scale = if size.x > ui.available_width().min(900.0) {
                ui.available_width().min(900.0) / size.x
            } else {
                1.0
            };
            ui.add(egui::Image::new((texture.id(), size * scale)));
            ui.label(format!(
                "{} {}×{} · ×{:.2}",
                translator.t("preview"),
                texture.size()[0],
                texture.size()[1],
                state.zoom
            ));
        } else {
            ui.colored_label(
                egui::Color32::YELLOW,
                translator.t_args("status.error", &[("msg", &translator.t("preview"))]),
            );
        }
        render_histogram(ui, &path, &translator);
    });
}

#[cfg(feature = "gui")]
fn render_histogram(
    ui: &mut egui::Ui,
    path: &std::path::Path,
    translator: &crate::inbound::i18n::Translator,
) {
    let Ok(image) = load_image(path) else {
        return;
    };
    ui.label(format!(
        "{}x{} {:?}",
        image.width, image.height, image.pixel_format
    ));
    let Ok(histogram) = histogram(&image) else {
        return;
    };
    let Some(luma) = histogram.get("luma").and_then(|value| value.as_array()) else {
        return;
    };
    let max = luma
        .iter()
        .filter_map(|value| value.as_u64())
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().min(512.0), 80.0),
        egui::Sense::hover(),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, egui::Color32::from_gray(30));
    let width = rect.width() / 256.0;
    for (index, value) in luma.iter().enumerate() {
        let height = value.as_u64().unwrap_or(0) as f32 / max * rect.height();
        let x = rect.left() + index as f32 * width;
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(x, rect.bottom() - height),
                egui::pos2(x + width.max(1.0), rect.bottom()),
            ),
            0.0,
            egui::Color32::LIGHT_GREEN,
        );
    }
    ui.label(translator.t("hist.title"));
}
