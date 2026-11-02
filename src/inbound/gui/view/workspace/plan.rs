//! The output plan: what a job will write, stated before it starts.

use super::*;
use std::path::{Path, PathBuf};

pub(super) fn writes_pdf(state: &GuiState, batch: bool) -> bool {
    state.output_fmt == "pdf" || (batch && state.multipage && state.multipage_format == "pdf")
}

fn duplex_sheets(state: &GuiState, batch: bool) -> Option<u32> {
    let real_paper = state.device != "mock" && !state.device.starts_with("file:");
    (batch && state.duplex && real_paper && state.batch_pages.is_multiple_of(2))
        .then_some(state.batch_pages / 2)
}

/// `one` and `many` are catalog keys whose text holds a `{count}` placeholder.
fn plural(count: u32, one: &str, many: &str) -> String {
    tr_args(
        if count == 1 { one } else { many },
        &[("count", &count.to_string())],
    )
}

fn up_to(amount: String) -> String {
    tr_args("ws.up_to", &[("amount", &amount)])
}

/// One sentence that says what the job produces.
pub(super) fn sentence(state: &GuiState, batch: bool) -> String {
    if !batch {
        return tr_args(
            "ws.plan_single",
            &[
                ("source", &tr(source_phrase(&state.media))),
                ("format", &state.output_fmt.to_uppercase()),
            ],
        );
    }
    let pages = match duplex_sheets(state, batch) {
        Some(sheets) => up_to(plural(sheets, "ws.sheet_one", "ws.sheet_many")),
        None => up_to(plural(state.batch_pages, "ws.side_one", "ws.side_many")),
    };
    tr_args(
        "ws.plan_batch",
        &[
            ("pages", &pages),
            ("output", &document_phrase(state, batch)),
        ],
    )
}

fn document_phrase(state: &GuiState, batch: bool) -> String {
    let pages = page_format(state).map_or_else(
        || tr("ws.page_images"),
        |format| {
            tr_args(
                "ws.page_images_format",
                &[("format", &format.to_uppercase())],
            )
        },
    );
    if writes_pdf(state, batch) {
        let kind = match (state.searchable_pdf, !state.pdf_password.is_empty()) {
            (true, true) => "ws.pdf_searchable_protected",
            (true, false) => "ws.pdf_searchable",
            (false, true) => "ws.pdf_protected",
            (false, false) => "ws.pdf_one",
        };
        tr_args(
            "ws.kind_plus_pages",
            &[("kind", &tr(kind)), ("pages", &pages)],
        )
    } else if state.multipage {
        tr_args("ws.tiff_plus_pages", &[("pages", &pages)])
    } else {
        tr_args("ws.separate_pages", &[("pages", &pages)])
    }
}

fn source_phrase(media: &str) -> &'static str {
    match media {
        "film" => "ws.source_film",
        "document" | "adf" => "ws.source_feeder",
        _ => "ws.source_flatbed",
    }
}

fn page_format(state: &GuiState) -> Option<String> {
    state
        .batch_args(
            Path::new(&state.output_dir).join("batch"),
            state.batch_pages,
        )
        .ok()
        .map(|args| args.format)
}

/// Plain-language version of the output-name validation error.
pub(super) fn name_problem(error: &str) -> String {
    if error.contains("UTF-8 bytes") {
        let limit = crate::domain::settings::MAX_OUTPUT_NAME_BYTES;
        return tr_args("ws.name_length", &[("limit", &limit.to_string())]);
    }
    let rules = [
        ("normal file name", "ws.name_real"),
        ("dot or space", "ws.name_trailing"),
        ("path separator", "ws.name_chars"),
        ("device name", "ws.name_reserved"),
    ];
    rules
        .iter()
        .find(|(needle, _)| error.contains(needle))
        .map_or_else(|| error.to_owned(), |(_, key)| tr(key))
}

pub(super) fn limit_label(state: &GuiState, batch: bool) -> String {
    if batch {
        up_to(plural(state.batch_pages, "ws.side_one", "ws.side_many"))
    } else {
        tr("ws.one_image")
    }
}

/// Ledger of the files this job will write, relative to the chosen folder.
pub(super) fn ledger(ui: &mut egui::Ui, state: &GuiState, batch: bool) {
    ui.add_space(space::M);
    ui.label(label_text(&tr("ws.files_this_scan_writes")));
    ui.add_space(space::XS);
    let root = Path::new(&state.output_dir);
    if !batch {
        match single_outputs(state) {
            Ok(outputs) => {
                for (path, role) in outputs {
                    row(ui, &relative(&path, root), &tr(role));
                }
            }
            Err(_) => note(ui, tr("ws.plan_needs_name")),
        }
        return;
    }
    let Ok(args) = state.batch_args(root.join("batch"), state.batch_pages) else {
        note(ui, tr("ws.plan_needs_name"));
        return;
    };
    if let Some(path) = &args.multipage_out {
        row(ui, &relative(path, root), &tr("ws.document_after_last"));
    }
    let page_name = |page| {
        crate::workflows::capture::batch::page_file_name(page, state.batch_pages, &args.format)
    };
    let last = page_name(state.batch_pages);
    let pages = if state.batch_pages == 1 {
        format!("batch/{last}")
    } else {
        format!("batch/{} … {last}", page_name(1))
    };
    row(
        ui,
        &pages,
        &plural(state.batch_pages, "ws.page_image_one", "ws.page_image_many"),
    );
    if let Some(path) = &args.contact_sheet {
        row(ui, &relative(path, root), &tr("ws.contact_sheet"));
    }
}

/// Use the canonical scan arguments so the plan cannot drift from the files
/// the single-image workflow will actually publish.
pub(super) fn single_outputs(
    state: &GuiState,
) -> crate::error::Result<Vec<(PathBuf, &'static str)>> {
    let args = state.scan_args(false)?;
    let role = if args
        .out
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
    {
        "ws.role_document"
    } else {
        "ws.role_image"
    };
    let mut outputs = vec![(args.out, role)];
    if let Some(path) = args.raw_out {
        outputs.push((path, "ws.role_raw_tiff"));
    }
    Ok(outputs)
}

fn relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// A ledger line: monospaced name, then its role in graphite.
pub(super) fn row(ui: &mut egui::Ui, name: &str, role: &str) {
    let width = ui.available_width();
    let top = ui.cursor().top();
    ui.add(egui::Label::new(RichText::new(name).font(theme::mono(size::MONO))).wrap());
    note(ui, role);
    let bottom = ui.cursor().top() - space::XS;
    ui.painter().hline(
        ui.min_rect().left()..=ui.min_rect().left() + width,
        bottom.max(top),
        egui::Stroke::new(theme::HAIRLINE, color::RULE),
    );
    ui.add_space(space::XS);
}
