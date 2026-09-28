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

fn plural(count: u32, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// One sentence that says what the job produces.
pub(super) fn sentence(state: &GuiState, batch: bool) -> String {
    if !batch {
        return format!(
            "One image from the {}, saved as {}.",
            source_phrase(&state.media),
            state.output_fmt.to_uppercase()
        );
    }
    let pages = match duplex_sheets(state, batch) {
        Some(sheets) => format!(
            "Up to {}",
            plural(sheets, "double-sided sheet", "double-sided sheets")
        ),
        None => format!("Up to {}", plural(state.batch_pages, "side", "sides")),
    };
    format!("{pages}, saved as {}.", document_phrase(state, batch))
}

fn document_phrase(state: &GuiState, batch: bool) -> String {
    let pages = page_format(state).map_or_else(
        || "page images".to_owned(),
        |format| format!("{} page images", format.to_uppercase()),
    );
    if writes_pdf(state, batch) {
        let kind = match (state.searchable_pdf, !state.pdf_password.is_empty()) {
            (true, true) => "a searchable, password-protected PDF",
            (true, false) => "a searchable PDF",
            (false, true) => "a password-protected PDF",
            (false, false) => "one PDF",
        };
        format!("{kind} plus {pages}")
    } else if state.multipage {
        format!("one multipage TIFF plus {pages}")
    } else {
        format!("separate {pages}")
    }
}

fn source_phrase(media: &str) -> &'static str {
    match media {
        "film" => "film holder",
        "document" | "adf" => "document feeder",
        _ => "flatbed",
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
        return format!("Enter a file name of 1 to {limit} bytes.");
    }
    let rules = [
        ("normal file name", "Use a real name, not . or ..."),
        (
            "dot or space",
            "File names can't end with a dot or a space.",
        ),
        (
            "path separator",
            "File names can't contain / \\ : * ? \" < > | or control characters.",
        ),
        (
            "device name",
            "That name is reserved on Windows (CON, PRN, NUL, COM1…). Choose another.",
        ),
    ];
    rules
        .iter()
        .find(|(needle, _)| error.contains(needle))
        .map_or_else(|| error.to_owned(), |(_, text)| (*text).to_owned())
}

pub(super) fn limit_label(state: &GuiState, batch: bool) -> String {
    if batch {
        format!("Up to {}", plural(state.batch_pages, "side", "sides"))
    } else {
        "One image".into()
    }
}

/// Ledger of the files this job will write, relative to the chosen folder.
pub(super) fn ledger(ui: &mut egui::Ui, state: &GuiState, batch: bool) {
    ui.add_space(space::M);
    ui.label(label_text("Files this scan writes"));
    ui.add_space(space::XS);
    let root = Path::new(&state.output_dir);
    if !batch {
        match single_outputs(state) {
            Ok(outputs) => {
                for (path, role) in outputs {
                    row(ui, &relative(&path, root), role);
                }
            }
            Err(_) => note(ui, "Enter a valid file name to see the plan."),
        }
        return;
    }
    let Ok(args) = state.batch_args(root.join("batch"), state.batch_pages) else {
        note(ui, "Enter a valid file name to see the plan.");
        return;
    };
    if let Some(path) = &args.multipage_out {
        row(
            ui,
            &relative(path, root),
            "Document, written after the last side",
        );
    }
    let last = format!("page_{:03}.{}", state.batch_pages, args.format);
    let pages = if state.batch_pages == 1 {
        format!("batch/{last}")
    } else {
        format!("batch/page_001.{} … {last}", args.format)
    };
    row(
        ui,
        &pages,
        &plural(state.batch_pages, "page image", "page images"),
    );
    if let Some(path) = &args.contact_sheet {
        row(ui, &relative(path, root), "Contact sheet");
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
        "Document"
    } else {
        "Image"
    };
    let mut outputs = vec![(args.out, role)];
    if let Some(path) = args.raw_out {
        outputs.push((path, "Unprocessed TIFF copy"));
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
