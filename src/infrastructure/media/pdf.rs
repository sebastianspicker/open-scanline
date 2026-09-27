//! PDF publication facade with private page, searchable-text, document, and path seams.

mod document;
mod page;
mod paths;
mod searchable;

pub use document::{save_pdf_with_options, save_pdf_with_options_and_cancellation, PdfOptions};
pub(crate) use paths::save_pdf_from_paths_with_loader_and_transform;
pub use paths::{
    save_multipage_pdf, save_multipage_pdf_with_cancellation, save_pdf_from_paths_with_options,
    save_pdf_from_paths_with_options_and_cancellation,
    save_pdf_from_paths_with_options_and_transform,
    save_pdf_from_paths_with_options_and_transform_and_cancellation,
};
pub(crate) use searchable::{checked_pdf_searchable_text_total, validate_pdf_password};

use crate::error::Result;
use crate::operation::CancellationToken;

fn check_pdf_cancellation(cancellation: Option<&CancellationToken>) -> Result<()> {
    crate::operation::check_cancellation(cancellation, "PDF publication cancelled")
}
