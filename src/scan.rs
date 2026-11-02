//! Compatibility facade for the single-capture workflow.

pub use crate::workflows::capture::single::{
    run_scan_to_file as run_scan_to_file_with_options, CaptureOptions, ScanToFileArgs,
};
pub use crate::workflows::compat::run_scan_to_file;
