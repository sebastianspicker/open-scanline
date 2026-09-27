use super::*;

pub(super) fn scan(ui: &mut egui::Ui, state: &mut GuiState, batch: &mut bool) {
    ui.heading("Scan settings");
    ui.add_space(10.0);
    scanner(ui, state);
    ui.add_space(12.0);
    acquisition_controls(ui, state, batch);
    ui.add_space(10.0);
    ui.separator();
    ui.collapsing(
        format!("Acquisition area · {} × {} px", state.width, state.height),
        |ui| {
            ui.horizontal_wrapped(|ui| {
                let width_label = ui.label("Width");
                ui.add(
                    egui::DragValue::new(&mut state.width)
                        .range(16..=4096)
                        .suffix(" px"),
                )
                .labelled_by(width_label.id);
                let height_label = ui.label("Height");
                ui.add(
                    egui::DragValue::new(&mut state.height)
                        .range(16..=4096)
                        .suffix(" px"),
                )
                .labelled_by(height_label.id);
            });
            note(
                ui,
                "Requested image dimensions; the scanner may negotiate supported settings.",
            );
        },
    );
    if ui.button("Image corrections").clicked() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new(TOOLS), true));
        state.active_tab = 1;
    }
}

pub(super) fn output(ui: &mut egui::Ui, state: &mut GuiState, batch: bool) {
    ui.heading("Output");
    ui.add_space(10.0);
    field(ui, "Filename base", &mut state.output_name);
    if let Err(error) = crate::domain::settings::validate_output_name(&state.output_name) {
        ui.colored_label(Color32::from_rgb(160, 62, 42), error.to_string());
    }
    ui.add_space(10.0);
    choice(
        ui,
        "workspace-format",
        &mut state.output_fmt,
        &[
            ("png", "PNG image"),
            ("jpg", "JPEG image"),
            ("tif", "TIFF image"),
            ("webp", "WebP image"),
            ("bmp", "BMP image"),
            ("gif", "GIF image"),
            ("pdf", "PDF document"),
            ("jxl", "JPEG XL image"),
        ],
    );
    if batch {
        if state.output_fmt != "pdf" {
            ui.checkbox(&mut state.multipage, "Create a multipage document");
        }
        if state.multipage && state.output_fmt != "pdf" {
            choice(
                ui,
                "workspace-container",
                &mut state.multipage_format,
                &[("pdf", "PDF"), ("tif", "TIFF")],
            );
        }
        if let Ok(args) = state.batch_args(
            std::path::PathBuf::from(&state.output_dir).join("batch"),
            state.batch_pages,
        ) {
            if let Some(path) = args.multipage_out {
                display_path(ui, "Document", &path, &state.output_dir);
            }
            note(ui, format!("Page images: batch/page_001.{} …", args.format));
        }
    } else if let Ok(path) = state.out_path("scan") {
        display_path(ui, "File", &path, &state.output_dir);
    }
    pdf_controls(ui, state, batch);
}

fn display_path(ui: &mut egui::Ui, label: &str, path: &std::path::Path, output_dir: &str) {
    let relative = path.strip_prefix(output_dir).unwrap_or(path);
    ui.label(
        RichText::new(format!("{label}: {}", relative.display()))
            .small()
            .color(MUTED),
    )
    .on_hover_text(path.display().to_string());
}

fn scanner(ui: &mut egui::Ui, state: &mut GuiState) {
    let device_label = ui.label(state.translator.t("input.device"));
    let old = state.device.clone();
    egui::ComboBox::from_id_salt("workspace-device")
        .width(ui.available_width())
        .selected_text(&state.device)
        .show_ui(ui, |ui| {
            for device in &state.devices {
                ui.selectable_value(&mut state.device, device.clone(), device);
            }
        })
        .response
        .labelled_by(device_label.id);
    if old != state.device {
        state.refresh_maintenance_capabilities();
    }
    ui.horizontal_wrapped(|ui| {
        if state.discovery_loading {
            ui.spinner();
            note(ui, "Looking for scanners…");
        } else if state.devices.is_empty() {
            note(ui, "No scanner found.");
        } else if state.device == "mock" {
            note(ui, "Mock source · generated test image");
        }
        if ui.small_button("Refresh").clicked() {
            state.refresh_devices();
        }
    });
}

fn acquisition_controls(ui: &mut egui::Ui, state: &mut GuiState, batch: &mut bool) {
    source_capture(ui, state, batch);
    sides_resolution(ui, state, batch);
    side_limit(ui, state, *batch);
}

fn pdf_controls(ui: &mut egui::Ui, state: &mut GuiState, batch: bool) {
    ui.add_space(20.0);
    let pdf =
        state.output_fmt == "pdf" || (batch && state.multipage && state.multipage_format == "pdf");
    ui.add_enabled_ui(pdf || state.searchable_pdf, |ui| {
        ui.checkbox(&mut state.searchable_pdf, "Make PDF searchable");
    });
    if !pdf {
        note(ui, "Searchable text and passwords require PDF output.");
        if state.searchable_pdf || !state.pdf_password.is_empty() {
            ui.colored_label(
                Color32::from_rgb(160, 62, 42),
                "Choose PDF output, or turn off searchable text and clear the PDF password.",
            );
        }
    }
    if batch && state.output_fmt == "jxl" {
        ui.colored_label(Color32::from_rgb(160, 62, 42), "JPEG XL is available for single images. Choose a different format for multiple image sides.");
    }
    if state.searchable_pdf {
        ui.add_enabled_ui(pdf, |ui| ocr_controls(ui, state));
    }
    ui.add_space(16.0);
    ui.separator();
    ui.add_enabled_ui(pdf || !state.pdf_password.is_empty(), |ui| {
        ui.collapsing("PDF password", |ui| {
            let label = ui.label("Password (runtime only)");
            ui.add(
                egui::TextEdit::singleline(&mut state.pdf_password)
                    .password(true)
                    .desired_width(ui.available_width()),
            )
            .labelled_by(label.id);
        });
    });
    ui.collapsing("More output options", |ui| {
        ui.add_enabled_ui(!batch, |ui| {
            ui.checkbox(&mut state.save_raw, "Save raw TIFF (single scan)");
        });
        ui.add_enabled_ui(batch, |ui| {
            ui.checkbox(&mut state.contact_sheet, "Save contact sheet (batch)");
        });
        field(ui, "Scanner profile path", &mut state.scanner_profile_path);
    });
}

fn source_capture(ui: &mut egui::Ui, state: &mut GuiState, batch: &mut bool) {
    ui.columns(2, |columns| {
        choice(
            &mut columns[0],
            "workspace-source",
            &mut state.media,
            &[
                ("flatbed", "Flatbed"),
                ("document", "Document feeder"),
                ("adf", "ADF"),
                ("film", "Film"),
            ],
        );
        let ui = &mut columns[1];
        let label = ui.label("Capture");
        egui::ComboBox::from_id_salt("workspace-capture")
            .width(ui.available_width())
            .selected_text(if *batch {
                "Multiple sides"
            } else {
                "Single image"
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(batch, false, "Single image");
                ui.selectable_value(batch, true, "Multiple sides");
            })
            .response
            .labelled_by(label.id);
    });
}

fn sides_resolution(ui: &mut egui::Ui, state: &mut GuiState, batch: &mut bool) {
    ui.add_space(8.0);
    let document = state.scan_mode() == crate::domain::acquisition::ScanMode::Document;
    ui.columns(2, |columns| {
        let ui = &mut columns[0];
        duplex_choice(ui, state, *batch, document);
        let ui = &mut columns[1];
        let label = ui.label(state.translator.t("input.dpi"));
        ui.add_sized(
            [ui.available_width(), 38.0],
            egui::DragValue::new(&mut state.dpi)
                .range(50..=1200)
                .suffix(" dpi"),
        )
        .labelled_by(label.id);
    });
    if state.duplex && (!document || !*batch) {
        note(
            ui,
            "Duplex requires a document feeder and multiple image sides.",
        );
    }
}

fn side_limit(ui: &mut egui::Ui, state: &mut GuiState, batch: bool) {
    if batch {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let label = ui.label("Side limit");
            ui.add(egui::DragValue::new(&mut state.batch_pages).range(1..=1000))
                .labelled_by(label.id);
            note(ui, "image sides");
        });
    }
    if batch && state.duplex {
        if state.batch_pages.is_multiple_of(2) {
            note(
                ui,
                if state.device == "mock" {
                    format!("{} simulated image sides maximum.", state.batch_pages)
                } else if state.device.starts_with("file:") {
                    format!("{} repeated image sides maximum.", state.batch_pages)
                } else {
                    format!("{} double-sided sheets maximum.", state.batch_pages / 2)
                },
            );
        } else {
            ui.colored_label(
                Color32::from_rgb(160, 62, 42),
                "Both sides requires an even number of image sides.",
            );
        }
    }
}

fn duplex_choice(ui: &mut egui::Ui, state: &mut GuiState, batch: bool, document: bool) {
    let label = ui.label("Sides");
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
                    ui.selectable_value(&mut state.duplex, true, "Both sides, duplex");
                });
            })
            .response
            .labelled_by(label.id)
            .on_hover_text("Both sides uses duplex acquisition.");
    });
}

fn ocr_controls(ui: &mut egui::Ui, state: &mut GuiState) {
    if ui.available_width() >= 400.0 {
        ui.columns(2, |columns| {
            ocr_engine(&mut columns[0], &mut state.ocr_engine);
            field(
                &mut columns[1],
                &state.translator.t("ocr.language"),
                &mut state.ocr_language,
            );
        });
    } else {
        ocr_engine(ui, &mut state.ocr_engine);
        field(
            ui,
            &state.translator.t("ocr.language"),
            &mut state.ocr_language,
        );
    }
    note(ui, match state.ocr_engine.as_str() {
        "tesseract" => "Requires installed Tesseract and language data. Availability is checked when processing.",
        "ocrs" => "Requires local OCRS models. Preview support for printed Latin English.",
        _ => "Basic 5×7 template recognition; not general document OCR.",
    });
}

fn ocr_engine(ui: &mut egui::Ui, engine: &mut String) {
    choice(
        ui,
        "workspace-ocr",
        engine,
        &[
            ("offline", "Built-in templates"),
            ("tesseract", "Tesseract"),
            ("ocrs", "OCRS (local models)"),
        ],
    );
}
