mod templates;

use super::probing::powershell_spec;
use crate::domain::acquisition::{ScanMode, ScanRequest};
use crate::domain::image::PixelFormat;
use crate::infrastructure::runtime::{CommandOutput, CommandSpec};
use std::path::Path;

/// Build the WIA COM transfer invocation as data for inspection/injection.
/// Values are quoted for the PowerShell script; dimensions and DPI remain
/// numeric data. Callers validate request bounds before invoking the adapter.
pub fn wia_transfer_command(
    raw_id: &str,
    request: &ScanRequest,
    output_path: &Path,
) -> CommandSpec {
    powershell_spec(render_script(
        templates::SINGLE_TRANSFER,
        &TransferData::new(raw_id, request, output_path),
        1,
    ))
}

/// Build one WIA feeder transfer process which keeps one COM device connection
/// alive while it materializes at most `max_pages` scanned sides.  The output
/// names are deliberately zero-padded so the Rust side can decode them in a
/// stable, numeric order independent of filesystem enumeration order.
pub(super) fn wia_transfer_pages_command(
    raw_id: &str,
    request: &ScanRequest,
    output_directory: &Path,
    max_pages: u32,
) -> CommandSpec {
    powershell_spec(render_script(
        templates::PAGE_TRANSFER,
        &TransferData::new(raw_id, request, output_directory),
        max_pages,
    ))
}

struct TransferData {
    output: String,
    raw_id: String,
    source: &'static str,
    duplex: &'static str,
    intent: u8,
    dpi_x: u32,
    dpi_y: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl TransferData {
    fn new(raw_id: &str, request: &ScanRequest, output_path: &Path) -> Self {
        let (x, y, width, height) = geometry(request);
        Self {
            output: powershell_literal(output_path.display()),
            raw_id: powershell_literal(raw_id),
            source: scan_source(request.mode),
            duplex: powershell_bool(request.duplex),
            intent: wia_intent(request.pixel_format),
            dpi_x: request.dpi_x,
            dpi_y: request.dpi_y,
            x,
            y,
            width,
            height,
        }
    }
}

fn geometry(request: &ScanRequest) -> (i32, i32, u32, u32) {
    request
        .region
        .map(|region| {
            (
                region.x.max(0),
                region.y.max(0),
                region.width.max(1),
                region.height.max(1),
            )
        })
        .unwrap_or((0, 0, request.width, request.height))
}

/// Escape for a single-quoted PowerShell string. PowerShell treats the
/// typographic single quotes U+2018, U+2019, U+201A and U+201B like `'`, so
/// each is doubled too.
fn powershell_literal(value: impl std::fmt::Display) -> String {
    let mut escaped = String::new();
    for character in value.to_string().chars() {
        escaped.push(character);
        if matches!(
            character,
            '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}'
        ) {
            escaped.push(character);
        }
    }
    escaped
}

fn scan_source(mode: ScanMode) -> &'static str {
    match mode {
        ScanMode::Reflective => "reflective",
        ScanMode::Document => "document",
        ScanMode::Film => "film",
    }
}

fn powershell_bool(value: bool) -> &'static str {
    if value {
        "$true"
    } else {
        "$false"
    }
}

fn wia_intent(pixel_format: PixelFormat) -> u8 {
    match pixel_format {
        PixelFormat::Gray8 => 2,
        PixelFormat::Rgb8 | PixelFormat::Rgba8 => 1,
    }
}

fn render_script(template: &str, data: &TransferData, max_pages: u32) -> String {
    template
        .replace("__OSL_OUTPUT__", &data.output)
        .replace("__OSL_RAW_ID__", &data.raw_id)
        .replace("__OSL_SOURCE__", data.source)
        .replace("__OSL_DUPLEX__", data.duplex)
        .replace("__OSL_MAX_PAGES__", &max_pages.to_string())
        .replace("__OSL_INTENT__", &data.intent.to_string())
        .replace("__OSL_DPI_X__", &data.dpi_x.to_string())
        .replace("__OSL_DPI_Y__", &data.dpi_y.to_string())
        .replace("__OSL_X__", &data.x.to_string())
        .replace("__OSL_Y__", &data.y.to_string())
        .replace("__OSL_WIDTH__", &data.width.to_string())
        .replace("__OSL_HEIGHT__", &data.height.to_string())
}
pub(super) fn feeder_exhausted(output: &CommandOutput) -> bool {
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    text.to_ascii_lowercase().contains("osl_feeder_exhausted")
        || crate::infrastructure::acquisition::command_backend::mentions_feeder_exhausted(&text)
}

#[cfg(test)]
mod tests {
    use super::powershell_literal;

    #[test]
    fn literal_doubles_every_powershell_single_quote() {
        assert_eq!(powershell_literal("it's"), "it''s");
        assert_eq!(
            powershell_literal("C:\\Users\\O\u{2019}Brien\\x.png"),
            "C:\\Users\\O\u{2019}\u{2019}Brien\\x.png"
        );
        assert_eq!(
            powershell_literal("\u{2018}\u{201A}\u{201B}"),
            "\u{2018}\u{2018}\u{201A}\u{201A}\u{201B}\u{201B}"
        );
    }
}
