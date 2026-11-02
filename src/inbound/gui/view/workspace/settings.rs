use super::*;
use crate::domain::acquisition::PaperSize;
use crate::inbound::gui::state::paper_label;

pub(super) fn scan(ui: &mut egui::Ui, state: &mut GuiState, batch: &mut bool) {
    section(ui, "1", "ws.sec_scanner");
    scanner(ui, state);
    ui.add_space(space::L);
    source_capture(ui, state, batch);
    ui.add_space(space::M);
    sides_resolution(ui, state, *batch);
    side_limit(ui, state, *batch);
    ui.add_space(space::L);
    scan_area(ui, state);
    if quiet(ui, &tr("ws.image_corrections"), true).clicked() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new(TOOLS), true));
        state.active_tab = 1;
    }
}

pub(super) fn output(ui: &mut egui::Ui, state: &mut GuiState, batch: bool) {
    section(ui, "2", "ws.sec_files");
    field(ui, "ws.file_name", &mut state.output_name);
    if let Err(error) = crate::domain::settings::validate_output_name(&state.output_name) {
        problem(ui, plan::name_problem(&error.to_string()));
    }
    ui.add_space(space::M);
    choice(
        ui,
        "workspace-format",
        "ws.format",
        &mut state.output_fmt,
        &[
            ("pdf", "ws.fmt_pdf"),
            ("png", "ws.fmt_png"),
            ("jpg", "ws.fmt_jpg"),
            ("tif", "ws.fmt_tif"),
            ("webp", "ws.fmt_webp"),
            ("bmp", "ws.fmt_bmp"),
            ("gif", "ws.fmt_gif"),
            ("jxl", "ws.fmt_jxl"),
        ],
    );
    if batch {
        document_controls(ui, state);
        if state.output_fmt == "jxl" {
            problem(ui, tr("ws.jxl_multi"));
        }
    }
    plan::ledger(ui, state, batch);
    pdf_controls(ui, state, batch);
}

fn document_controls(ui: &mut egui::Ui, state: &mut GuiState) {
    if state.output_fmt == "pdf" {
        return;
    }
    ui.add_space(space::S);
    ui.checkbox(&mut state.multipage, tr("ws.combine_pages"));
    if state.multipage {
        ui.add_space(space::XS);
        choice(
            ui,
            "workspace-container",
            "Document format",
            &mut state.multipage_format,
            &[("pdf", "PDF"), ("tif", "Multipage TIFF")],
        );
    }
}

fn scanner(ui: &mut egui::Ui, state: &mut GuiState) {
    let device_label = ui.label(label_text(&tr("ws.scanner")));
    let old = state.device.clone();
    egui::ComboBox::from_id_salt("workspace-device")
        .width(ui.available_width())
        .selected_text(
            RichText::new(state.device_label(&state.device)).font(theme::mono(size::MONO)),
        )
        .show_ui(ui, |ui| {
            if state.device_missing() {
                let device = state.device.clone();
                let label =
                    RichText::new(state.device_label(&device)).font(theme::mono(size::MONO));
                ui.selectable_value(&mut state.device, device, label);
            }
            for device in &state.devices {
                ui.selectable_value(
                    &mut state.device,
                    device.clone(),
                    RichText::new(device).font(theme::mono(size::MONO)),
                );
            }
        })
        .response
        .labelled_by(device_label.id);
    if old != state.device {
        state.refresh_maintenance_capabilities();
    }
    ui.horizontal_wrapped(|ui| {
        note(ui, tr(scanner_note(state)));
        if quiet(ui, &tr("ws.refresh"), true).clicked() {
            state.refresh_devices();
        }
    });
}

fn scanner_note(state: &GuiState) -> &'static str {
    if state.discovery_loading {
        "ws.looking_for_scanners"
    } else if state.device_missing() {
        "ws.scanner_missing"
    } else if state.devices.is_empty() {
        "ws.no_scanner"
    } else if state.device == "mock" {
        "ws.scanner_mock"
    } else if state.device.starts_with("file:") {
        "ws.scanner_file"
    } else {
        "ws.scanner_adjusts"
    }
}

fn source_capture(ui: &mut egui::Ui, state: &mut GuiState, batch: &mut bool) {
    ui.columns(2, |columns| {
        choice(
            &mut columns[0],
            "workspace-source",
            "ws.paper_source",
            &mut state.media,
            &[
                ("flatbed", "ws.media_flatbed"),
                ("document", "ws.media_feeder"),
                ("adf", "ADF"),
                ("film", "ws.media_film"),
            ],
        );
        segmented(
            &mut columns[1],
            "ws.capture",
            batch,
            [("ws.single_image", false), ("ws.multiple_sides", true)],
        );
    });
}

fn sides_resolution(ui: &mut egui::Ui, state: &mut GuiState, batch: bool) {
    let document = state.scan_mode() == crate::domain::acquisition::ScanMode::Document;
    ui.columns(2, |columns| {
        duplex_choice(&mut columns[0], state, batch, document);
        let ui = &mut columns[1];
        let label = ui.label(label_text(&tr("ws.resolution")));
        ui.add_sized(
            [ui.available_width(), size::CONTROL],
            egui::DragValue::new(&mut state.dpi)
                .range(50..=1200)
                .suffix(" dpi"),
        )
        .labelled_by(label.id)
        .on_hover_text(tr("ws.resolution_hint"));
    });
    if state.duplex && (!document || !batch) {
        problem(ui, tr("ws.duplex_needs_feeder"));
    }
}

fn duplex_choice(ui: &mut egui::Ui, state: &mut GuiState, batch: bool, document: bool) {
    let label = ui.label(label_text(&tr("ws.sides")));
    ui.add_enabled_ui((document && batch) || state.duplex, |ui| {
        egui::ComboBox::from_id_salt("workspace-sides")
            .width(ui.available_width())
            .selected_text(tr(if state.duplex {
                "ws.both_sides"
            } else {
                "ws.one_side"
            }))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.duplex, false, tr("ws.one_side"));
                ui.add_enabled_ui(document && batch, |ui| {
                    ui.selectable_value(&mut state.duplex, true, tr("ws.both_sides_duplex"));
                });
            })
            .response
            .labelled_by(label.id)
            .on_disabled_hover_text(tr("ws.both_sides_needs"));
    });
}

fn side_limit(ui: &mut egui::Ui, state: &mut GuiState, batch: bool) {
    if !batch {
        return;
    }
    ui.add_space(space::M);
    ui.horizontal(|ui| {
        let label = ui.label(label_text(&tr("ws.side_limit")));
        ui.add_sized(
            [72.0, size::CONTROL],
            egui::DragValue::new(&mut state.batch_pages).range(1..=1000),
        )
        .labelled_by(label.id);
        note(ui, side_limit_note(state));
    });
    if state.duplex && !state.batch_pages.is_multiple_of(2) {
        let suggestion = if state.batch_pages < 1000 {
            state.batch_pages + 1
        } else {
            state.batch_pages - 1
        };
        problem(
            ui,
            tr_args(
                "ws.duplex_pairs",
                &[("suggestion", &suggestion.to_string())],
            ),
        );
    }
}

fn side_limit_note(state: &GuiState) -> String {
    let sides = state.batch_pages;
    if !state.duplex || !sides.is_multiple_of(2) {
        return tr("ws.limit_note");
    }
    if state.device == "mock" {
        tr_args("ws.limit_simulated", &[("sides", &sides.to_string())])
    } else if state.device.starts_with("file:") {
        tr_args("ws.limit_repeated", &[("sides", &sides.to_string())])
    } else {
        tr_args("ws.limit_sheets", &[("sheets", &(sides / 2).to_string())])
    }
}

fn scan_area(ui: &mut egui::Ui, state: &mut GuiState) {
    ui.collapsing(
        RichText::new(tr_args(
            "ws.scan_area_summary",
            &[("summary", &state.scan_area_summary())],
        )),
        |ui| {
            paper_choice(ui, state);
            if state.custom_area {
                ui.horizontal_wrapped(|ui| {
                    let width_label = ui.label(label_text(&tr("ws.width")));
                    ui.add(
                        egui::DragValue::new(&mut state.width)
                            .range(1..=16384)
                            .suffix(" px"),
                    )
                    .labelled_by(width_label.id);
                    ui.add_space(space::S);
                    let height_label = ui.label(label_text(&tr("ws.height")));
                    ui.add(
                        egui::DragValue::new(&mut state.height)
                            .range(1..=16384)
                            .suffix(" px"),
                    )
                    .labelled_by(height_label.id);
                });
            }
            note(ui, tr("ws.area_note"));
        },
    );
}

fn paper_choice(ui: &mut egui::Ui, state: &mut GuiState) {
    let label_response = ui.label(label_text(&tr("ws.paper_size")));
    let selected = if state.custom_area {
        tr("ws.custom")
    } else {
        paper_text(state.paper)
    };
    let mut choice = None;
    egui::ComboBox::from_id_salt("workspace-paper")
        .width(ui.available_width())
        .height(size::CONTROL * 8.0)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for paper in PaperSize::ALL {
                let current = !state.custom_area && state.paper == paper;
                if ui.selectable_label(current, paper_text(paper)).clicked() {
                    choice = Some(Some(paper));
                }
            }
            if ui
                .selectable_label(state.custom_area, tr("ws.custom"))
                .clicked()
            {
                choice = Some(None);
            }
        })
        .response
        .labelled_by(label_response.id);
    match choice {
        Some(Some(paper)) => {
            state.paper = paper;
            state.custom_area = false;
        }
        Some(None) if !state.custom_area => {
            // Start the custom area from the size that was just selected.
            let (width, height) = state.requested_extent(state.dpi);
            let (width, height) = if width == 0 || height == 0 {
                PaperSize::A4
                    .to_pixels(state.dpi, state.dpi)
                    .unwrap_or((state.width.max(1), state.height.max(1)))
            } else {
                (width, height)
            };
            state.width = width;
            state.height = height;
            state.custom_area = true;
        }
        _ => {}
    }
}

fn paper_text(paper: PaperSize) -> String {
    match paper {
        PaperSize::Auto => tr("ws.device_maximum"),
        other => paper_label(other).to_owned(),
    }
}

fn pdf_controls(ui: &mut egui::Ui, state: &mut GuiState, batch: bool) {
    ui.add_space(space::L);
    let pdf = plan::writes_pdf(state, batch);
    ui.add_enabled_ui(pdf || state.searchable_pdf, |ui| {
        ui.checkbox(&mut state.searchable_pdf, tr("ws.searchable_pdf"));
    });
    if !pdf {
        if state.searchable_pdf || !state.pdf_password.is_empty() {
            problem(ui, tr("ws.pdf_conflict"));
        } else {
            note(ui, tr("ws.needs_pdf"));
        }
    }
    if state.searchable_pdf {
        ui.add_space(space::S);
        ui.add_enabled_ui(pdf, |ui| ocr_controls(ui, state));
    }
    ui.add_space(space::M);
    ui.add_enabled_ui(pdf || !state.pdf_password.is_empty(), |ui| {
        ui.collapsing(tr("ws.pdf_password"), |ui| {
            let label = ui.label(label_text(&tr("ws.password")));
            ui.add(
                egui::TextEdit::singleline(&mut state.pdf_password)
                    .password(true)
                    .desired_width(ui.available_width())
                    .margin(egui::Margin::symmetric(10, 10)),
            )
            .labelled_by(label.id);
            note(ui, tr("ws.password_note"));
        });
    });
    more_output(ui, state, batch);
}

fn more_output(ui: &mut egui::Ui, state: &mut GuiState, batch: bool) {
    ui.collapsing(tr("ws.more_output"), |ui| {
        ui.add_enabled_ui(!batch, |ui| {
            ui.checkbox(&mut state.save_raw, tr("ws.keep_raw"));
        });
        ui.add_enabled_ui(batch, |ui| {
            ui.checkbox(&mut state.contact_sheet, tr("ws.save_contact_sheet"));
        });
        ui.add_space(space::S);
        field(ui, "ws.scanner_profile", &mut state.scanner_profile_path);
    });
}

fn ocr_controls(ui: &mut egui::Ui, state: &mut GuiState) {
    if ui.available_width() >= 400.0 {
        ui.columns(2, |columns| {
            ocr_engine(&mut columns[0], &mut state.ocr_engine);
            field(&mut columns[1], "ws.text_language", &mut state.ocr_language);
        });
    } else {
        ocr_engine(ui, &mut state.ocr_engine);
        field(ui, "ws.text_language", &mut state.ocr_language);
    }
    note(
        ui,
        tr(match state.ocr_engine.as_str() {
            "tesseract" => "ws.ocr_tesseract_note",
            "ocrs" => "ws.ocr_ocrs_note",
            _ => "ws.ocr_basic_note",
        }),
    );
}

fn ocr_engine(ui: &mut egui::Ui, engine: &mut String) {
    choice(
        ui,
        "workspace-ocr",
        "ws.text_recognition",
        engine,
        &[
            ("offline", "ws.ocr_builtin"),
            ("tesseract", "Tesseract"),
            ("ocrs", "ws.ocr_ocrs"),
        ],
    );
}
