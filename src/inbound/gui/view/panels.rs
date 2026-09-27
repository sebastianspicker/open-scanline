use super::super::app::OpenScanlineApp;
use crate::domain::processing::list_film_profiles;
use crate::inbound::i18n;

#[cfg(feature = "gui")]
pub(super) fn render_side_panel(ctx: &egui::Context, app: &mut OpenScanlineApp) {
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
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_enabled_ui(!app.job_active(), |ui| match app.state.active_tab {
                    0 => render_input_tab(ui, app),
                    1 => render_crop_tab(ui, &mut app.state),
                    2 => render_filter_tab(ui, &mut app.state),
                    3 => render_color_tab(ui, &mut app.state),
                    4 => render_output_tab(ui, &mut app.state),
                    _ => render_preferences_tab(ui, &mut app.state),
                });
            });
        });
}

#[cfg(feature = "gui")]
fn render_input_tab(ui: &mut egui::Ui, app: &mut OpenScanlineApp) {
    let batch_enabled = !app.job_active();
    let start_batch = {
        let state = &mut app.state;
        render_device_controls(ui, state);
        render_source_controls(ui, state);
        render_scan_dimensions(ui, state);
        render_batch_controls(ui, state, batch_enabled)
    };
    if start_batch {
        app.start_batch();
    }
}

#[cfg(feature = "gui")]
fn render_device_controls(ui: &mut egui::Ui, state: &mut super::super::state::GuiState) {
    ui.label(state.translator.t("input.device"));
    let selected_device = state.device.clone();
    egui::ComboBox::from_id_salt("device")
        .selected_text(&state.device)
        .show_ui(ui, |ui| {
            for device in state.devices.clone() {
                ui.selectable_value(&mut state.device, device.clone(), device);
            }
        });
    if state.discovery_loading {
        ui.spinner();
        ui.label("Discovering scanners and capabilities…");
    }
    if state.devices.is_empty() {
        ui.label("No scanners found");
    }
    if state.device != selected_device {
        state.refresh_maintenance_capabilities();
    }
    if ui.button(state.translator.t("devices")).clicked() {
        state.refresh_devices();
    }
}

#[cfg(feature = "gui")]
fn render_source_controls(ui: &mut egui::Ui, state: &mut super::super::state::GuiState) {
    ui.label(state.translator.t("input.mode"));
    ui.horizontal(|ui| {
        for media in ["flatbed", "adf", "film"] {
            ui.selectable_value(&mut state.media, media.into(), media);
        }
    });
    let document_source = state.scan_mode() == crate::domain::acquisition::ScanMode::Document;
    ui.add_enabled_ui(document_source || state.duplex, |ui| {
        ui.checkbox(&mut state.duplex, "Duplex");
    });
}

#[cfg(feature = "gui")]
fn render_scan_dimensions(ui: &mut egui::Ui, state: &mut super::super::state::GuiState) {
    ui.add(egui::Slider::new(&mut state.width, 16..=4096).text("W"));
    ui.add(egui::Slider::new(&mut state.height, 16..=4096).text("H"));
    ui.add(egui::Slider::new(&mut state.dpi, 50..=1200).text(state.translator.t("input.dpi")));
    ui.add(
        egui::Slider::new(&mut state.batch_pages, 1..=99).text(state.translator.t("batch.pages")),
    );
}

#[cfg(feature = "gui")]
fn render_batch_controls(
    ui: &mut egui::Ui,
    state: &mut super::super::state::GuiState,
    batch_enabled: bool,
) -> bool {
    let mut start_batch = false;
    ui.horizontal(|ui| {
        render_maintenance_actions(ui, state);
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
    start_batch
}

#[cfg(feature = "gui")]
pub(super) fn render_maintenance_actions(
    ui: &mut egui::Ui,
    state: &mut super::super::state::GuiState,
) {
    let calibrate = ui
        .add_enabled(
            state.calibration_available(),
            egui::Button::new("Calibrate"),
        )
        .on_disabled_hover_text(state.calibration_explanation());
    if calibrate.clicked() {
        state.do_calibrate();
    }
    let focus = ui
        .add_enabled(state.focus_available(), egui::Button::new("Focus centre"))
        .on_disabled_hover_text(state.focus_explanation());
    if focus.clicked() {
        state.do_focus();
    }
}

#[cfg(feature = "gui")]
fn render_crop_tab(ui: &mut egui::Ui, state: &mut super::super::state::GuiState) {
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
fn render_filter_tab(ui: &mut egui::Ui, state: &mut super::super::state::GuiState) {
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
fn render_color_tab(ui: &mut egui::Ui, state: &mut super::super::state::GuiState) {
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
fn render_output_tab(ui: &mut egui::Ui, state: &mut super::super::state::GuiState) {
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
fn render_preferences_tab(ui: &mut egui::Ui, state: &mut super::super::state::GuiState) {
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
        ui.selectable_value(&mut state.ocr_engine, "ocrs".into(), "OCRS (local model)");
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
pub(super) fn render_preview(
    ctx: &egui::Context,
    state: &super::super::state::GuiState,
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
                egui::Color32::from_rgb(160, 62, 42),
                translator.t_args("status.error", &[("msg", &translator.t("preview"))]),
            );
        }
        if let Some(statistics) = &state.preview {
            render_histogram(ui, statistics, &translator);
        }
    });
}

#[cfg(feature = "gui")]
fn render_histogram(
    ui: &mut egui::Ui,
    statistics: &super::super::state::PreviewStatistics,
    translator: &crate::inbound::i18n::Translator,
) {
    ui.label(format!(
        "{}x{} {:?}",
        statistics.width, statistics.height, statistics.format
    ));
    let luma = &statistics.luma;
    let max = luma.iter().copied().max().unwrap_or(1).max(1) as f32;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().min(512.0), 80.0),
        egui::Sense::hover(),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, egui::Color32::from_gray(235));
    let width = rect.width() / 256.0;
    for (index, value) in luma.iter().enumerate() {
        let height = *value as f32 / max * rect.height();
        let x = rect.left() + index as f32 * width;
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(x, rect.bottom() - height),
                egui::pos2(x + width.max(1.0), rect.bottom()),
            ),
            0.0,
            egui::Color32::from_rgb(58, 103, 136),
        );
    }
    ui.label(translator.t("hist.title"));
}
