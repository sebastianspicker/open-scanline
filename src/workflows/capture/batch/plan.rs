//! Shared batch output naming derivation used by CLI and GUI callers.
//!
//! Both entry points independently derived the same rule set before this
//! module existed: a PDF page format means "write PNG pages, then assemble a
//! PDF", and default multipage/contact-sheet destinations follow
//! `{name}_multipage.{fmt}` / `{name}_contact.bmp`. Each caller still owns
//! its own explicit-destination overrides and input validation.
use std::path::{Path, PathBuf};

/// Inputs shared by CLI and GUI batch output planning.
pub(crate) struct BatchOutputRequest<'a> {
    pub(crate) out_dir: &'a Path,
    pub(crate) output_name: &'a str,
    /// The requested page format, or PDF when the whole batch is a PDF.
    pub(crate) configured_format: &'a str,
    /// Requests a default multipage destination when one isn't already named.
    pub(crate) want_multipage: bool,
    pub(crate) multipage_format: &'a str,
    /// Requests a default contact-sheet destination.
    pub(crate) want_contact_sheet: bool,
}

/// Derived page format plus default multipage/contact-sheet destinations.
pub(crate) struct BatchOutputPlan {
    pub(crate) page_format: String,
    pub(crate) multipage_out: Option<PathBuf>,
    pub(crate) contact_sheet: Option<PathBuf>,
}

pub(crate) fn plan_batch_outputs(request: BatchOutputRequest<'_>) -> BatchOutputPlan {
    let pdf_output = request.configured_format.eq_ignore_ascii_case("pdf");
    let page_format = if pdf_output {
        "png".to_string()
    } else {
        request.configured_format.to_owned()
    };
    let multipage_out = (request.want_multipage || pdf_output).then(|| {
        let format = if pdf_output {
            "pdf"
        } else {
            request.multipage_format
        };
        request
            .out_dir
            .join(format!("{}_multipage.{}", request.output_name, format))
    });
    let contact_sheet = request.want_contact_sheet.then(|| {
        request
            .out_dir
            .join(format!("{}_contact.bmp", request.output_name))
    });
    BatchOutputPlan {
        page_format,
        multipage_out,
        contact_sheet,
    }
}
