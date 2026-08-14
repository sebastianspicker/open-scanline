//! GUI action handlers, grouped by their external workflow.

mod batch;
mod config;
mod device;
mod files;
mod helpers;
mod scan;

#[cfg(feature = "gui")]
pub(in crate::gui) use files::SaveAction;
