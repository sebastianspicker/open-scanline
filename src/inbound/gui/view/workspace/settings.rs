use super::*;

pub(super) fn scan(ui: &mut egui::Ui, state: &mut GuiState, batch: &mut bool) {
    section(ui, "1", "Scanner and pages");
    scanner(ui, state);
    ui.add_space(space::L);
    source_capture(ui, state, batch);
    ui.add_space(space::M);
    sides_resolution(ui, state, *batch);
    side_limit(ui, state, *batch);
    ui.add_space(space::L);
    scan_area(ui, state);
    if quiet(ui, "Image corrections…", true).clicked() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new(TOOLS), true));
        state.active_tab = 1;
    }
}

pub(super) fn output(ui: &mut egui::Ui, state: &mut GuiState, batch: bool) {
    section(ui, "2", "Files");
    field(ui, "File name", &mut state.output_name);
    if let Err(error) = crate::domain::settings::validate_output_name(&state.output_name) {
        problem(ui, plan::name_problem(&error.to_string()));
    }
    ui.add_space(space::M);
    choice(
        ui,
        "workspace-format",
        "Format",
        &mut state.output_fmt,
        &[
            ("pdf", "PDF document"),
            ("png", "PNG image"),
            ("jpg", "JPEG image"),
            ("tif", "TIFF image"),
            ("webp", "WebP image"),
            ("bmp", "BMP image"),
            ("gif", "GIF image"),
            ("jxl", "JPEG XL image"),
        ],
    );
    if batch {
        document_controls(ui, state);
        if state.output_fmt == "jxl" {
            problem(
                ui,
                "JPEG XL holds a single image. Choose another format for multiple sides.",
            );
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
    ui.checkbox(
        &mut state.multipage,
        "Also combine the pages into one document",
    );
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
    let device_label = ui.label(label_text("Scanner"));
    let old = state.device.clone();
    egui::ComboBox::from_id_salt("workspace-device")
        .width(ui.available_width())
        .selected_text(RichText::new(&state.device).font(theme::mono(size::MONO)))
        .show_ui(ui, |ui| {
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
        note(ui, scanner_note(state));
        if quiet(ui, "Refresh", true).clicked() {
            state.refresh_devices();
        }
    });
}

fn scanner_note(state: &GuiState) -> &'static str {
    if state.discovery_loading {
        "Looking for scanners…"
    } else if state.devices.is_empty() {
        "No scanner found. Connect one and refresh, or use an image file."
    } else if state.device == "mock" {
        "Built-in test source. Generates sample pages, no hardware needed."
    } else if state.device.starts_with("file:") {
        "File source. Repeats the same image for every side."
    } else {
        "Settings the scanner doesn't support are adjusted by the scanner."
    }
}

fn source_capture(ui: &mut egui::Ui, state: &mut GuiState, batch: &mut bool) {
    ui.columns(2, |columns| {
        choice(
            &mut columns[0],
            "workspace-source",
            "Paper source",
            &mut state.media,
            &[
                ("flatbed", "Flatbed"),
                ("document", "Document feeder"),
                ("adf", "ADF"),
                ("film", "Film"),
            ],
        );
        segmented(
            &mut columns[1],
            "Capture",
            batch,
            [("Single image", false), ("Multiple sides", true)],
        );
    });
}

fn sides_resolution(ui: &mut egui::Ui, state: &mut GuiState, batch: bool) {
    let document = state.scan_mode() == crate::domain::acquisition::ScanMode::Document;
    ui.columns(2, |columns| {
        duplex_choice(&mut columns[0], state, batch, document);
        let ui = &mut columns[1];
        let label = ui.label(label_text("Resolution"));
        ui.add_sized(
            [ui.available_width(), size::CONTROL],
            egui::DragValue::new(&mut state.dpi)
                .range(50..=1200)
                .suffix(" dpi"),
        )
        .labelled_by(label.id)
        .on_hover_text("Drag or type. The scanner may use its nearest supported value.");
    });
    if state.duplex && (!document || !batch) {
        problem(ui, "Duplex needs the document feeder and multiple sides.");
    }
}

fn duplex_choice(ui: &mut egui::Ui, state: &mut GuiState, batch: bool, document: bool) {
    let label = ui.label(label_text("Sides"));
    ui.add_enabled_ui((document && batch) || state.duplex, |ui| {
        egui::ComboBox::from_id_salt("workspace-sides")
            .width(ui.available_width())
            .selected_text(if state.duplex {
                "Both sides"
            } else {
                "One side"
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.duplex, false, "One side");
                ui.add_enabled_ui(document && batch, |ui| {
                    ui.selectable_value(&mut state.duplex, true, "Both sides (duplex)");
                });
            })
            .response
            .labelled_by(label.id)
            .on_disabled_hover_text("Both sides needs the document feeder and multiple sides.");
    });
}

fn side_limit(ui: &mut egui::Ui, state: &mut GuiState, batch: bool) {
    if !batch {
        return;
    }
    ui.add_space(space::M);
    ui.horizontal(|ui| {
        let label = ui.label(label_text("Side limit"));
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
            format!("Duplex scans sides in pairs. Use an even limit, such as {suggestion}."),
        );
    }
}

fn side_limit_note(state: &GuiState) -> String {
    let sides = state.batch_pages;
    if !state.duplex || !sides.is_multiple_of(2) {
        return "Scanning stops here or when the feeder is empty.".into();
    }
    if state.device == "mock" {
        format!("{sides} simulated sides.")
    } else if state.device.starts_with("file:") {
        format!("{sides} repeated sides.")
    } else {
        format!("= {} double-sided sheets.", sides / 2)
    }
}

fn scan_area(ui: &mut egui::Ui, state: &mut GuiState) {
    ui.collapsing(
        RichText::new(format!("Scan area  {} × {} px", state.width, state.height)),
        |ui| {
            ui.horizontal_wrapped(|ui| {
                let width_label = ui.label(label_text("Width"));
                ui.add(
                    egui::DragValue::new(&mut state.width)
                        .range(16..=4096)
                        .suffix(" px"),
                )
                .labelled_by(width_label.id);
                ui.add_space(space::S);
                let height_label = ui.label(label_text("Height"));
                ui.add(
                    egui::DragValue::new(&mut state.height)
                        .range(16..=4096)
                        .suffix(" px"),
                )
                .labelled_by(height_label.id);
            });
            note(
                ui,
                "Requested size. The scanner may negotiate a supported area.",
            );
        },
    );
}

fn pdf_controls(ui: &mut egui::Ui, state: &mut GuiState, batch: bool) {
    ui.add_space(space::L);
    let pdf = plan::writes_pdf(state, batch);
    ui.add_enabled_ui(pdf || state.searchable_pdf, |ui| {
        ui.checkbox(
            &mut state.searchable_pdf,
            "Searchable PDF (adds a text layer)",
        );
    });
    if !pdf {
        if state.searchable_pdf || !state.pdf_password.is_empty() {
            problem(
                ui,
                "Choose PDF output, or turn off searchable text and clear the PDF password.",
            );
        } else {
            note(ui, "Searchable text and passwords need PDF output.");
        }
    }
    if state.searchable_pdf {
        ui.add_space(space::S);
        ui.add_enabled_ui(pdf, |ui| ocr_controls(ui, state));
    }
    ui.add_space(space::M);
    ui.add_enabled_ui(pdf || !state.pdf_password.is_empty(), |ui| {
        ui.collapsing("PDF password", |ui| {
            let label = ui.label(label_text("Password"));
            ui.add(
                egui::TextEdit::singleline(&mut state.pdf_password)
                    .password(true)
                    .desired_width(ui.available_width())
                    .margin(egui::Margin::symmetric(10, 10)),
            )
            .labelled_by(label.id);
            note(
                ui,
                "Used for this session only. It is never written to settings.",
            );
        });
    });
    more_output(ui, state, batch);
}

fn more_output(ui: &mut egui::Ui, state: &mut GuiState, batch: bool) {
    ui.collapsing("More output options", |ui| {
        ui.add_enabled_ui(!batch, |ui| {
            ui.checkbox(
                &mut state.save_raw,
                "Keep an unprocessed TIFF copy (single image)",
            );
        });
        ui.add_enabled_ui(batch, |ui| {
            ui.checkbox(
                &mut state.contact_sheet,
                "Save a contact sheet (multiple sides)",
            );
        });
        ui.add_space(space::S);
        field(ui, "Scanner color profile", &mut state.scanner_profile_path);
    });
}

fn ocr_controls(ui: &mut egui::Ui, state: &mut GuiState) {
    if ui.available_width() >= 400.0 {
        ui.columns(2, |columns| {
            ocr_engine(&mut columns[0], &mut state.ocr_engine);
            field(&mut columns[1], "Text language", &mut state.ocr_language);
        });
    } else {
        ocr_engine(ui, &mut state.ocr_engine);
        field(ui, "Text language", &mut state.ocr_language);
    }
    note(
        ui,
        match state.ocr_engine.as_str() {
            "tesseract" => {
                "Needs Tesseract and its language data installed. Checked when the PDF is written."
            }
            "ocrs" => "Needs local OCRS model files. Preview quality, printed Latin English only.",
            _ => "Basic 5×7 template matching for simple printed text, not general OCR.",
        },
    );
}

fn ocr_engine(ui: &mut egui::Ui, engine: &mut String) {
    choice(
        ui,
        "workspace-ocr",
        "Text recognition",
        engine,
        &[
            ("offline", "Built-in templates"),
            ("tesseract", "Tesseract"),
            ("ocrs", "OCRS (local models)"),
        ],
    );
}
