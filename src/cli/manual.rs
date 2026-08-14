use crate::{APP_NAME, VERSION};

pub(super) fn user_manual_text() -> String {
    format!(
        r#"{APP_NAME} {VERSION} — local Rust application for acquiring, processing, and exporting scanned images

Commands:
  devices              List sources and backend availability
  manufacturers        Browse the manufacturer support catalog
  scan                 Acquire one image from a selected source
  batch                Multi-page acquisition with PDF or TIFF output
  package              Validate and archive an existing executable as ZIP
  convert              Convert an image to PNG, JPEG, TIFF, WebP, BMP, GIF, or PDF; JPEG XL needs cjxl
  process              Crop, transform, clean, and adjust an image
  onnx                 Run a user-supplied ONNX image model and print JSON
  ocr                  Built-in offline OCR or optional Tesseract OCR
  info                 Capability and runtime availability JSON
  config               Show or write the JSON configuration
  gui                  Desktop GUI (default gui feature and desktop session)
  plugin / --mode=plugin   Headless host status and acquisition entry point
  help-text            This manual

Shared paths:
  open_scanline::scan::run_scan_to_file
  open_scanline::process::process_image_file
  open_scanline::batch::run_batch_scan

PDF export controls for scan, process, and batch:
  --pdf-searchable            Add OCR text layers to a PDF destination
  --pdf-password-file PATH    Read a PDF password from PATH, or - for standard input
  --pdf-password PASSWORD     UNSAFE/DEPRECATED; requires --allow-insecure-password-argv
  --allow-insecure-password-argv
                               Explicitly opt in to a command-line password
  --ocr-lang LANG             OCR language (default: eng)
  --ocr-engine offline|tesseract
                               OCR implementation for searchable PDF output
  --scanner-profile PATH      Apply a scanner profile before export
  For batch, these PDF-only controls require a PDF destination selected by --multipage-pdf,
  --multipage-out PATH.pdf, or the configured output/multipage format.

Direct eSCL trust:
  --allow-unlisted-escl        Permit a strict direct eSCL device id that was not discovered
                               or listed in OPEN_SCANLINE_ESCL_HOSTS (scan and batch only)

Not included: proprietary scanner drivers, firmware, activation or licensing systems,
a native TWAIN Data Source or direct TWAIN acquisition, or proprietary resource blobs.
"#
    )
}

#[cfg(test)]
mod tests {
    use super::user_manual_text;

    #[test]
    fn manual_describes_current_product_scope() {
        let manual = user_manual_text();

        for stale_claim in ["prototype", "optional later", "Desktop UI shell"] {
            assert!(
                !manual.contains(stale_claim),
                "manual still contains stale claim: {stale_claim}"
            );
        }
        assert!(manual.contains("local Rust application"));
        assert!(manual.contains("Built-in offline OCR or optional Tesseract OCR"));
        assert!(manual.contains("to PNG, JPEG, TIFF, WebP, BMP, GIF, or PDF"));
        assert!(manual.contains("Desktop GUI (default gui feature and desktop session)"));
        assert!(manual.contains("--pdf-searchable"));
        assert!(manual.contains("--pdf-password-file PATH"));
        assert!(manual.contains("UNSAFE/DEPRECATED"));
        assert!(manual.contains("--allow-unlisted-escl"));
    }
}
